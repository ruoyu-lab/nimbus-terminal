//! pty 层：起一个子进程，把它的终端接到我们手上。
//!
//! 跨平台的接缝是 [`PtyBackend`]：类 Unix 走 `forkpty`（[`posix`]，在 macOS 上跑，Linux 是同一套代码），
//! Windows 要另写一份 ConPTY 实现，那边现在调用会直接报错，而不是编出一个能过但跑不对的二进制。
//!
//! 数据走两条线：
//! - **读**：后台线程堵住 pty 的读端，读到多少就塞进 [`ByteQueue`]，主线程 `await` 它
//! - **写**：主线程只管把字节交给写线程，永不因为 shell 没在吃输入而卡住界面

#[cfg(unix)]
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
    /// shell 现在所在的目录；问不到（进程已经退出、平台不支持）是 `None`
    fn working_directory(&self) -> Option<PathBuf> {
        None
    }
    /// 前台有没有 shell 之外的程序在跑（`sleep`、`vim`、编译……）；shell 在等输入时是 `false`
    fn is_busy(&self) -> bool {
        false
    }
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

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.backend.working_directory()
    }

    pub fn is_busy(&self) -> bool {
        self.backend.is_busy()
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
    /// 终端的底色是深色：通过 `COLORFGBG` 告诉程序，vim、bat 这类靠它挑配色
    pub dark_background: bool,
}

impl Config {
    /// 跑一个程序，从主目录起
    pub fn command(
        program: impl Into<String>,
        arguments: Vec<String>,
        columns: u16,
        rows: u16,
    ) -> Self {
        Self {
            program: program.into(),
            arguments,
            columns,
            rows,
            working_directory: home_directory(),
            dark_background: false,
        }
    }

    /// 用户默认的 shell，加上登录 shell 该有的那几个参数
    pub fn login_shell(columns: u16, rows: u16) -> Self {
        let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        Self::shell(program, columns, rows)
    }

    /// 跑指定的 shell 程序；认识的 shell 按登录 shell 起
    pub fn shell(program: impl Into<String>, columns: u16, rows: u16) -> Self {
        let program = program.into();
        // zsh、bash 当登录 shell 跑要带 `-l`，不然 `/etc/zprofile`（macOS 上管 PATH 的那个）不执行
        let name = PathBuf::from(&program)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let arguments = match name.as_deref() {
            Some("zsh") | Some("bash") | Some("sh") | Some("fish") => vec!["-l".to_string()],
            _ => Vec::new(),
        };
        Self::command(program, arguments, columns, rows)
    }
}

/// 新会话的起始目录：用户的主目录。从访达、程序坞、开始菜单启动的应用，当前目录不是主目录，不能拿来当起点；
/// 没有主目录时返回 `None`，由子进程沿用当前目录
fn home_directory() -> Option<PathBuf> {
    crate::store::user_home()
}

/// 起一个 pty，返回句柄和读到的字节
pub fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    #[cfg(unix)]
    return posix::spawn(config);
    #[cfg(not(unix))]
    {
        let _ = config;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "这一版只在 macOS（类 Unix）上实现了 pty，Windows 的 ConPTY 还没写",
        ))
    }
}

/// 测试用：记下写给 shell 的字节的假 pty
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Recorder {
    written: std::sync::Mutex<Vec<u8>>,
    busy: std::sync::atomic::AtomicBool,
    directory: std::sync::Mutex<Option<PathBuf>>,
}

#[cfg(test)]
impl PtyBackend for Recorder {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.written.lock().expect("锁").extend_from_slice(bytes);
        Ok(())
    }
    fn resize(&self, _: u16, _: u16) {}
    fn is_busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn working_directory(&self) -> Option<PathBuf> {
        self.directory.lock().expect("锁").clone()
    }
}

#[cfg(test)]
impl Recorder {
    /// 取走到现在为止写进去的字节
    pub(crate) fn take(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.written.lock().expect("锁")))
            .into_owned()
    }

    /// 假装 shell 现在在这个目录
    pub(crate) fn set_directory(&self, directory: impl Into<PathBuf>) {
        *self.directory.lock().expect("锁") = Some(directory.into());
    }

    /// 假装前台有程序在跑
    pub(crate) fn set_busy(&self, busy: bool) {
        self.busy.store(busy, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试进程的当前目录是项目目录，不是主目录：起始目录要是主目录，说明没有拿当前目录凑合
    #[test]
    fn a_login_shell_starts_in_the_home_directory() {
        assert_eq!(
            Config::login_shell(80, 24).working_directory,
            home_directory()
        );
    }
}
