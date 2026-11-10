//! WSL（Windows 上的 Linux 子系统）：列出装了的发行版，起它们的 shell，在 Windows 和 Linux 两种路径写法之间换。
//!
//! 发行版的 shell 在标签里记成 `wsl:发行版`（[`program`]），和别的 shell 的程序路径放在同一处：拆分、存盘、恢复都跟着标签走。
//! 起的时候是 `wsl.exe -d 发行版 --exec /bin/sh -c 引导`（[`config`]）：引导脚本进目录、找到用户的登录 shell，
//! 开着 shell 集成时让 bash、zsh 加载脚本。
//!
//! Windows 这边看不到 WSL 里的进程：当前目录靠 shell 集成报（OSC 7），「有程序在跑」靠命令标记。
//! WSL 里的目录在 Windows 这边写成 `\\wsl.localhost\发行版\…`、`/mnt/c/…` 写回 `C:\…`（[`to_windows`]），
//! 拖进来的 Windows 路径反过来（[`to_linux`]）

use std::path::{Path, PathBuf};

use crate::pty;

/// 标签里记发行版的 shell 用的前缀：`wsl:Ubuntu`
const PREFIX: &str = "wsl:";

/// 引导脚本，`wsl.exe --exec /bin/sh -c` 跑它，`$1` 是要进的目录（WSL 的写法，空的是主目录）。
///
/// 先回主目录再进 `$1`：进不去（目录没了）就报一句、留在主目录，不让会话一闪而过（`wsl.exe --cd` 进不去会直接退出）。
/// 用户的登录 shell 按 `/etc/passwd` 找（`--exec` 不经过登录 shell），照登录 shell 起，和 `wsl.exe` 默认的一样。
/// 有集成脚本时（`NIMBUS_SHELL_DIR`，经 `WSLENV` 换成了 WSL 里的路径）：bash 用 `--init-file` 加载
/// （脚本自己先把登录 shell 该跑的配置跑一遍），zsh 用和 macOS 上一样的 `ZDOTDIR` 垫片；别的 shell 照常起
const BOOTSTRAP: &str = r#"cd ~ 2>/dev/null
if [ -n "$1" ] && ! cd -- "$1" 2>/dev/null; then
  echo 'nimbus: 进不了这个目录' >&2
fi
s=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
[ -x "$s" ] || s=${SHELL:-/bin/sh}
i=$NIMBUS_SHELL_DIR
unset NIMBUS_SHELL_DIR
case ${s##*/} in
bash)
  if [ -n "$i" ] && [ -r "$i/nimbus.bash" ]; then
    export NIMBUS_BASH_LOGIN=1
    exec "$s" --init-file "$i/nimbus.bash"
  fi ;;
zsh)
  if [ -n "$i" ] && [ -r "$i/zdotdir/.zshenv" ]; then
    if [ -n "${ZDOTDIR+x}" ]; then export NIMBUS_ORIG_ZDOTDIR="$ZDOTDIR"; fi
    export ZDOTDIR="$i/zdotdir" NIMBUS_ZSH_INTEGRATION="$i/nimbus.zsh"
    exec "$s" -l
  fi ;;
esac
exec "$s" -l"#;

/// 一个装了的发行版
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub name: String,
    /// 还没走完第一次启动的设置（建 Linux 用户）
    pub needs_setup: bool,
    /// 默认用哪个用户登录（用户号）
    pub uid: u32,
}

/// 发行版 `name` 的 shell 在标签里记成什么
pub fn program(name: &str) -> String {
    format!("{PREFIX}{name}")
}

/// `program` 是不是某个发行版的 shell（[`program`] 写的），是的话是哪个。只在 Windows 上认
pub fn distribution(program: &str) -> Option<&str> {
    if !cfg!(windows) {
        return None;
    }
    program
        .strip_prefix(PREFIX)
        .filter(|name| is_valid_name(name))
}

/// 发行版的名字只许字母、数字和 `. _ -`（WSL 自己也这么限制）：拼进 `\\wsl.localhost\名字` 和命令行都不会出岔子
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// 这台机器上装了的发行版：默认的那个在最前，别的按名字排。Docker Desktop、Rancher Desktop 自己用的
/// （`docker-desktop`、`rancher-desktop-data`……）不列，里面不是给人用的 shell。
/// 读的是注册表，不去问 `wsl.exe`：那样会把 WSL 的虚拟机拉起来。别的平台是空的
pub fn distributions() -> Vec<Distribution> {
    #[cfg(windows)]
    return registry::distributions();
    #[cfg(not(windows))]
    return Vec::new();
}

/// 不列的发行版：别的软件自己用的
fn is_internal(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("docker-desktop") || name.starts_with("rancher-desktop")
}

/// 起发行版 `name` 的 shell：`wsl.exe -d 发行版 --exec /bin/sh -c 引导 nimbus 目录`（见 [`BOOTSTRAP`]）。
///
/// `directory` 是 Windows 的写法，[`to_linux`] 换得过去的才用，别的从主目录起。`integration` 是 shell 集成脚本的目录
/// （[`crate::shell::install`]），给了就让 bash、zsh 加载它。还没建用户的发行版不带引导：`wsl.exe` 要先问用户名和密码，
/// `--exec` 会跳过这一步、直接用 root 起
pub fn config(
    name: &str,
    directory: Option<&Path>,
    integration: Option<&Path>,
    columns: u16,
    rows: u16,
) -> pty::Config {
    let needs_setup = distributions()
        .iter()
        .any(|distribution| distribution.name == name && distribution.needs_setup);
    build(name, needs_setup, directory, integration, columns, rows)
}

/// 测试用：不管发行版建没建过用户，都按建过的起（测试自己再加 `-u root`）
#[cfg(test)]
pub(crate) fn config_for_test(
    name: &str,
    directory: Option<&Path>,
    integration: Option<&Path>,
) -> pty::Config {
    build(name, false, directory, integration, 120, 24)
}

fn build(
    name: &str,
    needs_setup: bool,
    directory: Option<&Path>,
    integration: Option<&Path>,
    columns: u16,
    rows: u16,
) -> pty::Config {
    let directory = directory
        .and_then(Path::to_str)
        .and_then(|directory| to_linux(name, directory));
    let integration = integration.map(|path| path.to_string_lossy().into_owned());
    let mut config = pty::Config::command(
        wsl_program(),
        arguments(name, needs_setup, directory.as_deref()),
        columns,
        rows,
    );
    config.environment = environment(
        std::env::var("WSLENV").ok().as_deref(),
        integration.filter(|_| !needs_setup),
    );
    config
}

/// `wsl.exe` 的完整路径：不按 PATH 找，免得跑到别的同名程序
fn wsl_program() -> String {
    let root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join(r"System32\wsl.exe")
        .to_string_lossy()
        .into_owned()
}

/// 传给 `wsl.exe` 的参数（见 [`config`]）
fn arguments(name: &str, needs_setup: bool, directory: Option<&str>) -> Vec<String> {
    let mut arguments = vec!["-d".to_string(), name.to_string()];
    if needs_setup {
        arguments.extend(["--cd", "~"].map(String::from));
    } else {
        arguments.extend(
            [
                "--exec",
                "/bin/sh",
                "-c",
                BOOTSTRAP,
                "nimbus",
                directory.unwrap_or(""),
            ]
            .map(String::from),
        );
    }
    arguments
}

/// 要额外给 `wsl.exe` 的环境变量。WSL 里看不到 Windows 的环境变量，要传的得列在 `WSLENV` 里：
/// 终端的那几个（`TERM` WSL 自己会设）原样传；集成脚本的目录（`integration`）按路径传，WSL 会换成 `/mnt/c/…`。
/// `existing` 是我们自己的 `WSLENV`（用户设过的），接在它后面
fn environment(existing: Option<&str>, integration: Option<String>) -> Vec<(String, String)> {
    let mut passed: Vec<&str> = existing
        .into_iter()
        .flat_map(|existing| existing.split(':'))
        .filter(|entry| !entry.is_empty())
        .collect();
    passed.extend(["COLORTERM/u", "TERM_PROGRAM/u", "COLORFGBG/u"]);
    let mut environment = Vec::new();
    if let Some(integration) = integration {
        passed.push("NIMBUS_SHELL_DIR/pu");
        environment.push(("NIMBUS_SHELL_DIR".to_string(), integration));
    }
    environment.push(("WSLENV".to_string(), passed.join(":")));
    environment
}

/// WSL 里的绝对路径换成 Windows 的写法：`/mnt/c/x` 是 `C:\x`（WSL 默认把 Windows 的盘挂在那里），
/// 别的是 `\\wsl.localhost\发行版\x`。不是绝对路径、带控制字符的是 `None`
pub fn to_windows(name: &str, path: &str) -> Option<PathBuf> {
    if !path.starts_with('/') || path.chars().any(char::is_control) || !is_valid_name(name) {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if let ["mnt", drive, rest @ ..] = parts.as_slice()
        && drive.len() == 1
        && drive.chars().all(|ch| ch.is_ascii_alphabetic())
    {
        return Some(PathBuf::from(format!(
            r"{}:\{}",
            drive.to_ascii_uppercase(),
            rest.join(r"\")
        )));
    }
    Some(PathBuf::from(format!(
        r"\\wsl.localhost\{name}\{}",
        parts.join(r"\")
    )))
}

/// Windows 的路径换成 WSL 里的写法（拖文件进 WSL 的终端、从 Windows 的目录起 WSL 的 shell）：`C:\x` 是 `/mnt/c/x`，
/// 这个发行版自己的 `\\wsl.localhost\发行版\x`（老的写法 `\\wsl$\发行版\x` 也认）是 `/x`。
/// 别的（共享路径、别的发行版、相对路径、带控制字符的）是 `None`
pub fn to_linux(name: &str, path: &str) -> Option<String> {
    if path.chars().any(char::is_control) {
        return None;
    }
    let path = path.replace('/', r"\");
    let parts = |rest: &str| -> String {
        let parts: Vec<&str> = rest.split('\\').filter(|part| !part.is_empty()).collect();
        parts.join("/")
    };
    let bytes = path.as_bytes();
    if bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'\\')
    {
        let drive = char::from(bytes[0]).to_ascii_lowercase();
        let rest = parts(&path[2..]);
        return Some(if rest.is_empty() {
            format!("/mnt/{drive}")
        } else {
            format!("/mnt/{drive}/{rest}")
        });
    }
    let lower = path.to_ascii_lowercase();
    let host = [r"\\wsl.localhost\", r"\\wsl$\"]
        .into_iter()
        .find(|host| lower.starts_with(host))?;
    let rest = &path[host.len()..];
    let (distribution, rest) = rest.split_once('\\').unwrap_or((rest, ""));
    if !distribution.eq_ignore_ascii_case(name) {
        return None;
    }
    Some(format!("/{}", parts(rest)))
}

/// 发行版里默认用户的主目录（Windows 的写法）：`~/…` 的链接靠它。按注册表里的默认用户号在 `/etc/passwd` 里找；
/// 读的是 `\\wsl.localhost\发行版\etc\passwd`，发行版没在跑时会把它拉起来，所以只在问到 `~/…` 时才读
pub fn home(name: &str) -> Option<PathBuf> {
    let uid = distributions()
        .into_iter()
        .find(|distribution| distribution.name == name)?
        .uid;
    let passwd = std::fs::read_to_string(to_windows(name, "/etc/passwd")?).ok()?;
    to_windows(name, &passwd_home(&passwd, uid)?)
}

/// `/etc/passwd` 里用户号是 `uid` 的那个用户的主目录
fn passwd_home(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        let matches = fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid);
        matches.then(|| fields[5].to_string())
    })
}

/// 读 WSL 记在注册表里的发行版：`HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss` 下每个发行版一个子键
/// （名字是编号），里面有名字（`DistributionName`）、状态（`State`，1 是装好了）、要不要走第一次的设置（`RunOOBE`）、
/// 默认用户号（`DefaultUid`）；`Lxss` 本身的 `DefaultDistribution` 是默认那个的编号
#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
        RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    };

    use super::{Distribution, is_internal, is_valid_name};

    const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

    /// 开着的注册表键，丢掉时关上
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: 是 `RegOpenKeyExW` 打开的、归我们的键，只关这一次
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn open(parent: HKEY, path: &str) -> Option<Key> {
        let path = wide(path);
        let mut key = std::ptr::null_mut();
        // SAFETY: 路径以 0 结尾；打开成功时 `key` 是新开的键
        let status = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then(|| Key(key))
    }

    /// 键下的一个字符串值
    fn string(key: &Key, name: &str) -> Option<String> {
        let name = wide(name);
        let mut size = 0u32;
        // SAFETY: 不给缓冲，只问要多大（字节数）
        let status = unsafe {
            RegGetValueW(
                key.0,
                std::ptr::null(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: 缓冲有 `size` 个字节；值在两次调用之间变长了会返回错误，当没有
        let status = unsafe {
            RegGetValueW(
                key.0,
                std::ptr::null(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let end = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16(&buffer[..end]).ok()
    }

    /// 键下的一个 32 位整数值
    fn number(key: &Key, name: &str) -> Option<u32> {
        let name = wide(name);
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: 缓冲是一个 u32，大小如实给
        let status = unsafe {
            RegGetValueW(
                key.0,
                std::ptr::null(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut value as *mut u32).cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS).then_some(value)
    }

    /// 键下所有子键的名字
    fn subkeys(key: &Key) -> Vec<String> {
        let mut names = Vec::new();
        for index in 0u32.. {
            // 键名最长 255 个字
            let mut buffer = [0u16; 256];
            let mut length = buffer.len() as u32;
            // SAFETY: 缓冲的长度如实给；不要类名、修改时间
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        names
    }

    pub(super) fn distributions() -> Vec<Distribution> {
        let Some(lxss) = open(HKEY_CURRENT_USER, LXSS) else {
            return Vec::new();
        };
        let default = string(&lxss, "DefaultDistribution");
        let mut found: Vec<(bool, Distribution)> = subkeys(&lxss)
            .into_iter()
            .filter_map(|id| {
                let key = open(lxss.0, &id)?;
                let name = string(&key, "DistributionName")?;
                let installed = number(&key, "State").is_none_or(|state| state == 1);
                (installed && is_valid_name(&name) && !is_internal(&name)).then(|| {
                    let distribution = Distribution {
                        name,
                        needs_setup: number(&key, "RunOOBE") == Some(1),
                        uid: number(&key, "DefaultUid").unwrap_or(0),
                    };
                    (default.as_deref() == Some(id.as_str()), distribution)
                })
            })
            .collect();
        found.sort_by(|(a_default, a), (b_default, b)| {
            b_default
                .cmp(a_default)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        found
            .into_iter()
            .map(|(_, distribution)| distribution)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_before_they_reach_a_command_line() {
        assert!(is_valid_name("Ubuntu-24.04"));
        assert!(is_valid_name("my_distro"));
        for bad in ["", "a b", r"a\b", "a/b", "a;b", "中文", "a\"b"] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
        assert!(is_internal("docker-desktop"));
        assert!(is_internal("docker-desktop-data"));
        assert!(is_internal("Rancher-Desktop"));
        assert!(!is_internal("Ubuntu"));
    }

    /// 标签里记的是 `wsl:发行版`；只在 Windows 上认，名字不对的不认
    #[test]
    fn programs_name_their_distribution() {
        let program = program("Ubuntu-24.04");
        assert_eq!(program, "wsl:Ubuntu-24.04");
        if cfg!(windows) {
            assert_eq!(distribution(&program), Some("Ubuntu-24.04"));
            assert_eq!(distribution("wsl:a b"), None);
            assert_eq!(distribution("wsl:"), None);
        } else {
            assert_eq!(distribution(&program), None);
        }
        assert_eq!(distribution(r"C:\Windows\System32\wsl.exe"), None);
        assert_eq!(distribution("cmd.exe"), None);
    }

    /// WSL 里的路径 → Windows：挂上来的盘写回盘符，别的放到 `\\wsl.localhost\发行版` 下面
    #[test]
    fn linux_paths_map_to_windows_ones() {
        let windows =
            |path: &str| to_windows("Ubuntu", path).map(|path| path.to_string_lossy().into_owned());
        assert_eq!(
            windows("/home/me/中 x").as_deref(),
            Some(r"\\wsl.localhost\Ubuntu\home\me\中 x")
        );
        assert_eq!(windows("/").as_deref(), Some(r"\\wsl.localhost\Ubuntu\"));
        assert_eq!(
            windows("/home//me/").as_deref(),
            Some(r"\\wsl.localhost\Ubuntu\home\me")
        );
        assert_eq!(windows("/mnt/c/Users/me").as_deref(), Some(r"C:\Users\me"));
        assert_eq!(windows("/mnt/d").as_deref(), Some(r"D:\"));
        assert_eq!(
            windows("/mnt/wsl/x").as_deref(),
            Some(r"\\wsl.localhost\Ubuntu\mnt\wsl\x")
        );
        assert_eq!(windows("home/me"), None, "不是绝对路径");
        assert_eq!(windows("/a\nb"), None);
        assert_eq!(to_windows("a b", "/home"), None, "发行版的名字不对");
    }

    /// Windows 的路径 → WSL：盘符换成 `/mnt/盘`，这个发行版自己的共享路径去掉前缀（新老两种写法、斜杠反斜杠都认）
    #[test]
    fn windows_paths_map_to_linux_ones() {
        let linux = |path: &str| to_linux("Ubuntu", path);
        assert_eq!(
            linux(r"C:\Users\me\a b.txt").as_deref(),
            Some("/mnt/c/Users/me/a b.txt")
        );
        assert_eq!(linux("D:/data/").as_deref(), Some("/mnt/d/data"));
        assert_eq!(linux(r"C:\").as_deref(), Some("/mnt/c"));
        assert_eq!(
            linux(r"\\wsl.localhost\Ubuntu\home\me").as_deref(),
            Some("/home/me")
        );
        assert_eq!(
            linux(r"\\WSL$\ubuntu\etc\hosts").as_deref(),
            Some("/etc/hosts")
        );
        assert_eq!(linux(r"\\wsl.localhost\Ubuntu").as_deref(), Some("/"));
        assert_eq!(linux("//wsl.localhost/Ubuntu/tmp").as_deref(), Some("/tmp"));
        assert_eq!(linux(r"\\wsl.localhost\Debian\home"), None, "别的发行版");
        assert_eq!(linux(r"\\server\share\x"), None);
        assert_eq!(linux(r"relative\x"), None);
        assert_eq!(linux(r"C:x"), None, "盘符后面不是分隔符");
        assert_eq!(linux("C:\\a\nb"), None);
    }

    #[test]
    fn the_home_directory_comes_from_passwd() {
        let passwd =
            "root:x:0:0:root:/root:/bin/bash\nbroken\nme:x:1000:1000:Me,,,:/home/me:/usr/bin/zsh\n";
        assert_eq!(passwd_home(passwd, 1000).as_deref(), Some("/home/me"));
        assert_eq!(passwd_home(passwd, 0).as_deref(), Some("/root"));
        assert_eq!(passwd_home(passwd, 42), None);
    }

    /// 起的是 `wsl.exe -d 发行版 --exec /bin/sh -c 引导 nimbus 目录`；还没建用户的发行版只带 `--cd ~`，让 WSL 先问用户名
    #[test]
    fn the_command_line_runs_the_bootstrap_unless_setup_is_pending() {
        let started = arguments("Ubuntu", false, Some("/home/me"));
        assert_eq!(started[..5], ["-d", "Ubuntu", "--exec", "/bin/sh", "-c"]);
        assert_eq!(started[5], BOOTSTRAP);
        assert_eq!(started[6..], ["nimbus", "/home/me"]);
        assert_eq!(arguments("Ubuntu", false, None)[7], "", "没有目录：主目录");
        assert_eq!(
            arguments("Ubuntu", true, Some("/home/me")),
            ["-d", "Ubuntu", "--cd", "~"]
        );
        assert!(!BOOTSTRAP.contains('\r'), "在 Linux 里跑，不能带回车");
    }

    /// `WSLENV` 接在用户自己设的后面；集成脚本的目录按路径传
    #[test]
    fn the_environment_lists_what_wsl_should_pass_through() {
        let without = environment(None, None);
        assert_eq!(
            without,
            [(
                "WSLENV".to_string(),
                "COLORTERM/u:TERM_PROGRAM/u:COLORFGBG/u".to_string()
            )]
        );
        let with = environment(Some("GOPATH/l:"), Some(r"C:\x\shell".to_string()));
        assert_eq!(
            with,
            [
                ("NIMBUS_SHELL_DIR".to_string(), r"C:\x\shell".to_string()),
                (
                    "WSLENV".to_string(),
                    "GOPATH/l:COLORTERM/u:TERM_PROGRAM/u:COLORFGBG/u:NIMBUS_SHELL_DIR/pu"
                        .to_string()
                ),
            ]
        );
    }

    /// 注册表里读出来的：名字都合规、不含别的软件自用的，默认的在最前（有几个就几个，没装 WSL 是空的）
    #[cfg(windows)]
    #[test]
    fn installed_distributions_are_listed_from_the_registry() {
        let found = distributions();
        eprintln!("这台机器上的发行版：{found:?}");
        for distribution in &found {
            assert!(is_valid_name(&distribution.name), "{distribution:?}");
            assert!(!is_internal(&distribution.name), "{distribution:?}");
        }
    }
}
