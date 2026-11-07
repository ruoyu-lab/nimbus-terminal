//! Windows 的 pty：ConPTY 起一个控制台程序，管道两端各一个线程读和写。
//!
//! 伪终端自己负责把控制台画面收成 UTF-8。关句柄时先关掉伪终端，还没退出的进程再结束掉，
//! 相当于类 Unix 上关掉终端窗口时的 `SIGHUP`。

use std::ffi::{OsStr, c_void};
use std::io::{self, ErrorKind};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STILL_ACTIVE,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, STARTUPINFOW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::queue::{ByteQueue, Chunk, Producer};
use super::{Config, Pty, PtyBackend};

/// `NtQueryInformationProcess` 的类别：基本信息（里面有 PEB 地址）
const PROCESS_BASIC_INFORMATION: i32 = 0;
/// 32 位进程在 64 位系统上的 PEB 地址
const PROCESS_WOW64_INFORMATION: i32 = 26;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: i32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

/// 用过就关的内核句柄。`0` 和无效值都不当成开着的
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: 句柄是 `Create*` 给回来的，只在这里关一次
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: 内核句柄可以拿到别的线程上去读、写、取消。关掉只发生在 `Drop`，`Arc` 保证只有最后一份会关
unsafe impl Send for OwnedHandle {}
// SAFETY: 同一个句柄上的 `ReadFile`、`WriteFile`、`CancelIoEx` 可以由不同线程调用，系统自己串行
unsafe impl Sync for OwnedHandle {}

/// 伪终端。丢掉它，连着的控制台程序会收到关窗口
struct PseudoConsole(HPCON);

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: `HPCON` 是 `CreatePseudoConsole` 给回来的，只关这一次
            unsafe {
                ClosePseudoConsole(self.0);
            }
            self.0 = 0;
        }
    }
}

struct Session {
    /// 读线程也拿着一份：等它的 `ReadFile` 返回之后，最后一份丢掉才关伪终端。
    /// 读还堵着的时候关，`ClosePseudoConsole` 会和这次读取互相等
    console: Arc<PseudoConsole>,
    process: Arc<OwnedHandle>,
    output: Arc<OwnedHandle>,
    /// 交给写线程的口子。锁一下主要是为了满足 `Sync`
    writes: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    process_id: u32,
    /// 子进程已经退出（这时候退出码已经取走）
    exited: Arc<AtomicBool>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        // SAFETY: `output` 就是读线程堵着的那一端。取消之后 `ReadFile` 返回，再关伪终端才不会互相等。
        // 进程已经退出时 `TerminateProcess` 失败，无害
        unsafe {
            CancelIoEx(self.output.raw(), ptr::null());
            if !self.exited.load(Ordering::Acquire) {
                TerminateProcess(self.process.raw(), 1);
            }
        }
    }
}

impl PtyBackend for Session {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let sender = self
            .writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(sender) = sender.as_ref() else {
            return Err(io::Error::new(ErrorKind::BrokenPipe, "写入线程已经退出"));
        };
        sender
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "写入线程已经退出"))
    }

    fn resize(&self, columns: u16, rows: u16) {
        let size = coord(columns, rows);
        // SAFETY: `HPCON` 还由这个会话拿着。改不动就算了（进程可能正好退出）
        let _failed = unsafe { ResizePseudoConsole(self.console.0, size) };
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
        // 伪终端的宿主进程（conhost）不算前台程序。shell 下面还有别的进程，就是有程序在跑
        has_foreground_program(self.process_id)
    }
}

/// 系统默认的 shell：先 PowerShell 7，再 Windows PowerShell，最后 `cmd.exe`
pub(super) fn default_shell() -> PathBuf {
    for name in ["pwsh.exe", "powershell.exe"] {
        if let Some(path) = find_on_path(name) {
            return path;
        }
    }
    system_root().join(r"System32\cmd.exe")
}

pub(super) fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    let program = resolve_program(&config.program);
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    // 留给自己的这两端不能被子进程继承，不然它退出以后管道还关不上
    clear_inherit(input_write.raw())?;
    clear_inherit(output_read.raw())?;
    let console = pseudo_console(
        input_read.raw(),
        output_write.raw(),
        config.columns,
        config.rows,
    )?;
    // 伪终端已经复制了自己用的那两端
    drop(input_read);
    drop(output_write);

    let mut command = command_line(&program, &config.arguments);
    let application = wide_null(program.as_os_str());
    let environment = environment_block(config.dark_background);
    let directory = config
        .working_directory
        .as_ref()
        .map(|path| wide_null(path.as_os_str()));
    let attributes = attribute_list(console.0)?;
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.list;
    // 父进程的标准输入输出可能被重定向（测试、从终端里 `cargo run`）。不把它们标成无效的话，
    // 子进程会写到那边，而不是写进伪终端
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `application`、`command`、`environment`、`directory` 都是以 0 结尾的 UTF-16，活到这次调用结束。
    // `startup` 开头就是 `STARTUPINFOW`，`cb` 写成了扩展结构的大小，系统按这个去读属性列表。
    // 不继承句柄：控制台是通过属性列表交出去的，不是通过继承
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(ptr::null(), |text| text.as_ptr()),
            ptr::from_ref(&startup).cast::<STARTUPINFOW>(),
            &mut info,
        )
    };
    drop(attributes);
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    let process =
        OwnedHandle::new(info.hProcess).ok_or_else(|| io::Error::other("没有进程句柄"))?;
    if let Some(thread) = OwnedHandle::new(info.hThread) {
        drop(thread);
    }
    let process = Arc::new(process);
    let console = Arc::new(console);
    let output = Arc::new(output_read);
    let (queue, producer) = ByteQueue::pair();
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let exited = Arc::new(AtomicBool::new(false));
    start_writer(input_write, receiver);
    start_reader(
        Arc::clone(&output),
        producer,
        Arc::clone(&process),
        Arc::clone(&console),
        exited.clone(),
    );
    let session = Arc::new(Session {
        console,
        process,
        output,
        writes: Mutex::new(Some(sender)),
        process_id: info.dwProcessId,
        exited,
    });
    Ok((Pty { backend: session }, queue))
}

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        let candidate = directory.join(name);
        candidate.is_file().then_some(candidate)
    })
}

/// 已经是一个文件就用它；只写了名字（`pwsh`、`cmd`）就到 `PATH` 里找，并补上 `.exe`
fn resolve_program(program: &str) -> PathBuf {
    let path = PathBuf::from(program);
    if path.is_file() {
        return path;
    }
    if !program.contains(['\\', '/']) {
        let name = if program.to_ascii_lowercase().ends_with(".exe") {
            program.to_string()
        } else {
            format!("{program}.exe")
        };
        if let Some(found) = find_on_path(&name) {
            return found;
        }
    }
    path
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    // SAFETY: 两个句柄是写出口；安全描述符在栈上，只在这次调用里用
    let ok = unsafe { CreatePipe(&mut read, &mut write, &mut security, 0) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        OwnedHandle::new(read).ok_or_else(|| io::Error::other("管道的读端是空的"))?,
        OwnedHandle::new(write).ok_or_else(|| io::Error::other("管道的写端是空的"))?,
    ))
}

fn clear_inherit(handle: HANDLE) -> io::Result<()> {
    // SAFETY: `handle` 是刚建好的管道端，清掉「子进程继承」这一位
    let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn pseudo_console(
    input: HANDLE,
    output: HANDLE,
    columns: u16,
    rows: u16,
) -> io::Result<PseudoConsole> {
    let mut handle = 0;
    // SAFETY: 两个管道端都开着，一个读一个写；`handle` 是写出口
    let result =
        unsafe { CreatePseudoConsole(coord(columns, rows), input, output, 0, &mut handle) };
    if result < 0 {
        return Err(hresult(result));
    }
    Ok(PseudoConsole(handle))
}

fn coord(columns: u16, rows: u16) -> COORD {
    COORD {
        X: columns.clamp(1, i16::MAX as u16) as i16,
        Y: rows.clamp(1, i16::MAX as u16) as i16,
    }
}

struct AttributeList {
    _memory: Vec<u8>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if !self.list.is_null() {
            // SAFETY: 列表是 `InitializeProcThreadAttributeList` 在这块内存上建的，只删一次
            unsafe {
                DeleteProcThreadAttributeList(self.list);
            }
        }
    }
}

fn attribute_list(console: HPCON) -> io::Result<AttributeList> {
    let mut bytes = 0usize;
    // SAFETY: 第一次用空指针只为了问需要多少字节，系统不会往里面写
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
    }
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut memory = vec![0u8; bytes];
    let list = memory.as_mut_ptr().cast();
    // SAFETY: `memory` 的长度就是上一步问到的字节数；列表建在这块内存上，跟着 `AttributeList` 一起活
    let ok = unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `console` 这个伪终端句柄值本身就是属性的内容（不是指向它的指针），长度是一个句柄的大小。
    // 官方示例就是这样交的。列表还活着
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            console as *const c_void,
            std::mem::size_of::<HPCON>(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if ok == 0 {
        unsafe {
            DeleteProcThreadAttributeList(list);
        }
        return Err(io::Error::last_os_error());
    }
    Ok(AttributeList {
        _memory: memory,
        list,
    })
}

fn hresult(code: i32) -> io::Error {
    let raw = code as u32;
    if raw & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((raw & 0xFFFF) as i32)
    } else {
        io::Error::other(format!("伪终端创建失败：{code:#x}"))
    }
}

fn wide_null(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn command_line(program: &Path, arguments: &[String]) -> Vec<u16> {
    let mut line = quote(&program.to_string_lossy());
    for argument in arguments {
        line.push(' ');
        line.push_str(&quote(argument));
    }
    wide_null(OsStr::new(&line))
}

/// `CommandLineToArgvW` 的引号规则：空格、制表符、引号要包起来，引号前面的反斜杠要加倍
fn quote(text: &str) -> String {
    if !text.is_empty() && !text.contains([' ', '\t', '"']) {
        return text.to_string();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for character in text.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
            out.push('"');
        } else {
            out.push_str(&"\\".repeat(slashes));
            out.push(character);
        }
        slashes = 0;
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

fn environment_block(dark_background: bool) -> Vec<u16> {
    let overrides = [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "Nimbus"),
        ("COLORFGBG", if dark_background { "15;0" } else { "0;15" }),
    ];
    let mut block = Vec::new();
    for (key, value) in std::env::vars_os() {
        if overrides
            .iter()
            .any(|(name, _)| key.eq_ignore_ascii_case(name))
        {
            continue;
        }
        push_variable(&mut block, &key, &value);
    }
    for (key, value) in overrides {
        push_variable(&mut block, OsStr::new(key), OsStr::new(value));
    }
    block.push(0);
    block
}

fn push_variable(block: &mut Vec<u16>, key: &OsStr, value: &OsStr) {
    block.extend(key.encode_wide());
    block.push(u16::from(b'='));
    block.extend(value.encode_wide());
    block.push(0);
}

fn start_reader(
    output: Arc<OwnedHandle>,
    producer: Producer,
    process: Arc<OwnedHandle>,
    console: Arc<PseudoConsole>,
    exited: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("pty 读取".to_string())
        .spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            // 伪终端在客户进程退出之后并不关掉输出管道，堵在 `ReadFile` 上就永远等不到结束。
            // 所以先看管道里有没有字节，没有就等进程退出
            loop {
                match bytes_available(output.raw()) {
                    None => break,
                    Some(0) => {
                        // SAFETY: 进程句柄还开着。等到退出，或者 20 毫秒后再看管道
                        let state = unsafe { WaitForSingleObject(process.raw(), 20) };
                        if state == WAIT_OBJECT_0 {
                            std::thread::sleep(std::time::Duration::from_millis(30));
                            if bytes_available(output.raw()).unwrap_or(0) == 0 {
                                break;
                            }
                        }
                    }
                    Some(available) => {
                        let want = (available as usize).min(buffer.len()) as u32;
                        let mut read = 0u32;
                        // SAFETY: `output` 由这个线程拿着；只读管道里已经有的那些字节，不会堵死
                        let ok = unsafe {
                            ReadFile(
                                output.raw(),
                                buffer.as_mut_ptr(),
                                want,
                                &mut read,
                                ptr::null_mut(),
                            )
                        };
                        if ok == 0 || read == 0 {
                            break;
                        }
                        if !producer.push(Chunk::Bytes(buffer[..read as usize].to_vec())) {
                            break;
                        }
                    }
                }
            }
            let code = exit_code(process.raw());
            exited.store(true, Ordering::Release);
            drop(console);
            producer.push(Chunk::Closed(code));
        })
        .expect("起不了读 pty 的线程");
}

/// 管道里现在有多少字节。管道关了是 `None`
fn bytes_available(handle: HANDLE) -> Option<u32> {
    let mut available = 0u32;
    // SAFETY: `handle` 是还开着的管道读端；只问数量，不取走字节
    let ok = unsafe {
        PeekNamedPipe(
            handle,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

fn start_writer(input: OwnedHandle, receiver: mpsc::Receiver<Vec<u8>>) {
    std::thread::Builder::new()
        .name("pty 写入".to_string())
        .spawn(move || {
            while let Ok(bytes) = receiver.recv() {
                let mut written = 0;
                while written < bytes.len() {
                    let mut count = 0u32;
                    // SAFETY: `input` 由这个线程拿着；`bytes` 是这一轮收到的那一份
                    let ok = unsafe {
                        WriteFile(
                            input.raw(),
                            bytes[written..].as_ptr(),
                            (bytes.len() - written) as u32,
                            &mut count,
                            ptr::null_mut(),
                        )
                    };
                    if ok == 0 || count == 0 {
                        return;
                    }
                    written += count as usize;
                }
            }
        })
        .expect("起不了写 pty 的线程");
}

fn exit_code(process: HANDLE) -> i32 {
    // SAFETY: 句柄还开着。进程还活着就等到它退出，再读退出码
    unsafe {
        if WaitForSingleObject(process, 5_000) != WAIT_OBJECT_0 {
            return -1;
        }
        let mut code = 0u32;
        if GetExitCodeProcess(process, &mut code) == 0 || code == STILL_ACTIVE as u32 {
            return -1;
        }
        code as i32
    }
}

fn directory_of(process_id: u32) -> Option<PathBuf> {
    let opened = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, process_id) };
    let process = OwnedHandle::new(opened)?;
    let path = if wow64(process.raw()) {
        let peb = query_usize(process.raw(), PROCESS_WOW64_INFORMATION)?;
        cwd_at(process.raw(), peb, 0x10, 0x24, 0x28, true)?
    } else {
        let peb = native_peb(process.raw())?;
        cwd_at(process.raw(), peb, 0x20, 0x38, 0x40, false)?
    };
    Some(normalize_dos(&path))
}

/// 进程参数里的当前目录。32 位进程的指针是 4 字节，64 位是 8 字节
fn cwd_at(
    process: HANDLE,
    peb: usize,
    parameters_offset: usize,
    string_offset: usize,
    buffer_offset: usize,
    narrow: bool,
) -> Option<String> {
    let parameters = read_pointer(process, peb + parameters_offset, narrow)?;
    let length = read_u16(process, parameters + string_offset)?;
    if length == 0 || length > 32 * 1024 {
        return None;
    }
    let buffer = read_pointer(process, parameters + buffer_offset, narrow)?;
    let mut bytes = vec![0u8; length as usize];
    read_memory(process, buffer, &mut bytes)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    (!text.is_empty()).then_some(text)
}

fn normalize_dos(path: &str) -> PathBuf {
    let path = path.trim_end_matches('\0');
    let path = path.strip_prefix(r"\??\").unwrap_or(path);
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    if let Some(rest) = path.strip_prefix(r"UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else {
        PathBuf::from(path)
    }
}

fn wow64(process: HANDLE) -> bool {
    use windows_sys::Win32::System::Threading::IsWow64Process;
    let mut wow = 0;
    // SAFETY: `process` 开着，`wow` 是一个 `BOOL` 写出口
    let ok = unsafe { IsWow64Process(process, &mut wow) };
    ok != 0 && wow != 0
}

#[repr(C)]
struct ProcessBasicInformation {
    exit_status: i32,
    peb: *mut c_void,
    affinity: usize,
    base_priority: i32,
    pid: usize,
    parent: usize,
}

fn native_peb(process: HANDLE) -> Option<usize> {
    let mut info: ProcessBasicInformation = unsafe { std::mem::zeroed() };
    let mut returned = 0u32;
    // SAFETY: `info` 是栈上的基本信息结构，长度就是它的大小；只读
    let status = unsafe {
        NtQueryInformationProcess(
            process,
            PROCESS_BASIC_INFORMATION,
            ptr::from_mut(&mut info).cast(),
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            &mut returned,
        )
    };
    (status >= 0 && !info.peb.is_null()).then_some(info.peb as usize)
}

fn query_usize(process: HANDLE, class: i32) -> Option<usize> {
    let mut value = 0usize;
    let mut returned = 0u32;
    // SAFETY: `value` 是一个指针大小的写出口，类别是「32 位 PEB 的地址」，只读
    let status = unsafe {
        NtQueryInformationProcess(
            process,
            class,
            ptr::from_mut(&mut value).cast(),
            std::mem::size_of::<usize>() as u32,
            &mut returned,
        )
    };
    (status >= 0 && value != 0).then_some(value)
}

fn read_pointer(process: HANDLE, address: usize, narrow: bool) -> Option<usize> {
    if narrow {
        read_u32(process, address).map(|value| value as usize)
    } else {
        read_u64(process, address).map(|value| value as usize)
    }
}

fn read_u16(process: HANDLE, address: usize) -> Option<u16> {
    let mut bytes = [0u8; 2];
    read_memory(process, address, &mut bytes)?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u32(process: HANDLE, address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_memory(process, address, &mut bytes)?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u64(process: HANDLE, address: usize) -> Option<u64> {
    let mut bytes = [0u8; 8];
    read_memory(process, address, &mut bytes)?;
    Some(u64::from_le_bytes(bytes))
}

fn read_memory(process: HANDLE, address: usize, buffer: &mut [u8]) -> Option<()> {
    if address == 0 || buffer.is_empty() {
        return None;
    }
    let mut read = 0usize;
    // SAFETY: `address` 是目标进程里的地址，`buffer` 是我们自己的；读失败就返回空
    let ok = unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut read,
        )
    };
    (ok != 0 && read == buffer.len()).then_some(())
}

fn has_foreground_program(process_id: u32) -> bool {
    let Some(snapshot) =
        OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
    else {
        return false;
    };
    let mut processes = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: `snapshot` 开着，`entry.dwSize` 写成了结构的大小
    let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) };
    while more != 0 {
        processes.push((
            entry.th32ProcessID,
            entry.th32ParentProcessID,
            exe_name(&entry),
        ));
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        more = unsafe { Process32NextW(snapshot.raw(), &mut entry) };
    }
    let mut pending = vec![process_id];
    let mut seen = std::collections::HashSet::new();
    while let Some(parent) = pending.pop() {
        if !seen.insert(parent) {
            continue;
        }
        for (id, parent_id, name) in &processes {
            if *parent_id != parent || *id == process_id {
                continue;
            }
            pending.push(*id);
            if !is_console_host(name) {
                return true;
            }
        }
    }
    false
}

fn exe_name(entry: &PROCESSENTRY32W) -> String {
    let end = entry
        .szExeFile
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..end])
}

fn is_console_host(name: &str) -> bool {
    name.eq_ignore_ascii_case("conhost.exe") || name.eq_ignore_ascii_case("OpenConsole.exe")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;

    fn cmd() -> String {
        system_root()
            .join(r"System32\cmd.exe")
            .to_string_lossy()
            .into_owned()
    }

    fn powershell() -> String {
        system_root()
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
            .to_string_lossy()
            .into_owned()
    }

    fn config(program: String, arguments: Vec<String>) -> Config {
        Config {
            program,
            arguments,
            columns: 80,
            rows: 24,
            working_directory: None,
            dark_background: false,
        }
    }

    fn collect(queue: &ByteQueue, until: Option<&str>) -> (String, Option<i32>) {
        let deadline = Instant::now() + Duration::from_secs(10);
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
                        "等了 10 秒，到现在读到的是 {:?}",
                        String::from_utf8_lossy(&output)
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    fn drain(queue: &ByteQueue) -> (String, i32) {
        let (output, code) = collect(queue, None);
        (output, code.expect("只会因为子进程退出而返回"))
    }

    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        done()
    }

    fn same_dir(left: &Path, right: &Path) -> bool {
        left == right
            || left
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    }

    #[test]
    fn the_default_shell_is_a_program_that_exists() {
        let shell = default_shell();
        assert!(shell.is_file(), "{}", shell.display());
        let name = shell
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        assert!(
            name.eq_ignore_ascii_case("pwsh.exe")
                || name.eq_ignore_ascii_case("powershell.exe")
                || name.eq_ignore_ascii_case("cmd.exe"),
            "{name}"
        );
    }

    #[test]
    fn output_and_exit_code_come_back() {
        let (_pty, queue) = spawn(config(
            cmd(),
            vec!["/d".into(), "/c".into(), "echo hi& exit 3".into()],
        ))
        .expect("起得来");
        let (output, code) = drain(&queue);
        assert!(output.contains("hi"), "{output:?}");
        assert_eq!(code, 3);
    }

    #[test]
    fn a_missing_program_fails_to_start() {
        let error = match spawn(config(r"C:\no\such\shell.exe".into(), Vec::new())) {
            Ok(_) => panic!("不该起得来"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn the_child_gets_the_terminal_environment() {
        let (_pty, queue) = spawn(config(
            cmd(),
            vec![
                "/d".into(),
                "/c".into(),
                "echo %TERM%#%COLORTERM%#%COLORFGBG%#%TERM_PROGRAM%".into(),
            ],
        ))
        .expect("起得来");
        let (output, _) = drain(&queue);
        assert!(
            output.contains("xterm-256color#truecolor#0;15#Nimbus"),
            "{output:?}"
        );
    }

    #[test]
    fn the_child_starts_in_the_requested_directory() {
        let directory = std::env::temp_dir().join(format!("nimbus-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("建得了目录");
        let mut launch = config(cmd(), vec!["/d".into(), "/c".into(), "cd".into()]);
        launch.working_directory = Some(directory.clone());
        let (_pty, queue) = spawn(launch).expect("起得来");
        let (output, code) = drain(&queue);
        assert_eq!(code, 0, "{output:?}");
        let wanted = directory.to_string_lossy().to_ascii_lowercase();
        assert!(
            output.to_ascii_lowercase().contains(&wanted),
            "{output:?} 对比 {}",
            directory.display()
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_child_sees_the_initial_size_and_later_resizes() {
        let script = "Write-Host ([Console]::WindowHeight) ([Console]::WindowWidth); Read-Host | Out-Null; Write-Host ([Console]::WindowHeight) ([Console]::WindowWidth)";
        let (pty, queue) = spawn(config(
            powershell(),
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-Command".into(),
                script.into(),
            ],
        ))
        .expect("起得来");
        collect(&queue, Some("24 80"));
        pty.resize(100, 30);
        std::thread::sleep(Duration::from_millis(100));
        pty.write(b"go\r\n").expect("写得进去");
        let (output, code) = drain(&queue);
        assert!(output.contains("30 100"), "{output:?}");
        assert_eq!(code, 0, "{output:?}");
    }

    #[test]
    fn dropping_the_handle_ends_the_child() {
        let (pty, queue) = spawn(config(cmd(), vec!["/d".into(), "/k".into()])).expect("起得来");
        drop(pty);
        let (_output, code) = drain(&queue);
        assert_ne!(code, -1);
    }

    #[test]
    fn the_working_directory_follows_the_shell() {
        let root = std::env::temp_dir().join(format!("nimbus-follow-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).expect("建得了");
        std::fs::create_dir_all(&second).expect("建得了");
        let mut launch = config(cmd(), vec!["/d".into(), "/q".into(), "/k".into()]);
        launch.working_directory = Some(first.clone());
        let (pty, _queue) = spawn(launch).expect("起得来");
        assert!(
            wait_until(|| pty
                .backend
                .working_directory()
                .is_some_and(|path| same_dir(&path, &first))),
            "起始目录：{:?}",
            pty.backend.working_directory()
        );
        let command = format!("cd /d {}\r\n", second.display());
        pty.write(command.as_bytes()).expect("写得进去");
        assert!(
            wait_until(|| pty
                .backend
                .working_directory()
                .is_some_and(|path| same_dir(&path, &second))),
            "cd 之后：{:?}",
            pty.backend.working_directory()
        );
        drop(pty);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn busy_means_a_program_other_than_the_shell_is_running() {
        let (pty, _queue) =
            spawn(config(cmd(), vec!["/d".into(), "/q".into(), "/k".into()])).expect("起得来");
        assert!(
            wait_until(|| pty.backend.working_directory().is_some()),
            "shell 还没起来"
        );
        assert!(!pty.backend.is_busy(), "等输入时不忙");
        pty.write(b"ping -n 30 127.0.0.1\r\n").expect("写得进去");
        assert!(wait_until(|| pty.backend.is_busy()), "ping 跑着时忙");
        pty.write(b"\x03").expect("写得进去");
        assert!(wait_until(|| !pty.backend.is_busy()), "Ctrl-C 之后不忙");
        drop(pty);
    }

    #[test]
    fn dos_paths_lose_the_nt_prefix() {
        assert_eq!(
            normalize_dos(r"\??\C:\Users\a"),
            PathBuf::from(r"C:\Users\a")
        );
        assert_eq!(
            normalize_dos(r"\??\UNC\server\share"),
            PathBuf::from(r"\\server\share")
        );
    }
}
