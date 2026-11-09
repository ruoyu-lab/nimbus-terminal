//! pty 层：起一个子进程，把它的终端接到我们手上。
//!
//! 跨平台的接缝是 [`PtyBackend`]：类 Unix 走 `forkpty`（[`posix`]，在 macOS 上跑，Linux 是同一套代码），
//! Windows 走 ConPTY（[`windows`]）。
//! 加一个平台只要在这里加一个后端文件，再接到 [`spawn`] 和 [`default_shell`]，上面的代码不用动。
//!
//! 数据走两条线：
//! - **读**：后台线程堵住 pty 的读端，读到多少就塞进 [`ByteQueue`]，主线程 `await` 它
//! - **写**：主线程只管把字节交给写线程，永不因为 shell 没在吃输入而卡住界面

#[cfg(unix)]
mod posix;
mod queue;
#[cfg(windows)]
mod windows;

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

/// 子进程会自己去读这些环境变量：每个后端起进程时都设上，后面再接 [`Config::environment`]
fn terminal_variables(dark_background: bool) -> [(&'static str, &'static str); 4] {
    [
        // 仿真层按 xterm 的口径实现，报这个 shell 才会发 256 色和真彩
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "Nimbus"),
        // 白底黑字是 `0;15`，深底浅字是 `15;0`（`前景;背景` 的色号）。vim、bat 这类程序靠它猜背景深浅，
        // 不给就当深色背景，配出浅色的字落在白底上
        ("COLORFGBG", if dark_background { "15;0" } else { "0;15" }),
    ]
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
    /// 额外的环境变量（shell 集成靠它让 zsh 加载脚本）
    pub environment: Vec<(String, String)>,
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
            environment: Vec::new(),
        }
    }

    /// 用户默认的 shell，加上登录 shell 该有的那几个参数
    pub fn login_shell(columns: u16, rows: u16) -> Self {
        Self::shell(default_shell(), columns, rows)
    }

    /// 跑指定的 shell 程序；认识的 shell 按登录 shell 起
    pub fn shell(program: impl Into<String>, columns: u16, rows: u16) -> Self {
        let program = program.into();
        // zsh、bash 当登录 shell 跑要带 `-l`，不然 `/etc/zprofile`（macOS 上管 PATH 的那个）不执行；
        // PowerShell 不要启动时那几行版权说明
        let arguments = match program_name(&program).as_deref() {
            Some("zsh") | Some("bash") | Some("sh") | Some("fish") => vec!["-l".to_string()],
            Some("pwsh") | Some("powershell") if cfg!(windows) => vec!["-NoLogo".to_string()],
            _ => Vec::new(),
        };
        Self::command(program, arguments, columns, rows)
    }
}

/// 认 shell 用的程序名：去掉目录。Windows 上文件名带 `.exe`、不分大小写，再去掉扩展名、转成小写
/// （`C:\…\PowerShell.EXE` 是 `powershell`）
pub fn program_name(program: &str) -> Option<String> {
    let path = std::path::Path::new(program);
    if cfg!(windows) {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
    } else {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

/// 一种能起的 shell（Windows 上的命令提示符、Windows PowerShell、PowerShell 7）
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Shell {
    /// 不会变的标识：`pwsh`、`powershell`、`cmd`，存进设置里用
    pub id: &'static str,
    /// 给人看的名字
    pub name: &'static str,
    /// 程序的完整路径；没装的是它通常该在的位置
    pub program: String,
    /// 这台机器上有没有
    pub installed: bool,
}

/// 认得的 shell：标识（也就是程序名，见 [`program_name`]）和给人看的名字
const SHELL_NAMES: [(&str, &str); 3] = [
    ("pwsh", "PowerShell 7"),
    ("powershell", "Windows PowerShell"),
    ("cmd", "命令提示符"),
];

/// 这个程序是哪种 shell，给人看的名字（`C:\…\powershell.exe` 是「Windows PowerShell」）。
/// 只认 Windows 上那三种，别的（包括类 Unix 上的 shell）是 `None`
pub fn shell_name(program: &str) -> Option<&'static str> {
    if !cfg!(windows) {
        return None;
    }
    let id = program_name(program)?;
    SHELL_NAMES
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| *name)
}

/// 这台机器上能选的 shell，按推荐的顺序：PowerShell 7、Windows PowerShell、命令提示符。
/// 没装的也列出来（`installed` 是假）。类 Unix 上是空的：那里只用 `$SHELL`（见 [`default_shell`]）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn shells() -> Vec<Shell> {
    #[cfg(windows)]
    return windows_shells();
    #[cfg(not(windows))]
    return Vec::new();
}

#[cfg(windows)]
fn windows_shells() -> Vec<Shell> {
    use std::path::Path;

    let variable = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let system_root =
        variable("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let program_files =
        variable("ProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    // PowerShell 7 不随系统带：先找 PATH（商店版、winget 装的都在上面），再找安装程序默认放的地方
    let pwsh_default = program_files.join(r"PowerShell\7\pwsh.exe");
    let pwsh = std::env::var_os("PATH")
        .and_then(|path| find_in(std::env::split_paths(&path), "pwsh.exe"))
        .or_else(|| exists(&pwsh_default).then(|| pwsh_default.clone()));
    let powershell = system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let cmd =
        variable("ComSpec").map_or_else(|| system_root.join(r"System32\cmd.exe"), PathBuf::from);
    let entry = |id: &'static str, program: &Path, installed| Shell {
        id,
        name: SHELL_NAMES
            .iter()
            .find(|(known, _)| *known == id)
            .map_or(id, |(_, name)| *name),
        program: program.to_string_lossy().into_owned(),
        installed,
    };
    vec![
        entry(
            "pwsh",
            pwsh.as_deref().unwrap_or(&pwsh_default),
            pwsh.is_some(),
        ),
        entry("powershell", &powershell, exists(&powershell)),
        entry("cmd", &cmd, exists(&cmd)),
    ]
}

/// 在这几个目录里找 `name`，返回第一个找到的完整路径
#[cfg(any(windows, test))]
fn find_in(directories: impl IntoIterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    directories
        .into_iter()
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|candidate| exists(candidate))
}

/// 有这个文件。不跟着重解析点走：商店装的程序在 PATH 上是个「应用执行别名」，跟过去会打不开，
/// 但 `CreateProcessW` 起得来它
#[cfg(any(windows, test))]
fn exists(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_dir())
}

/// 新会话的起始目录：用户的主目录。从访达、程序坞、开始菜单启动的应用，当前目录不是主目录，不能拿来当起点；
/// 没有主目录时返回 `None`，由子进程沿用当前目录
fn home_directory() -> Option<PathBuf> {
    crate::store::user_home()
}

/// 这个平台上用户默认的 shell：类 Unix 读 `$SHELL`；Windows 按 [`shells`] 的顺序取第一个装了的
/// （PowerShell 7、Windows PowerShell、命令提示符），一个都找不到就交给系统去找 `cmd.exe`
pub fn default_shell() -> String {
    #[cfg(unix)]
    return std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    #[cfg(windows)]
    return shells()
        .into_iter()
        .find(|shell| shell.installed)
        .map_or_else(|| "cmd.exe".to_string(), |shell| shell.program);
    #[cfg(not(any(unix, windows)))]
    return String::new();
}

/// 起一个 pty，返回句柄和读到的字节
pub fn spawn(config: Config) -> io::Result<(Pty, ByteQueue)> {
    #[cfg(unix)]
    return posix::spawn(config);
    #[cfg(windows)]
    return windows::spawn(config);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = config;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "这个平台没有 pty 后端",
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

    fn arguments(program: &str) -> Vec<String> {
        Config::shell(program, 80, 24).arguments
    }

    /// Windows 上认 shell 比的是去掉 `.exe` 的文件名，不分大小写；PowerShell 不要版权说明
    #[cfg(windows)]
    #[test]
    fn windows_shells_are_recognized_by_their_file_stem() {
        assert_eq!(
            arguments(r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.EXE"),
            ["-NoLogo"]
        );
        assert_eq!(arguments("pwsh.exe"), ["-NoLogo"]);
        assert_eq!(arguments("pwsh"), ["-NoLogo"]);
        assert_eq!(arguments(r"C:\Program Files\Git\bin\bash.exe"), ["-l"]);
        assert!(arguments(r"C:\Windows\System32\cmd.exe").is_empty());
        assert!(arguments(r"C:\tools\powershell-wrapper.exe").is_empty());
    }

    /// 类 Unix 上和原来一样：按文件名原样比，PowerShell 不带参数
    #[cfg(unix)]
    #[test]
    fn unix_shells_keep_their_login_flag() {
        assert_eq!(arguments("/bin/zsh"), ["-l"]);
        assert_eq!(arguments("/usr/local/bin/fish"), ["-l"]);
        assert!(arguments("/usr/local/bin/pwsh").is_empty());
    }

    /// 三种都列出来、顺序固定；装没装和文件在不在对得上；默认的是第一个装了的。
    /// 这台机器上没装 PowerShell 7 时，列的是安装程序默认放的位置
    #[cfg(windows)]
    #[test]
    fn windows_lists_its_shells_and_defaults_to_the_first_installed() {
        let shells = shells();
        let ids: Vec<_> = shells.iter().map(|shell| shell.id).collect();
        assert_eq!(ids, ["pwsh", "powershell", "cmd"]);
        for shell in &shells {
            let program = std::path::Path::new(&shell.program);
            assert!(program.is_absolute(), "{shell:?}");
            assert_eq!(shell.installed, exists(program), "{shell:?}");
        }
        let lowercase = |index: usize| shells[index].program.to_lowercase();
        if !shells[0].installed {
            assert!(
                lowercase(0).ends_with(r"\powershell\7\pwsh.exe"),
                "{shells:?}"
            );
        }
        assert!(shells[1].installed, "Windows 自带 Windows PowerShell");
        assert!(lowercase(1).ends_with(r"\system32\windowspowershell\v1.0\powershell.exe"));
        assert!(shells[2].installed, "Windows 自带命令提示符");
        assert!(lowercase(2).ends_with(r"\cmd.exe"));
        let first = shells
            .iter()
            .find(|shell| shell.installed)
            .expect("至少有一个");
        assert_eq!(default_shell(), first.program);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_list_no_shells() {
        assert!(shells().is_empty());
    }

    /// 在 PATH 里找程序：按顺序取第一个有的；相对路径的目录不算（不然会跑当前目录里的同名程序）；同名的目录不算
    #[test]
    fn the_first_directory_with_the_program_wins() {
        let root = crate::store::tests::scratch("find-in-path");
        let (first, second) = (root.join("first"), root.join("second"));
        std::fs::create_dir_all(first.join("tool.exe")).expect("建得了目录");
        std::fs::create_dir_all(&second).expect("建得了目录");
        std::fs::write(second.join("tool.exe"), b"").expect("写得了");
        let search = |directories: Vec<PathBuf>| find_in(directories, "tool.exe");
        assert_eq!(
            search(vec![PathBuf::from("."), first.clone(), second.clone()]),
            Some(second.join("tool.exe"))
        );
        std::fs::write(root.join("tool.exe"), b"").expect("写得了");
        assert_eq!(
            search(vec![root.clone(), second.clone()]),
            Some(root.join("tool.exe"))
        );
        assert_eq!(search(vec![first.clone()]), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
