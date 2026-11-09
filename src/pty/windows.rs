//! Windows 的 pty：ConPTY（`CreatePseudoConsole`）起伪控制台，子进程挂在上面，两根管道一根读一根写。
//!
//! 线程和 [`super::posix`] 一样读写分开，多一个等子进程的：
//! - **读**：堵在输出管道上，读到多少交给 [`ByteQueue`]
//! - **写**：按键、粘贴排到写线程，写进输入管道
//! - **等**：子进程退出后关掉伪控制台。伪控制台开着时输出管道的写端在它手里，读线程读不到结尾；
//!   关掉它，最后一帧画面写出来、管道关上，读线程才读到结尾，再报退出码
//!
//! 关伪控制台（`ClosePseudoConsole`）要等输出管道被读空：所以读线程在取的那头没了以后也一直读到结尾（读到的丢掉），
//! 句柄丢了时的关也放在后台线程上，界面不等。
//!
//! 在 Windows 11 上用系统自带的 ConPTY 实测。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessTimes, INFINITE,
    InitializeProcThreadAttributeList, OpenProcess, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::queue::{ByteQueue, Chunk, Producer};
use super::{Config, Pty, PtyBackend};

/// 伪控制台。关掉它会结束挂在上面的程序（像关掉终端窗口），只关一次：子进程自己退出、或者句柄都丢了，谁先到谁关
struct Console {
    handle: Mutex<Option<HPCON>>,
}

impl Console {
    /// 锁坏了（别的线程持锁时 panic）也照样用：里面只有一个句柄
    fn lock(&self) -> MutexGuard<'_, Option<HPCON>> {
        self.handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn resize(&self, columns: u16, rows: u16) {
        let handle = self.lock();
        if let Some(handle) = *handle {
            // SAFETY: 锁着，`close` 拿不走它，句柄还开着。改不动就算了（子进程可能正好退出），这里没有能报告的地方
            let _failed = unsafe { ResizePseudoConsole(handle, size(columns, rows)) };
        }
    }

    /// 关掉。要等输出管道读空（读线程一直在读），所以别在主线程上调
    fn close(&self) {
        // 先拿出来再关：关的时候不持锁，`resize` 不会跟着等
        let handle = self.lock().take();
        if let Some(handle) = handle {
            // SAFETY: 句柄从 `Option` 里拿出来了，以后谁也拿不到它，只在这里关这一次
            unsafe { ClosePseudoConsole(handle) };
        }
    }
}

/// 伪控制台的大小。行列数放不进 `i16` 的（不会有这么大的窗口）取最大
fn size(columns: u16, rows: u16) -> COORD {
    COORD {
        X: i16::try_from(columns.max(1)).unwrap_or(i16::MAX),
        Y: i16::try_from(rows.max(1)).unwrap_or(i16::MAX),
    }
}

struct Session {
    console: Arc<Console>,
    /// 交给写线程的口子。锁一下主要是为了满足 `Sync`，一次按键就进一次
    writes: Mutex<mpsc::Sender<Vec<u8>>>,
    process_id: u32,
    /// 子进程的创建时间，认它的子进程时用（见 [`has_children`]）
    started: u64,
    /// 子进程已经退出（等线程马上会放掉它的句柄，之后这个进程号可能分给别人）
    exited: Arc<AtomicBool>,
}

impl Drop for Session {
    /// 句柄都丢了（标签关了、窗口关了）：关掉伪控制台，挂在上面的程序跟着结束，就像关掉终端窗口那样。
    /// 关的时候要等输出读空，放在后台线程上
    fn drop(&mut self) {
        let console = self.console.clone();
        let closer = std::thread::Builder::new()
            .name("关伪控制台".to_string())
            .spawn(move || console.close());
        if closer.is_err() {
            self.console.close();
        }
    }
}

impl PtyBackend for Session {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let sender = self
            .writes
            .lock()
            .map_err(|_| io::Error::other("写入线程的状态锁坏了"))?;
        sender
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "写入线程已经退出"))
    }

    fn resize(&self, columns: u16, rows: u16) {
        self.console.resize(columns, rows);
    }

    fn working_directory(&self) -> Option<PathBuf> {
        if self.exited.load(Ordering::Acquire) {
            return None;
        }
        directory_of(self.process_id)
    }

    fn is_busy(&self) -> bool {
        if self.exited.load(Ordering::Acquire) {
            return false;
        }
        has_children(self.process_id, self.started)
    }
}

/// 起一个挂在伪控制台上的子进程，返回句柄和读字节的队列
pub fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    restore_ctrl_c();
    let mut command_line = command_line(&config.program, &config.arguments)?;
    let environment = environment_block(&config)?;
    let directory = config
        .working_directory
        .as_ref()
        .map(|directory| wide(directory.as_os_str()))
        .transpose()?;

    // 我们写 `input_write`、读 `output_read`；另外两头交给伪控制台
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let mut handle: HPCON = 0;
    // SAFETY: 两个管道句柄都开着（伪控制台复制一份自己用）；结果写到栈上的变量
    let result = unsafe {
        CreatePseudoConsole(
            size(config.columns, config.rows),
            input_read.as_raw_handle(),
            output_write.as_raw_handle(),
            0,
            &mut handle,
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    // 我们这边的副本关掉：不然子进程退出、伪控制台关了，输出管道还有一个写端开着，永远读不到结尾
    drop(input_read);
    drop(output_write);
    let console = Arc::new(Console {
        handle: Mutex::new(Some(handle)),
    });

    let child = match start_process(
        handle,
        &mut command_line,
        &environment,
        directory.as_deref(),
    ) {
        Ok(child) => child,
        Err(error) => {
            // 先断掉输出管道再关：没有人读，关的时候就不会等它被读空
            drop(output_read);
            console.close();
            return Err(error);
        }
    };

    let (queue, producer) = ByteQueue::pair();
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let (code_sender, code_receiver) = mpsc::channel::<i32>();
    let exited = Arc::new(AtomicBool::new(false));
    start_writer(File::from(input_write), receiver);
    start_reader(File::from(output_read), producer, code_receiver);
    start_waiter(child.process, console.clone(), exited.clone(), code_sender);
    let session = Arc::new(Session {
        console,
        writes: Mutex::new(sender),
        process_id: child.id,
        started: child.started,
        exited,
    });
    Ok((Pty { backend: session }, queue))
}

/// 子进程会继承「忽略 Ctrl+C」这个标志：我们自己是被一个忽略它的程序起的（一些启动器、跑测试的那一串），
/// 伪控制台里按 Ctrl+C 就停不下任何程序。每次起子进程之前把自己的恢复成默认（不忽略）
fn restore_ctrl_c() {
    // SAFETY: 处理函数为空、第二个参数为假：只是清掉本进程「忽略 Ctrl+C」的标志，不注册任何东西
    unsafe { SetConsoleCtrlHandler(None, 0) };
}

/// 一根匿名管道：（读的一头，写的一头）。不让子进程继承，伪控制台自己会复制它要的那头
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    // SAFETY: 两个出参指向栈上的变量；安全属性为空（不继承），缓冲大小用系统默认
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 刚建好的两个句柄，只有这里拿着
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

/// 起好的子进程
struct Child {
    process: OwnedHandle,
    id: u32,
    started: u64,
}

/// 用完删掉的线程属性表
struct AttributeList {
    /// 按指针对齐的缓冲，表就放在这里
    buffer: Vec<usize>,
}

impl AttributeList {
    fn as_ptr(&mut self) -> *mut c_void {
        self.buffer.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: 只有初始化成功以后才会造出 `AttributeList`，缓冲还在
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// 一项属性的表：「挂到这个伪控制台上」
fn attribute_list(console: HPCON) -> io::Result<AttributeList> {
    let mut size = 0usize;
    // SAFETY: 表是空指针，这一次只问要多大；按约定它会失败并填好 `size`
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
    // SAFETY: 缓冲至少 `size` 字节、按指针对齐
    if unsafe { InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 1, 0, &mut size) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut list = AttributeList { buffer };
    // SAFETY: 表已经初始化。这一项的值就是伪控制台句柄本身（不是指向它的指针），大小写的是句柄的大小；
    // 伪控制台活得比用这张表的 `CreateProcessW` 久
    let updated = unsafe {
        UpdateProcThreadAttribute(
            list.as_ptr(),
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            console as *const c_void,
            size_of::<HPCON>(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if updated == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(list)
}

fn start_process(
    console: HPCON,
    command_line: &mut [u16],
    environment: &[u16],
    directory: Option<&[u16]>,
) -> io::Result<Child> {
    let mut attributes = attribute_list(console)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // 标准输入输出明说「没有」：不然父进程的标准输出被重定向过时（跑测试、被别的程序起），
    // 子进程会继承那几个句柄，输出跑到那里去而不是伪控制台
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_ptr();
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: 命令行是可写的、以 0 结尾的缓冲（`CreateProcessW` 会临时改里面的字）；环境块以两个 0 结尾；
    // 目录要么是空指针、要么以 0 结尾；启动信息带着属性表（扩展的那种，`EXTENDED_STARTUPINFO_PRESENT` 说明了），
    // 表和这几样都活到调用之后。不继承句柄
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.map_or(ptr::null(), <[u16]>::as_ptr),
            ptr::from_ref(&startup).cast(),
            &mut information,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 成功时两个句柄都是新开的、归我们；线程句柄用不着，马上关
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
        )
    };
    let started = creation_time(process.as_raw_handle()).unwrap_or(0);
    Ok(Child {
        process,
        id: information.dwProcessId,
        started,
    })
}

/// 以 0 结尾的 UTF-16。中间有 0 的不行（Windows 会在那里截断）
fn wide(text: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = text.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(ErrorKind::InvalidInput, "里面有 0 字符"));
    }
    wide.push(0);
    Ok(wide)
}

/// 拼 `CreateProcessW` 要的命令行：程序名和参数，按微软 C 运行库拆参数的规矩加引号（子进程自己再拆回去）
fn command_line(program: &str, arguments: &[String]) -> io::Result<Vec<u16>> {
    // 程序名的拆法和参数不一样（没有反斜杠转义）：整个包进引号就行，所以里面不能有引号（文件名里本来也不会有）
    if program.is_empty() || program.contains('"') {
        return Err(io::Error::new(ErrorKind::InvalidInput, "程序名不能用"));
    }
    let mut line = format!("\"{program}\"");
    for argument in arguments {
        line.push(' ');
        quote_argument(&mut line, argument);
    }
    wide(OsStr::new(&line))
}

/// 把一个参数接到命令行后面：没有空白和引号的原样接；别的包进引号，引号前面的反斜杠加倍、引号本身转义，
/// 结尾的反斜杠也加倍（不然会把收尾的引号转义掉）
fn quote_argument(line: &mut String, argument: &str) {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '\u{b}', '"']) {
        line.push_str(argument);
        return;
    }
    line.push('"');
    let mut backslashes = 0;
    for ch in argument.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(ch);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

/// 子进程的环境：继承我们自己的，再盖上终端要的那几个和 [`Config::environment`]。
/// 写成 `CreateProcessW` 要的块：`名=值\0` 一个接一个，最后再一个 `\0`；名字不分大小写，按大写排好
fn environment_block(config: &Config) -> io::Result<Vec<u16>> {
    let key = |name: &OsStr| name.to_string_lossy().to_uppercase();
    let mut variables: BTreeMap<String, (OsString, OsString)> = std::env::vars_os()
        .map(|(name, value)| (key(&name), (name, value)))
        .collect();
    let overrides = super::terminal_variables(config.dark_background)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .chain(config.environment.iter().cloned());
    for (name, value) in overrides {
        let name = OsString::from(name);
        variables.insert(key(&name), (name, OsString::from(value)));
    }
    let mut block = Vec::new();
    for (name, value) in variables.into_values() {
        let mut entry = name;
        entry.push("=");
        entry.push(value);
        let mut entry = wide(&entry)?;
        block.append(&mut entry);
    }
    // 空的块也要两个 0
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

/// 读的一端：一直堵在输出管道上，读到多少就交给主线程多少；读到结尾，等来退出码再报结束
fn start_reader(mut output: File, producer: Producer, exit_code: mpsc::Receiver<i32>) {
    std::thread::Builder::new()
        .name("pty 读取".to_string())
        .spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            let mut wanted = true;
            loop {
                match output.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        // 取的那头没了（窗口关了）也接着读、读到的丢掉：关伪控制台要等这根管道读空
                        if wanted && !producer.push(Chunk::Bytes(buffer[..read].to_vec())) {
                            wanted = false;
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            // 读到结尾说明伪控制台关了：子进程已经退出，或者正被结束（句柄丢了），退出码马上就到
            let code = exit_code.recv().unwrap_or(-1);
            producer.push(Chunk::Closed(code));
        })
        .expect("起不了读 pty 的线程");
}

/// 写的一端：按键、粘贴都排到这个线程，界面不会因为 shell 没在吃输入而卡住
fn start_writer(mut input: File, receiver: mpsc::Receiver<Vec<u8>>) {
    std::thread::Builder::new()
        .name("pty 写入".to_string())
        .spawn(move || {
            while let Ok(bytes) = receiver.recv() {
                // 子进程没了：丢掉就行，读线程会报结束
                if input.write_all(&bytes).is_err() {
                    break;
                }
            }
        })
        .expect("起不了写 pty 的线程");
}

/// 等的一端：子进程退出后关伪控制台（读线程这才读得到结尾），把退出码交给读线程。
/// 手里的进程句柄开着，这个进程号就不会分给别的进程；先置 `exited` 再放掉句柄
fn start_waiter(
    process: OwnedHandle,
    console: Arc<Console>,
    exited: Arc<AtomicBool>,
    exit_code: mpsc::Sender<i32>,
) {
    std::thread::Builder::new()
        .name("pty 等待".to_string())
        .spawn(move || {
            // SAFETY: 句柄是这个线程自己拿着的子进程句柄，一直开着
            unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
            let mut code = 0u32;
            // SAFETY: 同一个句柄，子进程已经退出；出参指向栈上的变量
            let got = unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) };
            exited.store(true, Ordering::Release);
            console.close();
            // Windows 的退出码是无符号的：被结束的进程常见 0xC000013A 这种，按位转过来是负数
            let _ = exit_code.send(if got != 0 { code as i32 } else { -1 });
            drop(process);
        })
        .expect("起不了等子进程的线程");
}

/// 进程的创建时间（从 1601 年起的 100 纳秒数）
fn creation_time(process: HANDLE) -> Option<u64> {
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    // SAFETY: 句柄开着、至少带查询权限；四个出参都指向栈上的 `FILETIME`
    let got = unsafe { GetProcessTimes(process, creation, exit, kernel, user) };
    (got != 0)
        .then(|| (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime))
}

/// `shell` 下面有没有子进程在跑。Windows 没有「前台进程组」，shell 下面挂着别的进程就算忙：
/// 跑着的命令行程序是，从 shell 里打开、没等它结束的窗口程序（记事本之类）也会算进来。
///
/// 父进程号是创建时记下的、之后不更新：一个老进程的父进程早退出了、那个号又分给了我们的 shell，
/// 它看起来也像子进程。所以还要比创建时间，比 shell 早的不算
fn has_children(shell: u32, started: u64) -> bool {
    // SAFETY: 拍一张全系统进程的快照，失败返回 `INVALID_HANDLE_VALUE`
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: 刚拍的快照，归我们
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: 快照开着，`entry` 填好了 `dwSize`
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) } != 0;
    while more {
        if entry.th32ParentProcessID == shell && created_since(entry.th32ProcessID, started) {
            return true;
        }
        // SAFETY: 同上
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    false
}

/// 进程 `process_id` 是不是在 `since` 之后（或同时）创建的。已经退出的不算；打不开的（比如管理员身份起的）当它是
fn created_since(process_id: u32, since: u64) -> bool {
    // SAFETY: 只要「查有限的信息」的权限，失败返回空
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() {
        return io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32);
    }
    // SAFETY: 刚打开的句柄，归我们
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    creation_time(handle.as_raw_handle()).is_none_or(|created| created >= since)
}

/// 进程当前所在的目录：读它的进程环境块（PEB）里的 `CurrentDirectory`。
///
/// 这块结构没有公开文档，偏移量是 64 位 Windows 上一直没变过的；32 位的子进程（WOW64）布局不一样，不读。
/// PowerShell 的 `cd`（`Set-Location`）不改进程的当前目录，对它读到的一直是启动时的目录，要靠 shell 集成报
#[cfg(target_pointer_width = "64")]
fn directory_of(process_id: u32) -> Option<PathBuf> {
    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows_sys::Win32::System::Threading::{
        IsWow64Process, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    /// `PEB.ProcessParameters`
    const PROCESS_PARAMETERS: usize = 0x20;
    /// `RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath`
    const CURRENT_DIRECTORY: usize = 0x38;
    /// Windows 路径最长 32767 个字符
    const LONGEST: usize = 32767;

    // SAFETY: 打开子进程，要查询和读内存的权限，失败返回空
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, process_id) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: 刚打开的句柄，归我们
    let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
    let process = owned.as_raw_handle();
    let mut wow64 = 0;
    // SAFETY: 句柄开着，出参指向栈上的变量
    if unsafe { IsWow64Process(process, &mut wow64) } == 0 || wow64 != 0 {
        return None;
    }
    let mut basic = PROCESS_BASIC_INFORMATION::default();
    // SAFETY: 出参是栈上的 `PROCESS_BASIC_INFORMATION`，长度写的就是它的大小
    let status = unsafe {
        NtQueryInformationProcess(
            process,
            ProcessBasicInformation,
            ptr::from_mut(&mut basic).cast(),
            size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            ptr::null_mut(),
        )
    };
    if status < 0 {
        return None;
    }
    let mut parameters = [0u8; 8];
    read_memory(
        process,
        basic.PebBaseAddress as usize + PROCESS_PARAMETERS,
        &mut parameters,
    )?;
    let parameters = usize::from_ne_bytes(parameters);
    // `UNICODE_STRING`：长度（字节，u16）、容量（u16）、4 字节空、缓冲的地址
    let mut directory = [0u8; 16];
    read_memory(process, parameters + CURRENT_DIRECTORY, &mut directory)?;
    let length = usize::from(u16::from_ne_bytes([directory[0], directory[1]])) / 2;
    let buffer = usize::from_ne_bytes(directory[8..16].try_into().ok()?);
    if length == 0 || length > LONGEST {
        return None;
    }
    let mut bytes = vec![0u8; length * 2];
    read_memory(process, buffer, &mut bytes)?;
    let mut text: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect();
    // 存的是带结尾反斜杠的（`C:\Users\x\`）；根目录（`C:\`）留着
    while text.len() > 3 && text.last() == Some(&u16::from(b'\\')) {
        text.pop();
    }
    Some(PathBuf::from(OsString::from_wide(&text)))
}

#[cfg(not(target_pointer_width = "64"))]
fn directory_of(_process_id: u32) -> Option<PathBuf> {
    None
}

/// 从进程 `process` 的地址 `address` 读满 `buffer`。读不全是 `None`
#[cfg(target_pointer_width = "64")]
fn read_memory(process: HANDLE, address: usize, buffer: &mut [u8]) -> Option<()> {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;

    let mut read = 0usize;
    // SAFETY: 往自己拥有的 `buffer` 里写最多它那么长；对方的地址读不了只会返回失败，不会碰到我们的内存
    let done = unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut read,
        )
    };
    (done != 0 && read == buffer.len()).then_some(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::*;

    /// 起 `cmd /d /c 脚本`，80 × 24（`/d`：不跑注册表里的自动运行命令）
    fn cmd(script: &str) -> Config {
        command("cmd.exe", &["/d", "/c", script])
    }

    /// 起 `powershell -NoProfile -Command 脚本`：要问控制台的东西（大小）时用，cmd 的 `mode con` 是本地化的
    fn powershell(script: &str) -> Config {
        command(
            "powershell.exe",
            &["-NoLogo", "-NoProfile", "-Command", script],
        )
    }

    fn command(program: &str, arguments: &[&str]) -> Config {
        Config {
            program: program.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect(),
            columns: 80,
            rows: 24,
            working_directory: None,
            dark_background: false,
            environment: Vec::new(),
        }
    }

    /// 读队列，直到子进程退出（带退出码），或者读到的文字里出现了 `until`（退出码是 `None`）。
    /// 读线程在后台跑，这里轮询；20 秒没等到就当失败，免得测试挂住
    fn collect(queue: &ByteQueue, until: Option<&str>) -> (String, Option<i32>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut output = Vec::new();
        loop {
            match queue.try_next() {
                Some(Chunk::Bytes(bytes)) => {
                    output.extend_from_slice(&bytes);
                    let text = String::from_utf8_lossy(&output);
                    if until.is_some_and(|needle| text.contains(needle)) {
                        return (text.into_owned(), None);
                    }
                }
                Some(Chunk::Closed(code)) => {
                    return (String::from_utf8_lossy(&output).into_owned(), Some(code));
                }
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "等了 20 秒，到现在读到的是 {:?}",
                        String::from_utf8_lossy(&output)
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// 读到子进程退出：全部输出和退出码
    fn drain(queue: &ByteQueue) -> (String, i32) {
        let (output, code) = collect(queue, None);
        (
            output,
            code.expect("没有等待的内容时，只会因为子进程退出而返回"),
        )
    }

    /// 反复问，直到 `done` 成立或者等了 10 秒
    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        done()
    }

    fn quoted(argument: &str) -> String {
        let mut line = String::new();
        quote_argument(&mut line, argument);
        line
    }

    /// 微软 C 运行库拆参数的规矩：反斜杠只有在引号前面才是转义
    #[test]
    fn arguments_are_quoted_the_way_the_c_runtime_splits_them() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(r"C:\dir\file"), r"C:\dir\file");
        assert_eq!(quoted(""), "\"\"");
        assert_eq!(quoted("a b"), "\"a b\"");
        assert_eq!(quoted("say \"hi\""), r#""say \"hi\"""#);
        assert_eq!(quoted(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
    }

    #[test]
    fn a_program_name_with_a_quote_is_refused() {
        assert!(command_line("a\"b", &[]).is_err());
        assert!(command_line("", &[]).is_err());
    }

    /// 名字不分大小写：后设的盖过先设的，不会出现两份
    #[test]
    fn environment_overrides_ignore_case() {
        let mut config = cmd("");
        config.environment = vec![
            ("Nimbus_Case".to_string(), "1".to_string()),
            ("NIMBUS_CASE".to_string(), "2".to_string()),
        ];
        let block = String::from_utf16_lossy(&environment_block(&config).expect("拼得出来"));
        let entries: Vec<&str> = block.split('\0').collect();
        assert!(entries.contains(&"NIMBUS_CASE=2"), "{entries:?}");
        assert!(
            !entries
                .iter()
                .any(|entry| entry.starts_with("Nimbus_Case="))
        );
        assert!(block.ends_with("\0\0"));
    }

    #[test]
    fn output_and_exit_code_come_back() {
        let (_pty, queue) = spawn(cmd("echo hi& exit /b 3")).expect("起得来");
        let (output, code) = drain(&queue);
        assert!(output.contains("hi"), "{output:?}");
        assert_eq!(code, 3);
    }

    /// 程序不存在：直接报错（不像 Unix 那样起了再用 127 退出）
    #[test]
    fn a_missing_program_is_an_error() {
        let error = spawn(command(r"C:\nonexistent\shell.exe", &[])).err();
        assert_eq!(error.map(|error| error.kind()), Some(ErrorKind::NotFound));
    }

    /// 起的时候就是给定的行列数，之后 `resize` 改了子进程也看得到；同时走通了写线程
    #[test]
    fn the_child_sees_the_initial_size_and_later_resizes() {
        let script = "'size={0}x{1}' -f [Console]::WindowWidth,[Console]::WindowHeight; \
                      [void][Console]::ReadLine(); \
                      'size={0}x{1}' -f [Console]::WindowWidth,[Console]::WindowHeight";
        let (pty, queue) = spawn(powershell(script)).expect("起得来");
        collect(&queue, Some("size=80x24"));
        pty.resize(100, 30);
        pty.write(b"go\r").expect("写得进去");
        let (output, code) = drain(&queue);
        assert!(output.contains("size=100x30"), "{output:?}");
        assert_eq!(code, 0);
    }

    /// 终端的那几个变量、调用方加的变量都到了子进程那里；中文经过伪控制台出来还是 UTF-8
    #[test]
    fn the_child_gets_the_terminal_environment() {
        let mut config =
            cmd("echo [%TERM%][%COLORTERM%][%COLORFGBG%][%TERM_PROGRAM%][%NIMBUS_EXTRA%]");
        config.environment = vec![("NIMBUS_EXTRA".to_string(), "中文".to_string())];
        let (_pty, queue) = spawn(config).expect("起得来");
        let (output, _) = drain(&queue);
        assert!(
            output.contains("[xterm-256color][truecolor][0;15][Nimbus][中文]"),
            "{output:?}"
        );
    }

    /// 系统目录（`C:\Windows`）：短，不会在 80 列里折行
    fn system_root() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("有 SystemRoot"))
    }

    #[test]
    fn the_child_starts_in_the_requested_directory() {
        let directory = system_root();
        let mut config = cmd("cd");
        config.working_directory = Some(directory.clone());
        let (_pty, queue) = spawn(config).expect("起得来");
        let (output, _) = drain(&queue);
        assert!(output.contains(&*directory.to_string_lossy()), "{output:?}");
    }

    /// 句柄丢了（标签关了）：子进程要跟着结束，不能留在后台
    #[test]
    fn dropping_the_handle_ends_the_child() {
        let (pty, queue) = spawn(cmd("pause")).expect("起得来");
        collect(&queue, Some("."));
        drop(pty);
        drain(&queue);
    }

    /// 程序发的命令标记、超链接要穿过伪控制台，仿真层还认得出来（系统的 ConPTY 会重画屏幕，不认识的序列会被吃掉）。
    /// 实测命令标记原样过来；超链接被改写成 `8;id=…;网址`，结束的那条挪到了后面
    #[test]
    fn marks_and_links_pass_through_the_pseudoconsole() {
        // 只用单引号：命令行里的双引号要转义，PowerShell 5 拆它的规矩又不太一样
        let script = "$e = [char]27; $b = [char]7; \
                      [Console]::Write($e + ']133;A' + $b + '> ' + $e + ']133;B' + $b); \
                      [Console]::Write($e + ']8;;https://example.com/' + $e + '\\link' + $e + ']8;;' + $e + '\\'); \
                      [Console]::Write($e + ']133;D;0' + $b)";
        let (_pty, queue) = spawn(powershell(script)).expect("起得来");
        let (output, _) = drain(&queue);
        for needle in ["\x1b]133;A\x07", "\x1b]133;B\x07", "\x1b]133;D;0\x07"] {
            assert!(output.contains(needle), "没有 {needle:?}：{output:?}");
        }
        let mut terminal = crate::vt::Terminal::new(80, 24);
        terminal.feed(output.as_bytes());
        assert!(terminal.has_command_marks(), "{output:?}");
        assert_eq!(terminal.link_uris(), ["https://example.com/"], "{output:?}");
    }

    /// 问得到 shell 现在在哪个目录：起始目录，以及 `cd` 之后的新目录（cmd 的 `cd` 会改进程的当前目录）
    #[test]
    fn the_working_directory_follows_the_shell() {
        let windows = system_root();
        let mut config = command("cmd.exe", &["/d", "/k"]);
        config.working_directory = Some(windows.clone());
        let (pty, _queue) = spawn(config).expect("起得来");
        assert!(
            wait_until(|| pty.backend.working_directory().as_deref() == Some(windows.as_path())),
            "起始目录：{:?}",
            pty.backend.working_directory()
        );
        pty.write(b"cd \\\r").expect("写得进去");
        // `C:\Windows` 所在盘的根目录 `C:\`
        let root = windows.ancestors().last().expect("有根目录").to_path_buf();
        assert!(
            wait_until(|| pty.backend.working_directory().as_deref() == Some(Path::new(&root))),
            "cd 之后：{:?}",
            pty.backend.working_directory()
        );
    }

    /// shell 下面有程序在跑才算忙：等输入时不忙，`ping` 跑着时忙，Ctrl-C 之后又不忙。
    /// 测试进程是从 bash、cargo 一串里起的，继承了「忽略 Ctrl+C」：这里同时测了 [`restore_ctrl_c`]
    #[test]
    fn busy_means_the_shell_has_a_program_running() {
        let (pty, queue) = spawn(command("cmd.exe", &["/d", "/k"])).expect("起得来");
        collect(&queue, Some(">"));
        assert!(!pty.backend.is_busy(), "等输入时不忙");
        pty.write(b"ping -n 30 127.0.0.1\r").expect("写得进去");
        assert!(wait_until(|| pty.backend.is_busy()), "ping 跑着时忙");
        pty.write(b"\x03").expect("写得进去");
        assert!(wait_until(|| !pty.backend.is_busy()), "Ctrl-C 之后不忙");
    }
}
