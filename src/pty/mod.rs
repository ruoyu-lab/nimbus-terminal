//! pty 层：起一个子进程，把它的终端接到我们手上。
//!
//! 跨平台的接缝是 [`PtyBackend`]：类 Unix 走 `forkpty`（[`posix`]，在 macOS 上跑，Linux 是同一套代码），
//! Windows 要另写一份 ConPTY 实现，那边现在调用会直接报错，而不是编出一个能过但跑不对的二进制。
//!
//! 数据走两条线：
//! - **读**：后台线程堵住 pty 的读端，读到多少就塞进 [`ByteQueue`]，主线程 `await` 它
//! - **写**：主线程只管把字节交给写线程，永不因为 shell 没在吃输入而卡住界面

mod posix;
mod queue;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

pub use queue::{ByteQueue, Chunk};

/// 一个跑着的子进程要做到的几件事
pub trait PtyBackend: Send + Sync {
    /// 写进 shell 的标准输入
    fn write(&self, bytes: &[u8]) -> io::Result<()>;
    /// 改窗口大小（`TIOCSWINSZ`，子进程会收到 `SIGWINCH`）
    fn resize(&self, columns: u16, rows: u16);
    /// 子进程的 pid
    fn process_id(&self) -> i32;
}

/// 应用手里的那份句柄：可以克隆，随便带到哪个闭包里
#[derive(Clone)]
pub struct Pty {
    backend: Arc<dyn PtyBackend>,
}

impl Pty {
    /// 用别的后端造一个句柄（测试里放一个只记录写入的假的）
    #[cfg(test)]
    pub(crate) fn from_backend(backend: Arc<dyn PtyBackend>) -> Self {
        Self { backend }
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.backend.write(bytes)
    }

    pub fn resize(&self, columns: u16, rows: u16) {
        self.backend.resize(columns, rows)
    }

    pub fn process_id(&self) -> i32 {
        self.backend.process_id()
    }
}

/// 起一个子进程要说的东西
pub struct Config {
    /// 要跑的程序，一般是 `$SHELL`
    pub program: String,
    /// 传给它的参数（登录 shell 用 `-l`，让 `/etc/zprofile` 那些跑一遍）
    pub arguments: Vec<String>,
    pub columns: u16,
    pub rows: u16,
    /// 起始目录，`None` 用当前进程的
    pub working_directory: Option<PathBuf>,
}

impl Config {
    /// 用户默认的 shell，加上登录 shell 该有的那几个参数
    pub fn login_shell(columns: u16, rows: u16) -> Self {
        let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        // zsh、bash 当登录 shell 跑要带 `-l`，不然 `/etc/zprofile`（macOS 上管 PATH 的那个）不执行
        let name = PathBuf::from(&program).file_name().map(|name| name.to_string_lossy().into_owned());
        let arguments = match name.as_deref() {
            Some("zsh") | Some("bash") | Some("sh") | Some("fish") => vec!["-l".to_string()],
            _ => Vec::new(),
        };
        Self { program, arguments, columns, rows, working_directory: std::env::current_dir().ok() }
    }
}

/// 起一个 pty，返回句柄和读到的字节
pub fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    #[cfg(unix)]
    return posix::spawn(config);
    #[cfg(not(unix))]
    {
        let _ = config;
        Err(io::Error::new(io::ErrorKind::Unsupported, "这一版只在 macOS（类 Unix）上实现了 pty，Windows 的 ConPTY 还没写"))
    }
}

/// 测试用：记下写给 shell 的字节的假 pty
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Recorder(std::sync::Mutex<Vec<u8>>);

#[cfg(test)]
impl PtyBackend for Recorder {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.0.lock().expect("锁").extend_from_slice(bytes);
        Ok(())
    }
    fn resize(&self, _: u16, _: u16) {}
    fn process_id(&self) -> i32 {
        0
    }
}

#[cfg(test)]
impl Recorder {
    /// 取走到现在为止写进去的字节
    pub(crate) fn take(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.0.lock().expect("锁"))).into_owned()
    }
}
