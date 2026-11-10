//! WSL 的终端（Windows）：目录、拖进来的文件、链接都是 Linux 的写法，「有程序在跑」看命令标记。

use super::*;

/// 本机的、起的是 `distribution` 这个发行版的会话
fn wsl_session(distribution: &str) -> (Harness, Arc<Recorder>, Rc<Session>) {
    let (harness, recorder, _grid, _cell, session) =
        mounted_with("", crate::settings::Settings::default());
    session.set_local(true);
    session.set_shell_program(Some(crate::wsl::program(distribution)));
    (harness, recorder, session)
}

fn host() -> String {
    std::env::var("COMPUTERNAME").expect("有 COMPUTERNAME")
}

/// 当前目录只看 WSL 里的 shell 报的（`wsl.exe` 这个进程的目录不算）：报的 Linux 路径换成 Windows 的写法；
/// 别的机器报的（在 WSL 里 ssh 出去了）不算
#[test]
fn the_working_directory_comes_from_the_linux_report() {
    let (_harness, recorder, session) = wsl_session("Ubuntu");
    recorder.set_directory(r"C:\Windows");
    assert_eq!(
        session.working_directory(),
        None,
        "没报：不拿进程的目录凑合"
    );
    session.feed_for_test(&format!("\x1b]7;file://{}/home/me/%E4%B8%AD\x07", host()));
    assert_eq!(
        session.working_directory(),
        Some(std::path::PathBuf::from(
            r"\\wsl.localhost\Ubuntu\home\me\中"
        ))
    );
    session.feed_for_test(&format!("\x1b]7;file://{}/mnt/c/Users\x07", host()));
    assert_eq!(
        session.working_directory(),
        Some(std::path::PathBuf::from(r"C:\Users"))
    );
    session.feed_for_test("\x1b]7;file://some-server/srv\x07");
    assert_eq!(session.working_directory(), None, "别的机器上的目录");
    // 同样的报告在 PowerShell 的会话里不算目录（没有盘符）
    session.set_shell_program(Some("powershell.exe".into()));
    session.feed_for_test(&format!("\x1b]7;file://{}/home/me\x07", host()));
    assert_eq!(session.working_directory(), Some(r"C:\Windows".into()));
}

/// 「有程序在跑」：WSL 的终端只看命令标记（Windows 这边看不到 WSL 里的进程），pty 说什么不算
#[test]
fn busy_follows_the_command_marks() {
    let (_harness, recorder, session) = wsl_session("Ubuntu");
    recorder.set_busy(true);
    assert!(!session.is_busy(), "没有命令标记：当没有");
    session.feed_for_test("\x1b]133;A\x07$ \x1b]133;B\x07");
    assert!(!session.is_busy(), "在提示符上");
    session.feed_for_test("vim\r\n\x1b]133;C\x07");
    assert!(session.is_busy(), "命令开始了");
    session.feed_for_test("\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07");
    assert!(!session.is_busy(), "命令结束了");
    session.set_shell_program(Some("powershell.exe".into()));
    assert!(session.is_busy(), "别的 shell 照旧问 pty");
}

/// 从资源管理器拖进 WSL 的终端：盘上的路径写成 `/mnt/c/…`，这个发行版自己的写成 Linux 里的路径，按 `sh` 的规矩引；
/// 换不过去的（共享路径）原样引好写进去
#[test]
fn dropped_files_become_linux_paths() {
    let (_harness, recorder, session) = wsl_session("Ubuntu");
    let files = [
        std::path::PathBuf::from(r"C:\Users\me\a b.txt"),
        std::path::PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\it's"),
        std::path::PathBuf::from(r"\\server\share\x.txt"),
    ];
    assert!(session.drop_files(&files));
    assert_eq!(
        recorder.take(),
        r#"'/mnt/c/Users/me/a b.txt' '/home/me/it'\''s' '\\server\share\x.txt' "#
    );
}

/// 装了的第一个发行版；没装的测试跳过
fn installed() -> Option<String> {
    let found = crate::wsl::distributions()
        .into_iter()
        .next()
        .map(|distribution| distribution.name);
    if found.is_none() {
        eprintln!("没装 WSL 的发行版，跳过");
    }
    found
}

/// 点 WSL 终端里的路径（真的发行版）：`/etc/hostname` 是 `\\wsl.localhost\发行版\etc\hostname`，`~/` 是发行版里那个用户的主目录，
/// 相对路径从报的目录算起；`/mnt/c/…` 是 Windows 的盘
#[test]
fn linux_paths_open_through_the_wsl_share() {
    let Some(distribution) = installed() else {
        return;
    };
    let (_harness, _recorder, session) = wsl_session(&distribution);
    let canonical =
        |path: std::path::PathBuf| path.canonicalize().ok().map(crate::links::without_verbatim);
    let hostname = crate::wsl::to_windows(&distribution, "/etc/hostname").expect("换得过去");
    assert_eq!(
        hostname,
        std::path::PathBuf::from(format!(r"\\wsl.localhost\{distribution}\etc\hostname"))
    );
    assert_eq!(
        session.resolve_path("/etc/hostname"),
        canonical(hostname.clone())
    );
    assert!(session.resolve_path("/etc/hostname").is_some(), "文件在");
    assert!(session.resolve_path("/no/such/nimbus").is_none());
    let home = crate::wsl::home(&distribution).expect("找得到主目录");
    assert_eq!(session.resolve_path("~/"), canonical(home));
    session.feed_for_test(&format!("\x1b]7;file://{}/etc\x07", host()));
    assert_eq!(session.resolve_path("./hostname"), canonical(hostname));
    let windows = crate::store::user_home().expect("有主目录");
    let mounted = format!(
        "/mnt/{}/{}",
        windows.to_string_lossy()[..1].to_ascii_lowercase(),
        windows.to_string_lossy()[3..].replace('\\', "/")
    );
    assert_eq!(session.resolve_path(&mounted), canonical(windows));
}
