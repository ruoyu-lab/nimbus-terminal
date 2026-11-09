//! 连到服务器：用系统的 `ssh` 命令放进 pty 里跑，和本机的 shell 是同一回事。
//!
//! 不自己实现 SSH：认证（密钥、`ssh-agent`、跳板机、`~/.ssh/config`、两步验证、host key 确认）全沿用用户现有的配置，
//! 密码和确认的提示就直接出现在终端里。这里只做三件事：把主机名变成命令行（并且防止它被当成选项）、
//! 从 `~/.ssh/config` 里读出有哪些主机、判断一个主机名能不能用。

use std::path::{Path, PathBuf};

use crate::pty;
use crate::remote::{Ran, run};

/// 主连接在没有任何客户端连着之后多久自己退出（秒）。正常情况由我们在标签关闭时关掉
const CONTROL_PERSIST: u32 = 600;

/// 主机名最长多少：`ssh` 自己的上限是 255
const HOST_MAX: usize = 255;

/// 这个主机名能不能交给 `ssh`。只认字母数字和 `. _ - @ : %`（`user@host`、IPv6、`host%zone`），
/// 不能以 `-` 开头（不然会被当成选项）。空白、引号、分号这些一律不要：它们在任何 shell 里都有特殊意思
pub fn is_valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= HOST_MAX
        && !host.starts_with('-')
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@' | ':' | '%'))
}

/// 用户填的主机名：去掉首尾空白，能用就返回
pub fn normalize(host: &str) -> Option<String> {
    let host = host.trim();
    is_valid_host(host).then(|| host.to_string())
}

/// 连 `host` 要起的命令。`--` 把主机名和选项隔开（主机名已经保证不以 `-` 开头，这是第二道保险）；
/// 保活：每 30 秒问一次，连着 3 次没回应就断开，网络掉了 `ssh` 会自己退出，标签上才看得到「已断开」，
/// 而不是一直卡在那里
///
/// `control` 是连接复用的套接字：终端的 `ssh` 当主连接，监控再起的 `ssh` 借它跑命令，不用再认证一次
pub fn config(
    host: &str,
    columns: u16,
    rows: u16,
    control: Option<&Path>,
    directory: Option<&str>,
) -> pty::Config {
    let mut arguments: Vec<String> = extra_arguments();
    arguments.extend(
        [
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "ServerAliveCountMax=3",
        ]
        .into_iter()
        .map(String::from),
    );
    if let Some(path) = control.and_then(Path::to_str) {
        // 终端的连接当主连接，并且**留在后台**（`ControlPersist`）：终端里 `exit` 之后客户端马上结束、标签才关得掉，
        // 常驻通道还借着连接没用完。实测：不加这个，只要有一个通道还连在主连接上，主连接自己的会话早结束了，
        // `ssh` 进程还要等通道结束才退出，标签就卡住。主连接由我们负责关（标签关了、会话结束了、应用退出时
        // 发 `ssh -O exit`），这个秒数只是应用崩溃时的兜底：没有任何客户端连着之后多久自己退出
        arguments.extend([
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPersist={CONTROL_PERSIST}"),
            "-o".into(),
            format!("ControlPath={path}"),
        ]);
    }
    // 要进远端的某个目录：交给 `ssh` 一条命令（开登录 shell 之前先 cd），这时必须 `-t` 要一个终端
    let command = directory.and_then(crate::files::login_command);
    if command.is_some() {
        arguments.push("-t".to_string());
    }
    arguments.extend(["--".to_string(), host.to_string()]);
    arguments.extend(command);
    pty::Config::command("ssh", arguments, columns, rows)
}

/// 放连接复用套接字的目录：`<base>/nimbus-<uid>`，只有自己能进（权限 0700）。
/// 套接字能让人直接借用已经认证的连接，所以目录必须是自己的、别人进不去：
/// 已经存在的目录要是符号链接、不是自己的、或者别人有权限，就不用它（监控不可用，终端照常能连）
#[cfg(unix)]
pub fn control_directory_in(base: &Path, uid: u32) -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let directory = base.join(format!("nimbus-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    // 不跟着符号链接走：别人可以在 /tmp 里预先放一个同名的链接
    let metadata = std::fs::symlink_metadata(&directory).ok()?;
    (metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0)
        .then_some(directory)
}

/// 当前用户的复用目录（在 `/tmp` 下：套接字路径有长度上限，系统的临时目录太长）
#[cfg(unix)]
pub fn control_directory() -> Option<PathBuf> {
    // SAFETY: `getuid` 没有参数、不会失败，只返回当前进程的用户号
    let uid = unsafe { libc::getuid() };
    control_directory_in(Path::new("/tmp"), uid)
}

#[cfg(not(unix))]
pub fn control_directory() -> Option<PathBuf> {
    None
}

/// 标签 `tab` 的复用套接字路径。带进程号：上次崩溃留下的套接字不会和这次的撞上
pub fn control_path(tab: u64) -> Option<PathBuf> {
    control_directory().map(|directory| directory.join(format!("{}-{tab}", std::process::id())))
}

/// 起主连接之前清掉同名的旧套接字（重连时上一条连接留下的）。目录不可用返回 `false`
pub fn control_ready(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !parent.is_dir() {
        return false;
    }
    let _ = std::fs::remove_file(path);
    true
}

/// 从 `ssh_config` 的文本里读出 `Host` 列出的主机：按出现的先后，去掉重复的，
/// 去掉带通配符（`*` `?`）和取反（`!`）的模式——那些不是一台具体的机器。不展开 `Include`
pub fn parse_hosts(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        // 关键字和值之间是空白，也可以是 `=`
        let Some((keyword, rest)) = line.split_once(|ch: char| ch.is_whitespace() || ch == '=')
        else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for pattern in rest
            .trim_start_matches(|ch: char| ch.is_whitespace() || ch == '=')
            .split_whitespace()
        {
            let pattern = pattern.trim_matches('"');
            if pattern.contains(['*', '?', '!']) || !is_valid_host(pattern) {
                continue;
            }
            if !hosts.iter().any(|known| known == pattern) {
                hosts.push(pattern.to_string());
            }
        }
    }
    hosts
}

/// `~/.ssh/config` 在哪
pub fn config_path() -> Option<PathBuf> {
    crate::store::user_home().map(|home| home.join(".ssh").join("config"))
}

/// 读一个 `ssh_config` 文件里的主机；读不到（没有这个文件、没权限）就是没有
pub fn load_hosts(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path).map_or_else(|_| Vec::new(), |text| parse_hosts(&text))
}

/// 测试里给每个 `ssh` 都加的参数（比如 `-F 配置文件`，指向本机的临时 sshd）；平时没有
#[cfg(test)]
pub static TEST_ARGUMENTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn extra_arguments() -> Vec<String> {
    #[cfg(test)]
    {
        TEST_ARGUMENTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    #[cfg(not(test))]
    {
        Vec::new()
    }
}

/// 每个标签的常驻通道，按复用套接字的路径找
static CHANNELS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<crate::channel::Channel>>>,
> = std::sync::LazyLock::new(Default::default);

fn channels() -> std::sync::MutexGuard<
    'static,
    std::collections::HashMap<PathBuf, std::sync::Arc<crate::channel::Channel>>,
> {
    CHANNELS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 借复用连接开一条通道：`ssh … -- host sh`，远端的 `sh` 一直开着
fn start_channel(
    host: &str,
    control: &Path,
) -> Result<crate::channel::Channel, crate::monitor::Failure> {
    use crate::monitor::Failure;
    let options = Transport::mux_options(control).ok_or(Failure::NotConnected)?;
    let mut command = std::process::Command::new("ssh");
    command
        .args(extra_arguments())
        .args(options)
        .args(["--", host, "sh"]);
    crate::channel::Channel::start(command).map_err(|_| {
        if control.exists() {
            Failure::Failed("起不了通道".into())
        } else {
            Failure::NotConnected
        }
    })
}

/// 在这个连接的常驻通道里跑一段 `sh` 脚本。没有通道就开一条；通道断了（比如主连接重启过）换一条新的再试一次。
/// 超时：通道作废（下一次重开），这次报没有及时回应。套接字不在（终端没连上、断了）是 `NotConnected`
fn run_in_channel(
    host: &str,
    control: &Path,
    script: &str,
    timeout: std::time::Duration,
) -> Result<Ran, crate::monitor::Failure> {
    use crate::channel::Error;
    use crate::monitor::Failure;
    if !is_valid_host(host) {
        return Err(Failure::Failed("主机名不能用".into()));
    }
    if !control.exists() {
        channels().remove(control);
        return Err(Failure::NotConnected);
    }
    for _ in 0..2 {
        let existing = channels().get(control).cloned();
        let channel = match existing {
            Some(channel) => channel,
            None => {
                // 起通道要等网络，不能占着登记表的锁
                let started = std::sync::Arc::new(start_channel(host, control)?);
                channels()
                    .entry(control.to_path_buf())
                    .or_insert(started)
                    .clone()
            }
        };
        match channel.request(script, timeout) {
            Ok(output) => {
                return Ok(Ran {
                    success: output.success,
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            Err(Error::Timeout) => {
                channels().remove(control);
                return Err(Failure::Failed("服务器没有及时回应".into()));
            }
            Err(Error::Dead) => {
                channels().remove(control);
            }
        }
    }
    Err(if control.exists() {
        Failure::Failed("和服务器的通道断了".into())
    } else {
        Failure::NotConnected
    })
}

/// 这个标签不用连接了（标签关了、终端会话结束了、应用要退出了）：收掉通道，再让后台的主连接退出（`ssh -O exit`）。
/// 会等 `ssh -O exit` 做完，很快（毫秒级）；套接字已经不在就什么也不用做
pub fn shutdown(host: &str, control: &Path) {
    channels().remove(control);
    if !control.exists() || !is_valid_host(host) {
        return;
    }
    let Some(path) = control.to_str() else { return };
    let _ = std::process::Command::new("ssh")
        .args(extra_arguments())
        .args([
            "-o",
            &format!("ControlPath={path}"),
            "-O",
            "exit",
            "--",
            host,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 同 [`shutdown`]，但放到后台线程做，不占着界面
pub fn shutdown_in_background(host: &str, control: &Path) {
    let (host, control) = (host.to_string(), control.to_path_buf());
    std::thread::spawn(move || shutdown(&host, &control));
}

/// 借终端的连接在远端跑一次监控脚本，返回它的输出。走常驻通道
pub fn poll(host: &str, control: &Path) -> Result<String, crate::monitor::Failure> {
    use crate::monitor::Failure;
    let ran = run_in_channel(
        host,
        control,
        crate::monitor::MONITOR_SCRIPT,
        std::time::Duration::from_secs(10),
    )?;
    // 脚本最后一条命令失败（没有 /proc/uptime 之类）退出码就不是 0，但前面的分段标记已经打出来了：
    // 有输出就交给解析器判断是不是「不支持」。以前把非 0 一律当成读取失败，非 Linux 的服务器上
    // 监控页写的是 `head: /proc/stat: No such file…`，不是「暂不支持」（用真的 ssh 才发现）
    if ran.success || !ran.stdout.is_empty() {
        Ok(String::from_utf8_lossy(&ran.stdout).into_owned())
    } else {
        let reason = ran
            .stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("命令没有跑成")
            .trim()
            .to_string();
        Err(Failure::Failed(reason.chars().take(120).collect()))
    }
}

/// 通到远端的路：借终端的 SSH 连接。测试里还有一条直接连本机 `sftp-server` 的，
/// 这样 `sftp` 的批处理（引号、通配符、退出码）能用真的 `sftp` 验证，不用一台服务器
pub enum Transport {
    Ssh {
        host: String,
        control: PathBuf,
    },
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))]
    Local,
}

impl Transport {
    /// 借复用连接用的选项：不新开认证（`BatchMode`），也不自己当主连接
    fn mux_options(control: &Path) -> Option<Vec<String>> {
        let path = control.to_str()?;
        Some(
            [
                "-o",
                "ControlMaster=no",
                "-o",
                &format!("ControlPath={path}"),
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
            ]
            .iter()
            .map(|option| option.to_string())
            .collect(),
        )
    }

    /// 连接不在（套接字没有了）就是没连上
    fn check(&self) -> Result<(), crate::monitor::Failure> {
        match self {
            Self::Ssh { host, control } => {
                if !is_valid_host(host) {
                    return Err(crate::monitor::Failure::Failed("主机名不能用".into()));
                }
                if control.exists() {
                    Ok(())
                } else {
                    Err(crate::monitor::Failure::NotConnected)
                }
            }
            #[cfg(test)]
            Self::Local => Ok(()),
        }
    }

    /// 在远端跑一段 `sh` 脚本。远端走常驻通道（脚本写进通道里那个 `sh` 的标准输入，所以路径怎么引只看 `sh` 的规矩，
    /// 不看远端用户的登录 shell）；测试里的本机那条每次起一个 `sh -s`
    fn script(
        &self,
        script: &str,
        timeout: std::time::Duration,
    ) -> Result<Ran, crate::monitor::Failure> {
        self.check()?;
        match self {
            Self::Ssh { host, control } => run_in_channel(host, control, script, timeout),
            #[cfg(test)]
            Self::Local => {
                let mut command = std::process::Command::new("sh");
                command.arg("-s");
                run(command, Some(script.as_bytes().to_vec()), Some(timeout))
            }
        }
    }

    /// 跑一段 `sftp` 批处理。不设超时：大文件要传多久不知道，连接断了 `sftp` 自己会退出
    fn sftp(&self, batch: &str) -> Result<Ran, crate::monitor::Failure> {
        self.check()?;
        let mut command = std::process::Command::new("sftp");
        command.arg("-q");
        match self {
            Self::Ssh { host, control } => {
                let options =
                    Self::mux_options(control).ok_or(crate::monitor::Failure::NotConnected)?;
                command
                    .args(extra_arguments())
                    .args(options)
                    .args(["-b", "-", "--", host]);
            }
            #[cfg(test)]
            Self::Local => {
                command.args(["-b", "-", "-D", "/usr/libexec/sftp-server"]);
            }
        }
        run(command, Some(batch.as_bytes().to_vec()), None)
    }
}

/// `sftp` 出错时给人看的一句话：错误输出里第一条不是命令回显（`sftp> …`）的非空行
fn sftp_reason(ran: &Ran) -> String {
    let stdout = String::from_utf8_lossy(&ran.stdout);
    ran.stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("sftp>"))
        .map_or_else(
            || "传输没有成功".to_string(),
            |line| line.chars().take(160).collect(),
        )
}

/// 列一个目录。`dir` 为 `None` 是远端的主目录
pub fn list_via(
    transport: &Transport,
    dir: Option<&str>,
) -> Result<crate::files::Listing, crate::monitor::Failure> {
    let ran = transport.script(
        &crate::files::list_script(dir),
        std::time::Duration::from_secs(15),
    )?;
    if ran.success {
        crate::files::parse_listing(&ran.stdout)
    } else {
        Err(crate::files::classify_list_error(&ran.stderr))
    }
}

/// 把远端的 `remote`（绝对路径）下载到本地目录 `into` 里，返回存成的路径。
/// 本地已有同名的就加序号，不覆盖；失败时删掉这次留下的半截（名字是这次新取的，不会误删别的）
pub fn download_via(
    transport: &Transport,
    remote: &str,
    directory: bool,
    into: &Path,
) -> Result<PathBuf, crate::monitor::Failure> {
    use crate::monitor::Failure;
    let name = remote.rsplit('/').next().unwrap_or_default();
    if !crate::files::is_safe_name(name) {
        return Err(Failure::Failed("这个文件名不能传（带控制字符）".into()));
    }
    std::fs::create_dir_all(into)
        .map_err(|error| Failure::Failed(format!("建不了下载目录：{error}")))?;
    let destination = crate::files::unique_local(into, name);
    let batch = crate::files::sftp_get(remote, &destination, directory)
        .ok_or_else(|| Failure::Failed("路径不能用".into()))?;
    let ran = transport.sftp(&batch)?;
    if ran.success {
        Ok(destination)
    } else {
        let _ = if directory {
            std::fs::remove_dir_all(&destination)
        } else {
            std::fs::remove_file(&destination)
        };
        Err(Failure::Failed(sftp_reason(&ran)))
    }
}

/// 把本地的文件或目录上传到远端目录 `remote_dir`（绝对路径）里，名字不变
pub fn upload_via(
    transport: &Transport,
    local: &Path,
    remote_dir: &str,
) -> Result<(), crate::monitor::Failure> {
    use crate::monitor::Failure;
    let name = local
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !crate::files::is_safe_name(name) {
        return Err(Failure::Failed("这个文件名不能传（带控制字符）".into()));
    }
    if !local.exists() {
        return Err(Failure::Failed("本地没有这个文件".into()));
    }
    let batch = crate::files::sftp_put(local, remote_dir, local.is_dir())
        .ok_or_else(|| Failure::Failed("路径不能用".into()))?;
    let ran = transport.sftp(&batch)?;
    if ran.success {
        Ok(())
    } else {
        Err(Failure::Failed(sftp_reason(&ran)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_valid() {
        for host in [
            "prod-hk",
            "user@10.0.0.5",
            "a.b.example.com",
            "::1",
            "fe80::1%en0",
            "db_1",
        ] {
            assert!(is_valid_host(host), "{host}");
        }
    }

    /// 以 `-` 开头会被 `ssh` 当成选项（`-oProxyCommand=…` 能执行任意命令）；空白、分号、引号在 shell 里有特殊意思
    #[test]
    fn names_that_could_be_options_or_shell_syntax_are_refused() {
        for host in [
            "",
            "-oProxyCommand=touch /tmp/x",
            "-v",
            "host name",
            "host;rm -rf ~",
            "$(id)",
            "`id`",
            "a'b",
            "a\"b",
            "a\nb",
            "host\tx",
            "主机",
        ] {
            assert!(!is_valid_host(host), "{host:?} 不该被接受");
        }
        assert!(!is_valid_host(&"a".repeat(HOST_MAX + 1)));
    }

    #[test]
    fn normalize_trims_and_validates() {
        assert_eq!(normalize("  prod-hk \n").as_deref(), Some("prod-hk"));
        assert_eq!(normalize("-evil"), None);
        assert_eq!(normalize("   "), None);
    }

    /// 命令行：主机名永远在 `--` 后面，保活选项在前面；从主目录起
    #[test]
    fn the_command_puts_the_host_after_the_separator() {
        let config = config("prod-hk", 120, 36, None, None);
        assert_eq!(config.program, "ssh");
        assert_eq!(
            config.arguments,
            [
                "-o",
                "ServerAliveInterval=30",
                "-o",
                "ServerAliveCountMax=3",
                "--",
                "prod-hk"
            ]
        );
        assert_eq!((config.columns, config.rows), (120, 36));
    }

    /// 要进远端目录：多一个 `-t`（在 `--` 之前）和最后一个命令参数；主机名仍然紧跟在 `--` 后面
    #[test]
    fn a_start_directory_adds_a_tty_flag_and_a_command_after_the_host() {
        let config = config("prod-hk", 80, 24, None, Some("/srv/app"));
        let separator = config
            .arguments
            .iter()
            .position(|argument| argument == "--")
            .expect("有 --");
        assert!(config.arguments[..separator].contains(&"-t".to_string()));
        assert_eq!(config.arguments[separator + 1], "prod-hk");
        assert!(config.arguments[separator + 2].starts_with("sh -c '"));
        assert_eq!(config.arguments.len(), separator + 3);
        // 目录不能用（相对路径）：不加命令，也不加 -t，就是普通连接
        let plain = self::config("prod-hk", 80, 24, None, Some("relative"));
        assert!(
            !plain.arguments.contains(&"-t".to_string())
                && plain.arguments.last().map(String::as_str) == Some("prod-hk")
        );
    }

    /// 带复用套接字：终端当主连接；主机名仍然在 `--` 后面
    #[test]
    fn the_control_socket_options_come_before_the_separator() {
        let config = config(
            "prod-hk",
            80,
            24,
            Some(Path::new("/tmp/nimbus-501/1-2")),
            None,
        );
        let separator = config
            .arguments
            .iter()
            .position(|argument| argument == "--")
            .expect("有 --");
        assert_eq!(config.arguments[separator + 1..], ["prod-hk"]);
        let before = config.arguments[..separator].join(" ");
        assert!(
            before.contains("ControlMaster=auto")
                && before.contains("ControlPath=/tmp/nimbus-501/1-2")
        );
        // 主连接留在后台：终端里 exit 之后客户端马上结束，常驻通道不会把它拖住
        assert!(
            before.contains(&format!("ControlPersist={CONTROL_PERSIST}")),
            "{before}"
        );
    }

    /// 复用目录：自己的、0700 才用；权限松了、是符号链接都不用（别人能借走已经认证的连接）
    #[cfg(unix)]
    #[test]
    fn the_control_directory_must_be_private_and_not_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let base = crate::store::tests::scratch("control-dir");
        std::fs::create_dir_all(&base).expect("建得了目录");
        // SAFETY: `getuid` 没有参数、不会失败
        let uid = unsafe { libc::getuid() };
        let directory = control_directory_in(&base, uid).expect("第一次就能建");
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("在")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(control_directory_in(&base, uid).is_some(), "已经有了也能用");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
            .expect("改得了权限");
        assert!(control_directory_in(&base, uid).is_none(), "别人能进就不用");
        std::fs::remove_dir(&directory).expect("删得了");
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir(&elsewhere).expect("建得了目录");
        symlink(&elsewhere, base.join(format!("nimbus-{uid}"))).expect("建得了链接");
        assert!(control_directory_in(&base, uid).is_none(), "符号链接不用");
        // 目录是自己建的，但用户号对不上（要的是别人的目录）：不是这个用户的就不用
        assert!(control_directory_in(&base, uid.wrapping_add(1)).is_none());
        std::fs::remove_dir_all(base).ok();
    }

    /// 套接字不在就是没连上，不去起任何进程
    #[test]
    fn polling_without_a_socket_means_not_connected() {
        use crate::monitor::Failure;
        assert_eq!(
            poll("prod-hk", Path::new("/nonexistent/nimbus/sock")),
            Err(Failure::NotConnected)
        );
        assert!(matches!(
            poll("-oProxyCommand=x", Path::new("/tmp")),
            Err(Failure::Failed(_))
        ));
    }

    #[test]
    fn hosts_are_read_from_the_config_in_order_without_patterns() {
        let text = "\
# 我的服务器
Host prod-hk
    HostName 1.2.3.4
    User root

host staging  Staging2
  HostName 5.6.7.8

Host *
    ServerAliveInterval 60

Host *.internal !bad web?
Host=eq-sign
Match host foo
Host prod-hk
Host \"quoted\"
Host -oProxyCommand=x
HostName not-a-host
";
        assert_eq!(
            parse_hosts(text),
            ["prod-hk", "staging", "Staging2", "eq-sign", "quoted"]
        );
    }

    #[test]
    fn a_missing_config_file_means_no_hosts() {
        assert!(load_hosts(Path::new("/nonexistent/nowhere/config")).is_empty());
    }
}

#[cfg(all(test, unix))]
mod real {
    use std::time::{Duration, Instant};

    use crate::app::{Launch, spawn};
    use crate::pty::Chunk;
    use crate::target::Target;

    /// 真的起 `ssh`，连一个解析不了的域名（`.invalid` 保证不存在）：进程要能起来、报出原因、以 255 退出，
    /// 这就是界面上「连接已断开，按回车重连」的来源。机器上没有 `ssh` 时跳过
    #[test]
    fn a_real_ssh_to_an_unresolvable_host_fails_with_255() {
        if !std::path::Path::new("/usr/bin/ssh").exists() {
            return;
        }
        let spawned = spawn(&Launch {
            integration: None,
            target: Target::Ssh {
                host: "nimbus-test.invalid".into(),
            },
            columns: 80,
            rows: 24,
            directory: None,
            shell: None,
            dark_background: false,
            control: None,
            remote_directory: None,
        });
        assert!(spawned.error.is_none(), "{:?}", spawned.error);
        let queue = spawned.queue.expect("有读队列");
        let (mut output, mut code) = (Vec::new(), None);
        let deadline = Instant::now() + Duration::from_secs(20);
        while code.is_none() && Instant::now() < deadline {
            match queue.try_next() {
                Some(Chunk::Bytes(bytes)) => output.extend(bytes),
                Some(Chunk::Closed(exit)) => code = Some(exit),
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let text = String::from_utf8_lossy(&output);
        assert_eq!(code, Some(255), "ssh 连不上时退出码是 255：{text}");
        assert!(
            text.contains("nimbus-test.invalid"),
            "输出里要说明连的是谁：{text}"
        );
    }
}

/// 用本机自带的 `sftp-server` 当「远端」，真的跑 `sftp` 批处理：验证引号、通配符、退出码、不覆盖、清理半截
#[cfg(all(test, unix))]
mod sftp {
    use std::path::Path;

    use super::*;
    use crate::monitor::Failure;

    fn available() -> bool {
        Path::new("/usr/libexec/sftp-server").exists() && Path::new("/usr/bin/sftp").exists()
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let directory = crate::store::tests::scratch(label);
        std::fs::create_dir_all(&directory).expect("建得了目录");
        // 路径要绝对、并且不带符号链接（macOS 的 /tmp 是链接）：sftp-server 看到的就是这个路径
        directory.canonicalize().expect("路径在")
    }

    /// 名字里有引号、反斜杠、空格、通配符的文件，原样下载，而且不会顺带把别的文件也下下来
    #[test]
    fn awkward_names_download_exactly() {
        if !available() {
            return;
        }
        let (remote, local) = (scratch("sftp-remote"), scratch("sftp-local"));
        let names = [
            "a \"q\" b.txt",
            "back\\slash.txt",
            "star*.txt",
            "[x].txt",
            "q?.txt",
            "中文 名.txt",
            "-rf",
        ];
        for name in names {
            std::fs::write(remote.join(name), name).expect("写得了");
        }
        // 诱饵：通配符真要展开的话，这些会一起被下载
        for decoy in ["starX.txt", "x.txt", "qZ.txt"] {
            std::fs::write(remote.join(decoy), "decoy").expect("写得了");
        }
        for name in names {
            let path = download_via(
                &Transport::Local,
                remote.join(name).to_str().expect("UTF-8"),
                false,
                &local,
            )
            .unwrap_or_else(|error| panic!("{name}：{error:?}"));
            assert_eq!(
                std::fs::read_to_string(&path).expect("读得了"),
                name,
                "{name} 内容对得上"
            );
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some(name),
                "{name} 存成原来的名字"
            );
        }
        let mut downloaded: Vec<String> = std::fs::read_dir(&local)
            .expect("读得了")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        downloaded.sort();
        let mut expected: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        expected.sort();
        assert_eq!(downloaded, expected, "只有要的几个，诱饵没被带下来");
        std::fs::remove_dir_all(remote).ok();
        std::fs::remove_dir_all(local).ok();
    }

    /// 本地已有同名文件：下到「名字 (1)」，原来的文件不动
    #[test]
    fn a_download_never_overwrites_a_local_file() {
        if !available() {
            return;
        }
        let (remote, local) = (scratch("sftp-remote2"), scratch("sftp-local2"));
        std::fs::write(remote.join("report.txt"), "remote").expect("写得了");
        std::fs::write(local.join("report.txt"), "mine").expect("写得了");
        let path = download_via(
            &Transport::Local,
            remote.join("report.txt").to_str().expect("UTF-8"),
            false,
            &local,
        )
        .expect("下得了");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("report (1).txt")
        );
        assert_eq!(
            std::fs::read_to_string(local.join("report.txt")).expect("读得了"),
            "mine"
        );
        assert_eq!(std::fs::read_to_string(path).expect("读得了"), "remote");
        std::fs::remove_dir_all(remote).ok();
        std::fs::remove_dir_all(local).ok();
    }

    /// 远端没有这个文件：报出原因，本地不留半截
    #[test]
    fn a_missing_remote_file_fails_with_a_reason_and_leaves_nothing() {
        if !available() {
            return;
        }
        let (remote, local) = (scratch("sftp-remote3"), scratch("sftp-local3"));
        let missing = remote.join("nope.txt");
        let Err(Failure::Failed(reason)) = download_via(
            &Transport::Local,
            missing.to_str().expect("UTF-8"),
            false,
            &local,
        ) else {
            panic!("应该失败");
        };
        assert!(
            reason.contains("not found") || reason.contains("nope.txt"),
            "{reason}"
        );
        assert_eq!(std::fs::read_dir(&local).expect("读得了").count(), 0);
        std::fs::remove_dir_all(remote).ok();
        std::fs::remove_dir_all(local).ok();
    }

    /// 目录整个下载、整个上传（`-r`），里面的文件和子目录都在
    #[test]
    fn directories_go_both_ways() {
        if !available() {
            return;
        }
        let (remote, local) = (scratch("sftp-remote4"), scratch("sftp-local4"));
        let tree = remote.join("project");
        std::fs::create_dir_all(tree.join("sub")).expect("建得了目录");
        std::fs::write(tree.join("a.txt"), "a").expect("写得了");
        std::fs::write(tree.join("sub").join("b.txt"), "b").expect("写得了");
        let downloaded = download_via(
            &Transport::Local,
            tree.to_str().expect("UTF-8"),
            true,
            &local,
        )
        .expect("下得了");
        assert_eq!(
            std::fs::read_to_string(downloaded.join("sub").join("b.txt")).expect("读得了"),
            "b"
        );
        let target = scratch("sftp-remote4-up");
        upload_via(
            &Transport::Local,
            &downloaded,
            target.to_str().expect("UTF-8"),
        )
        .expect("传得上");
        assert_eq!(
            std::fs::read_to_string(target.join("project").join("a.txt")).expect("读得了"),
            "a"
        );
        for directory in [remote, local, target] {
            std::fs::remove_dir_all(directory).ok();
        }
    }

    /// 上传带怪名字的文件；带换行的名字拒绝（批处理是一行一条命令）
    #[test]
    fn uploads_handle_odd_names_and_refuse_newlines() {
        if !available() {
            return;
        }
        let (remote, local) = (scratch("sftp-remote5"), scratch("sftp-local5"));
        let odd = local.join("a \"q\" [x]*.txt");
        std::fs::write(&odd, "hi").expect("写得了");
        std::fs::write(local.join("decoy.txt"), "decoy").expect("写得了");
        upload_via(&Transport::Local, &odd, remote.to_str().expect("UTF-8")).expect("传得上");
        assert_eq!(
            std::fs::read_dir(&remote).expect("读得了").count(),
            1,
            "只传了这一个"
        );
        assert_eq!(
            std::fs::read_to_string(remote.join("a \"q\" [x]*.txt")).expect("读得了"),
            "hi"
        );
        let newline = local.join("line\nbreak.txt");
        if std::fs::write(&newline, "x").is_ok() {
            assert!(matches!(
                upload_via(&Transport::Local, &newline, remote.to_str().expect("UTF-8")),
                Err(Failure::Failed(_))
            ));
            assert_eq!(
                std::fs::read_dir(&remote).expect("读得了").count(),
                1,
                "带换行的名字没有传"
            );
        }
        assert!(matches!(
            upload_via(
                &Transport::Local,
                &local.join("missing"),
                remote.to_str().expect("UTF-8")
            ),
            Err(Failure::Failed(_))
        ));
        std::fs::remove_dir_all(remote).ok();
        std::fs::remove_dir_all(local).ok();
    }

    /// 列目录：本机的 `find` 没有 `-printf`（BSD），所以应该明确报「不支持」，而不是乱码或崩溃；
    /// 在 GNU 环境（Linux）上则真的列出来
    #[test]
    fn listing_either_works_on_gnu_find_or_is_reported_unsupported() {
        let directory = scratch("sftp-list");
        std::fs::write(directory.join("a b.txt"), "12345").expect("写得了");
        std::fs::create_dir(directory.join("dir")).expect("建得了目录");
        match list_via(&Transport::Local, Some(directory.to_str().expect("UTF-8"))) {
            Ok(listing) => {
                assert_eq!(listing.path, directory.to_str().expect("UTF-8"));
                let names: Vec<_> = listing
                    .entries
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.size))
                    .collect();
                assert_eq!(names[0].0, "dir");
                assert!(names.contains(&("a b.txt", 5)));
            }
            Err(failure) => assert_eq!(failure, Failure::Unsupported, "没有 GNU find 就该说不支持"),
        }
        // 进不去的目录：报原因
        assert!(matches!(
            list_via(&Transport::Local, Some("/nonexistent/nowhere")),
            Err(Failure::Failed(_)) | Err(Failure::Unsupported)
        ));
        std::fs::remove_dir_all(directory).ok();
    }

    /// 输出很大（一个目录几万个文件）不会卡死：读输出的线程和等进程是分开的
    #[test]
    fn large_output_does_not_deadlock() {
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "yes 0123456789012345678901234567890123456789 | head -c 3000000",
        ]);
        let ran = run(command, None, Some(std::time::Duration::from_secs(20))).expect("跑得完");
        assert!(ran.success);
        assert_eq!(ran.stdout.len(), 3_000_000);
    }

    #[test]
    fn a_command_that_takes_too_long_is_killed() {
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        let started = std::time::Instant::now();
        let result = run(command, None, Some(std::time::Duration::from_millis(200)));
        assert!(matches!(result, Err(Failure::Failed(_))));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}

/// 用真的 `ssh` 连一个本机临时起的 sshd（`NIMBUS_SSHD_LAB=目录`，里面有 `ssh_config`，主机叫 `nimbus-lab`）
/// 端到端验证：主连接留在后台、终端 `exit` 时不被常驻通道拖住、通道的延迟、收尾。
/// 没设环境变量就跳过，所以平时的 `cargo test` 不需要任何服务器
#[cfg(all(test, unix))]
mod lab {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::monitor::Failure;
    use crate::pty::Chunk;

    const HOST: &str = "nimbus-lab";

    /// 设好 `-F 配置文件`；环境变量没设返回 `false`（跳过）
    fn configured() -> bool {
        let Ok(directory) = std::env::var("NIMBUS_SSHD_LAB") else {
            return false;
        };
        let mut arguments = TEST_ARGUMENTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        arguments.clear();
        arguments.extend(["-F".to_string(), format!("{directory}/ssh_config")]);
        true
    }

    /// 每个测试一个自己的套接字路径，互不干扰
    fn control(number: u64) -> PathBuf {
        control_path(9_000_000 + number).expect("复用目录可用")
    }

    /// 起终端那条主连接（和应用里一样的参数），等套接字出现
    fn start_terminal(control: &Path) -> (crate::pty::Pty, crate::pty::ByteQueue) {
        assert!(control_ready(control));
        let (pty, queue) =
            crate::pty::spawn(config(HOST, 120, 30, Some(control), None)).expect("起得来 ssh");
        let deadline = Instant::now() + Duration::from_secs(15);
        while !control.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(control.exists(), "主连接的套接字要出现");
        (pty, queue)
    }

    /// 读到 `Closed` 为止，返回等了多久；超时返回 `None`
    fn wait_closed(queue: &crate::pty::ByteQueue, limit: Duration) -> Option<Duration> {
        let start = Instant::now();
        while start.elapsed() < limit {
            match queue.try_next() {
                Some(Chunk::Closed(_)) => return Some(start.elapsed()),
                Some(Chunk::Bytes(_)) => {}
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        None
    }

    fn masters_left(control: &Path) -> usize {
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "ps -axo command | grep -F '{}' | grep -v grep | wc -l",
                    control.display()
                ),
            ])
            .output()
            .expect("跑得了");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    /// 最关键的一条：常驻通道开着的时候，终端里 `exit`，终端的 `ssh` 要马上结束（标签才关得掉）。
    /// 以前（主连接是前台的）这时 `ssh` 会一直挂着等通道结束
    #[test]
    fn exiting_the_terminal_is_not_held_up_by_the_open_channel() {
        if !configured() {
            return;
        }
        let control = control(1);
        let (pty, queue) = start_terminal(&control);
        // 通道开着、用着
        let ran = run_in_channel(HOST, &control, "echo from-channel", Duration::from_secs(10))
            .expect("通道能用");
        assert_eq!(ran.stdout, b"from-channel\n");
        pty.write(b"exit\r").expect("写得进");
        let waited = wait_closed(&queue, Duration::from_secs(10))
            .expect("终端的 ssh 要结束，不能被通道拖住");
        assert!(waited < Duration::from_secs(8), "等了 {waited:?}");
        // 通道所在的主连接还在后台，我们收尾
        shutdown(HOST, &control);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!control.exists(), "收尾之后套接字没了");
        assert_eq!(masters_left(&control), 0, "后台没有残留的主连接");
        assert_eq!(
            run_in_channel(HOST, &control, "echo x", Duration::from_secs(5)),
            Err(Failure::NotConnected)
        );
    }

    /// 通道的延迟：连续很多个请求。对比「每次起一个 ssh」。只打印，不断言具体毫秒（机器不同）；
    /// 断言的是通道不比每次起进程慢
    #[test]
    fn the_channel_is_faster_than_spawning_ssh_each_time() {
        if !configured() {
            return;
        }
        let control = control(2);
        let (_pty, _queue) = start_terminal(&control);
        run_in_channel(HOST, &control, "true", Duration::from_secs(10)).expect("先把通道开起来");
        let rounds = 30;
        let start = Instant::now();
        for _ in 0..rounds {
            assert!(
                run_in_channel(HOST, &control, "echo hi", Duration::from_secs(10))
                    .expect("通道能用")
                    .success
            );
        }
        let channel = start.elapsed() / rounds;
        let start = Instant::now();
        for _ in 0..rounds {
            let mut command = std::process::Command::new("ssh");
            command
                .args(extra_arguments())
                .args(Transport::mux_options(&control).expect("路径是 UTF-8"))
                .args(["--", HOST, "echo hi"]);
            assert!(
                run(command, None, Some(Duration::from_secs(10)))
                    .expect("跑得了")
                    .success
            );
        }
        let spawned = start.elapsed() / rounds;
        println!("LAB 通道每次 {channel:?}，每次起 ssh 进程 {spawned:?}");
        assert!(
            channel <= spawned,
            "通道 {channel:?} 不该比起进程 {spawned:?} 慢"
        );
        shutdown(HOST, &control);
    }

    /// 通道没了（连接断了）：报「未连接」；终端重新连上之后通道自己重开，不用任何手动处理
    #[test]
    fn the_channel_recovers_after_the_connection_comes_back() {
        if !configured() {
            return;
        }
        let control = control(3);
        let (pty, queue) = start_terminal(&control);
        assert!(
            run_in_channel(HOST, &control, "echo one", Duration::from_secs(10))
                .expect("通道能用")
                .success
        );
        pty.write(b"exit\r").expect("写得进");
        wait_closed(&queue, Duration::from_secs(10)).expect("终端结束");
        shutdown(HOST, &control);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            run_in_channel(HOST, &control, "echo two", Duration::from_secs(5)),
            Err(Failure::NotConnected)
        );
        // 重连：新的主连接，同一个套接字路径
        let (pty, queue) = start_terminal(&control);
        let ran = run_in_channel(HOST, &control, "echo three", Duration::from_secs(10))
            .expect("重连之后通道自己重开");
        assert_eq!(ran.stdout, b"three\n");
        pty.write(b"exit\r").expect("写得进");
        wait_closed(&queue, Duration::from_secs(10)).expect("终端结束");
        shutdown(HOST, &control);
    }

    /// 借真的 ssh 连接：监控的脚本在 macOS 服务器上没有 /proc，要明确说「不支持」；
    /// 列目录脚本在没有 GNU find 的服务器上同样；错误输出和退出码走通道也对
    #[test]
    fn real_scripts_report_unsupported_on_a_non_linux_server() {
        if !configured() {
            return;
        }
        let control = control(4);
        let (pty, queue) = start_terminal(&control);
        let polled = poll(HOST, &control).expect("脚本本身能跑");
        assert_eq!(crate::monitor::parse(&polled), Err(Failure::Unsupported));
        let transport = Transport::Ssh {
            host: HOST.to_string(),
            control: control.clone(),
        };
        let listed = list_via(&transport, Some("/tmp"));
        assert!(
            matches!(listed, Err(Failure::Unsupported) | Ok(_)),
            "{listed:?}"
        );
        let failure = run_in_channel(
            HOST,
            &control,
            "echo oops >&2; exit 4",
            Duration::from_secs(10),
        )
        .expect("通道能用");
        assert!(!failure.success);
        assert_eq!(failure.stderr, "oops\n");
        pty.write(b"exit\r").expect("写得进");
        wait_closed(&queue, Duration::from_secs(10)).expect("终端结束");
        shutdown(HOST, &control);
    }

    /// sftp 也借同一条连接：下载、上传、怪名字，都用真的 ssh（不是直连 sftp-server）
    #[test]
    fn transfers_work_over_the_real_multiplexed_connection() {
        if !configured() {
            return;
        }
        let control = control(5);
        let (pty, queue) = start_terminal(&control);
        let remote_dir = crate::store::tests::scratch("lab-remote");
        let local_dir = crate::store::tests::scratch("lab-local");
        std::fs::create_dir_all(&remote_dir).expect("建得了目录");
        let remote_dir = remote_dir.canonicalize().expect("路径在");
        let name = "a \"q\" [x]*.txt";
        std::fs::write(remote_dir.join(name), "payload").expect("写得了");
        let transport = Transport::Ssh {
            host: HOST.to_string(),
            control: control.clone(),
        };
        let saved = download_via(
            &transport,
            remote_dir.join(name).to_str().expect("UTF-8"),
            false,
            &local_dir,
        )
        .expect("下得了");
        assert_eq!(std::fs::read_to_string(&saved).expect("读得了"), "payload");
        std::fs::write(local_dir.join("up.txt"), "uploaded").expect("写得了");
        upload_via(
            &transport,
            &local_dir.join("up.txt"),
            remote_dir.to_str().expect("UTF-8"),
        )
        .expect("传得上");
        assert_eq!(
            std::fs::read_to_string(remote_dir.join("up.txt")).expect("读得了"),
            "uploaded"
        );
        pty.write(b"exit\r").expect("写得进");
        wait_closed(&queue, Duration::from_secs(10)).expect("终端结束");
        shutdown(HOST, &control);
        std::fs::remove_dir_all(remote_dir).ok();
        std::fs::remove_dir_all(local_dir).ok();
    }
}
