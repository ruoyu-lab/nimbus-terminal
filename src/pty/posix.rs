//! 类 Unix 的 pty：`forkpty` 起子进程，主端两个线程读和写。
//!
//! 在 macOS 上实测；Linux 用的是同一套接口（`forkpty`、`TIOCSWINSZ`），但没有在真实桌面上跑过。

use std::ffi::{CString, c_char, c_int};
use std::io::{self, ErrorKind};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use super::queue::{ByteQueue, Chunk, Producer};
use super::{Config, Pty, PtyBackend};

/// 子进程会自己去读这些环境变量，我们只负责在 `exec` 之前把它们设好
const ENVIRONMENT: &[(&str, &str)] = &[
    // 仿真层按 xterm 的口径实现，报这个 shell 才会发 256 色和真彩
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "aureole-terminal"),
];

/// 从 Finder / Dock 启动的应用没有 `LANG`，shell 会退回 C 语言环境：中文显示成问号、
/// 行编辑按字节退格。当前环境里没有一个明确的 UTF-8 区域设置时补一个
fn locale_fallback() -> Option<(&'static str, &'static str)> {
    let is_utf8 = |name: &str| {
        std::env::var(name).is_ok_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
    };
    let overridden = std::env::var("LC_ALL").is_ok_and(|value| !value.is_empty());
    if overridden || is_utf8("LANG") || is_utf8("LC_CTYPE") {
        None
    } else {
        Some(("LANG", "en_US.UTF-8"))
    }
}

/// pty 的主端文件描述符：谁最后拿着它谁负责关
struct Master {
    fd: RawFd,
}

impl Drop for Master {
    fn drop(&mut self) {
        // SAFETY: `fd` 是 `forkpty` 给回来的主端。读、写两个线程各持一份 `Arc`，
        // 走到这里说明两边都用完了，只关这一次
        unsafe {
            libc::close(self.fd);
        }
    }
}

struct Session {
    master: Arc<Master>,
    /// 交给写线程的口子。锁一下主要是为了满足 `Sync`，一次按键就进一次
    writes: Mutex<mpsc::Sender<Vec<u8>>>,
    process_id: c_int,
    /// 子进程已经退出并且被收过尸了（这时候再 `kill` 它的 pid 可能杀到别人）
    exited: Arc<AtomicBool>,
}

impl Drop for Session {
    /// 句柄都丢了（标签关了、窗口关了）：给子进程发 `SIGHUP`，就像关掉终端窗口那样。
    /// 不发的话，读线程还握着主端，shell 会一直活着
    fn drop(&mut self) {
        if !self.exited.load(Ordering::Acquire) {
            // SAFETY: `kill` 只是给一个进程发信号。`exited` 没置位说明读线程还没收过尸，
            // 这个 pid 还是我们那个子进程的（收尸之前 pid 不会被系统重新分配）
            unsafe {
                libc::kill(self.process_id, libc::SIGHUP);
            }
        }
    }
}

impl PtyBackend for Session {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let sender = self.writes.lock().map_err(|_| io::Error::other("写入线程的状态锁坏了"))?;
        sender.send(bytes.to_vec()).map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "写入线程已经退出"))
    }

    fn resize(&self, columns: u16, rows: u16) {
        let size = libc::winsize { ws_row: rows, ws_col: columns, ws_xpixel: 0, ws_ypixel: 0 };
        // SAFETY: `fd` 是还开着的 pty 主端，`size` 是栈上的 `winsize`，请求号 `TIOCSWINSZ` 说明带的就是它。
        // 改不动就算了（子进程可能正好退出）：行列对不上比崩掉好，这里也没有能报告给它的地方
        let _failed = unsafe { libc::ioctl(self.master.fd, libc::TIOCSWINSZ, &size) };
    }

    fn process_id(&self) -> i32 {
        self.process_id
    }
}

/// 起一个 pty 子进程，返回句柄和读字节的队列
pub fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    let program = CString::new(config.program).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "程序名里有 0 字节"))?;
    let mut arguments = vec![program.clone()];
    for argument in &config.arguments {
        arguments.push(CString::new(argument.as_str()).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "参数里有 0 字节"))?);
    }
    // `execvp` 要的是以 `null` 结尾的指针数组。`arguments` 那些 `CString` 一直活到函数结束，
    // 指针在 `fork` 之后、`exec` 之前这段路上才不会失效
    let argv: Vec<*const c_char> = arguments.iter().map(|argument| argument.as_ptr()).chain(std::iter::once(ptr::null())).collect();
    let directory = config
        .working_directory
        .as_ref()
        .map(|path| CString::new(path.as_os_str().as_bytes()))
        .transpose()
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "目录名里有 0 字节"))?;
    // `fork` 之后不再分配内存，所以所有字符串现在就备好
    let variables: Vec<(CString, CString)> = ENVIRONMENT
        .iter()
        .copied()
        .chain(locale_fallback())
        .map(|(name, value)| (CString::new(name).unwrap_or_default(), CString::new(value).unwrap_or_default()))
        .collect();

    let (queue, producer) = ByteQueue::pair();
    let mut master: RawFd = -1;
    let mut size = libc::winsize { ws_row: config.rows, ws_col: config.columns, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: 四个参数分别指向：放主端描述符的变量、`null`（不要终端文件名，终端属性用内核默认的）、
    // 窗口大小。子进程不会从这一行返回（要么 `exec` 换掉地址空间，要么 `_exit`）
    let process_id = unsafe { libc::forkpty(&mut master, ptr::null_mut(), ptr::null_mut(), &mut size) };
    match process_id {
        -1 => Err(io::Error::last_os_error()),
        0 => child(&argv, &variables, directory.as_ref()),
        pid => {
            let master = Arc::new(Master { fd: master });
            let (sender, receiver) = mpsc::channel::<Vec<u8>>();
            let exited = Arc::new(AtomicBool::new(false));
            start_writer(master.clone(), receiver);
            start_reader(master.clone(), producer, pid, exited.clone());
            let session = Arc::new(Session { master, writes: Mutex::new(sender), process_id: pid, exited });
            Ok((Pty { backend: session }, queue))
        }
    }
}

/// 子进程这条路不返回：换目录、设环境变量、变成 shell
fn child(arguments: &[*const c_char], variables: &[(CString, CString)], directory: Option<&CString>) -> ! {
    // 行编辑（回车前的退格、方向键）按字符而不是按字节删：内核的终端驱动要知道这是 UTF-8
    // SAFETY: 标准输入此刻就是 `forkpty` 给子进程接好的从端；`termios` 是栈上的结构，`tcgetattr` 填满它才用
    unsafe {
        let mut attributes: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut attributes) == 0 {
            attributes.c_iflag |= libc::IUTF8;
            libc::tcsetattr(0, libc::TCSANOW, &attributes);
        }
    }
    if let Some(directory) = directory {
        // SAFETY: 字符串是 `fork` 之前准备好的 `CString`，`chdir` 只读它
        unsafe {
            libc::chdir(directory.as_ptr());
        }
    }
    for (name, value) in variables {
        // SAFETY: 两个都是活的 `CString`；最后的 `1` 表示覆盖已有的值
        unsafe {
            libc::setenv(name.as_ptr(), value.as_ptr(), 1);
        }
    }
    // SAFETY: `arguments[0]` 是程序名，数组末尾有 `null`，指向的字符串都还活着
    unsafe {
        libc::execvp(arguments[0], arguments.as_ptr());
    }
    // SAFETY: `_exit` 不跑清理、不刷缓冲。`exec` 失败时这样退出去，才不会把父进程的输出搅乱
    unsafe {
        libc::_exit(127);
    }
}

/// 读的一端：一直堵在 `read` 上，读到多少就交给主线程多少
fn start_reader(master: Arc<Master>, producer: Producer, process_id: c_int, exited: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("pty 读取".to_string())
        .spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                // SAFETY: `fd` 被这一轮循环手里的 `Arc` 保持着打开；
                // 缓冲是自己拥有的 `Vec`，长度写的就是它的容量
                let read = unsafe { libc::read(master.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
                match read {
                    0 => break,
                    n if n > 0 => {
                        // 队列那头没了（窗口关了）或者队列满了会在这里等；返回 false 就不读了
                        if !producer.push(Chunk::Bytes(buffer[..n as usize].to_vec())) {
                            break;
                        }
                    }
                    -1 => {
                        // 子进程退出以后 macOS 在这里给 `EIO`，那就是「结束了」；别的错误同样只能收摊
                        if io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    _ => break,
                }
            }
            let code = wait(process_id);
            exited.store(true, Ordering::Release);
            producer.push(Chunk::Closed(code));
        })
        .expect("起不了读 pty 的线程");
}

/// 写的一端：按键、粘贴都排到这个线程，界面不会因为 shell 没在吃输入而卡住
fn start_writer(master: Arc<Master>, receiver: mpsc::Receiver<Vec<u8>>) {
    std::thread::Builder::new()
        .name("pty 写入".to_string())
        .spawn(move || {
            while let Ok(bytes) = receiver.recv() {
                let mut written = 0;
                while written < bytes.len() {
                    let rest = &bytes[written..];
                    // SAFETY: 同一个还开着的 `fd`；`rest` 是自己拥有的切片
                    let count = unsafe { libc::write(master.fd, rest.as_ptr().cast(), rest.len()) };
                    match count {
                        n if n > 0 => written += n as usize,
                        -1 if io::Error::last_os_error().kind() == ErrorKind::Interrupted => {}
                        // 子进程没了：丢掉就行，读线程会报结束
                        _ => break,
                    }
                }
            }
        })
        .expect("起不了写 pty 的线程");
}

/// 等子进程退出，取退出码（被信号打死按 shell 的惯例报 `128 + 信号`）
fn wait(process_id: c_int) -> i32 {
    let mut status = 0;
    // SAFETY: 只写一个 `c_int`，指针指向它自己
    let done = unsafe { libc::waitpid(process_id, &mut status, 0) };
    if done < 0 {
        return -1;
    }
    if status & 0x7f != 0 {
        128 + (status & 0x7f)
    } else {
        (status >> 8) & 0xff
    }
}
