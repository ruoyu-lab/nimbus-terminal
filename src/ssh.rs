//! 连到服务器：用系统的 `ssh` 命令放进 pty 里跑，和本机的 shell 是同一回事。
//!
//! 不自己实现 SSH：认证（密钥、`ssh-agent`、跳板机、`~/.ssh/config`、两步验证、host key 确认）全沿用用户现有的配置，
//! 密码和确认的提示就直接出现在终端里。这里只做三件事：把主机名变成命令行（并且防止它被当成选项）、
//! 从 `~/.ssh/config` 里读出有哪些主机、判断一个主机名能不能用。

use std::path::{Path, PathBuf};

use crate::pty;

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
pub fn config(host: &str, columns: u16, rows: u16, control: Option<&Path>) -> pty::Config {
    let mut arguments: Vec<String> =
        ["-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3"].into_iter().map(String::from).collect();
    if let Some(path) = control.and_then(Path::to_str) {
        arguments.extend(["-o".into(), "ControlMaster=yes".into(), "-o".into(), format!("ControlPath={path}")]);
    }
    arguments.extend(["--".to_string(), host.to_string()]);
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
    (metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0).then_some(directory)
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
    let Some(parent) = path.parent() else { return false };
    if !parent.is_dir() {
        return false;
    }
    let _ = std::fs::remove_file(path);
    true
}

/// 借终端的连接在远端跑一次监控脚本，返回它的输出。套接字不在（终端还没连上、断了）是 `NotConnected`；
/// 不新开认证：`BatchMode` 下要认证就直接失败，绝不会弹提示或者悄悄另起一条连接去试
pub fn poll(host: &str, control: &Path) -> Result<String, crate::monitor::Failure> {
    use crate::monitor::Failure;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(10);
    if !is_valid_host(host) {
        return Err(Failure::Failed("主机名不能用".into()));
    }
    if !control.exists() {
        return Err(Failure::NotConnected);
    }
    let Some(path) = control.to_str() else { return Err(Failure::NotConnected) };
    let mut child = Command::new("ssh")
        .args(["-o", "ControlMaster=no", "-o"])
        .arg(format!("ControlPath={path}"))
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "--", host, crate::monitor::REMOTE_COMMAND])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failure::Failed(format!("起不了 ssh：{error}")))?;
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Failed("服务器没有及时回应".into()));
            }
            Err(error) => return Err(Failure::Failed(error.to_string())),
        }
    };
    let (mut output, mut errors) = (String::new(), String::new());
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut output);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut errors);
    }
    if status.success() {
        Ok(output)
    } else if !control.exists() {
        // 跑的过程中终端断了，套接字没了
        Err(Failure::NotConnected)
    } else {
        let reason = errors.lines().find(|line| !line.trim().is_empty()).unwrap_or("命令没有跑成").trim();
        Err(Failure::Failed(reason.chars().take(120).collect()))
    }
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
        for pattern in rest.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '=').split_whitespace() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_valid() {
        for host in ["prod-hk", "user@10.0.0.5", "a.b.example.com", "::1", "fe80::1%en0", "db_1"] {
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
        let config = config("prod-hk", 120, 36, None);
        assert_eq!(config.program, "ssh");
        assert_eq!(
            config.arguments,
            ["-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3", "--", "prod-hk"]
        );
        assert_eq!((config.columns, config.rows), (120, 36));
    }

    /// 带复用套接字：终端当主连接；主机名仍然在 `--` 后面
    #[test]
    fn the_control_socket_options_come_before_the_separator() {
        let config = config("prod-hk", 80, 24, Some(Path::new("/tmp/nimbus-501/1-2")));
        let separator = config.arguments.iter().position(|argument| argument == "--").expect("有 --");
        assert_eq!(config.arguments[separator + 1..], ["prod-hk"]);
        let before = config.arguments[..separator].join(" ");
        assert!(before.contains("ControlMaster=yes") && before.contains("ControlPath=/tmp/nimbus-501/1-2"));
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
        assert_eq!(std::fs::metadata(&directory).expect("在").permissions().mode() & 0o777, 0o700);
        assert!(control_directory_in(&base, uid).is_some(), "已经有了也能用");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).expect("改得了权限");
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
        assert_eq!(poll("prod-hk", Path::new("/nonexistent/nimbus/sock")), Err(Failure::NotConnected));
        assert!(matches!(poll("-oProxyCommand=x", Path::new("/tmp")), Err(Failure::Failed(_))));
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
        assert_eq!(parse_hosts(text), ["prod-hk", "staging", "Staging2", "eq-sign", "quoted"]);
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
            target: Target::Ssh { host: "nimbus-test.invalid".into() },
            columns: 80,
            rows: 24,
            directory: None,
            shell: None,
            dark_background: false,
            control: None,
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
        assert!(text.contains("nimbus-test.invalid"), "输出里要说明连的是谁：{text}");
    }
}
