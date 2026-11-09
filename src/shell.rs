//! Shell 集成：把 `OSC 133`（命令标记）、`OSC 7`（当前目录）的脚本放到配置目录里，本机的 shell 起来时自动加载，
//! 不改用户的任何配置文件：
//! - zsh：靠 `ZDOTDIR` 垫片
//! - Windows 上的 PowerShell（5.1 和 7）：`-NoExit -Command` 在配置文件之后加载脚本
//!
//! 别的 shell 手动 `source` 同目录下的脚本。加一个 shell：在 `assets/shell/` 放它的脚本，再在 [`integrate`] 里认出它、让它起来时加载。

use std::path::{Path, PathBuf};

use crate::pty;
use crate::store::Store;

const ZSH: &str = include_str!("../assets/shell/nimbus.zsh");
const BASH: &str = include_str!("../assets/shell/nimbus.bash");
const ZSHENV: &str = include_str!("../assets/shell/zdotdir/.zshenv");
/// 带 BOM：Windows PowerShell 5.1 读没有 BOM 的脚本按本地编码，手动 `. 路径` 加载时里面的中文注释会乱
const POWERSHELL: &str = concat!("\u{feff}", include_str!("../assets/shell/nimbus.ps1"));

/// 把脚本写到配置目录下的 `shell/`，返回那个目录。内容没变就不重写；写不了（没有配置目录、只读）是 `None`
pub fn install(store: &Store) -> Option<PathBuf> {
    let directory = store.directory()?.join("shell");
    let zdotdir = directory.join("zdotdir");
    std::fs::create_dir_all(&zdotdir).ok()?;
    let files: [(PathBuf, &str); 4] = [
        (directory.join("nimbus.zsh"), ZSH),
        (directory.join("nimbus.bash"), BASH),
        (directory.join("nimbus.ps1"), POWERSHELL),
        (zdotdir.join(".zshenv"), ZSHENV),
    ];
    for (path, content) in files {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
            std::fs::write(&path, content).ok()?;
        }
    }
    Some(directory)
}

/// 让 `config` 要起的 shell 加载集成脚本（`directory` 是 [`install`] 返回的目录）。
/// 认的是要起的程序本身（`powershell.exe`、`pwsh.exe` 都行），不是默认 shell；认不出来的（设置里指定的别的 shell）不动
pub fn integrate(directory: &Path, config: &mut pty::Config) {
    config
        .environment
        .extend(environment(directory, &config.program));
    config
        .arguments
        .extend(arguments(directory, &config.program));
}

/// 起 `program` 时要加的环境变量。只有 zsh 要（`ZDOTDIR` 垫片）；别的 shell 是空的
pub fn environment(directory: &Path, program: &str) -> Vec<(String, String)> {
    let is_zsh = Path::new(program)
        .file_name()
        .is_some_and(|name| name == "zsh");
    if !is_zsh {
        return Vec::new();
    }
    let mut environment = vec![
        (
            "ZDOTDIR".to_string(),
            directory.join("zdotdir").to_string_lossy().into_owned(),
        ),
        (
            "NIMBUS_ZSH_INTEGRATION".to_string(),
            directory.join("nimbus.zsh").to_string_lossy().into_owned(),
        ),
    ];
    // 用户自己设过 ZDOTDIR：垫片把它还回去；没设过就不传，垫片会 unset
    if let Some(original) = std::env::var_os("ZDOTDIR") {
        environment.push((
            "NIMBUS_ORIG_ZDOTDIR".to_string(),
            original.to_string_lossy().into_owned(),
        ));
    }
    environment
}

/// 起 `program` 时要接在最后的参数。只有 Windows 上的 PowerShell 有：`-NoExit -Command 加载脚本`，
/// 配置文件在 `-Command` 之前跑，所以脚本包得住用户自己的提示符。别的 shell（类 Unix 上的 `pwsh` 也是）是空的
pub fn arguments(directory: &Path, program: &str) -> Vec<String> {
    let is_powershell = cfg!(windows)
        && matches!(
            pty::program_name(program).as_deref(),
            Some("powershell") | Some("pwsh")
        );
    if !is_powershell {
        return Vec::new();
    }
    vec![
        "-NoExit".to_string(),
        "-Command".to_string(),
        powershell_command(&directory.join("nimbus.ps1")),
    ]
}

/// 加载 PowerShell 脚本的那条命令。不直接 `. 路径`：那样受执行策略管，Windows 默认的 `Restricted` 不让跑脚本文件；
/// 读出文字、建成脚本块再点进全局作用域就不受它管，按 UTF-8 读也不怕 5.1 把中文当成本地编码。
/// 加载不了（受限语言模式之类）就算了，shell 照常能用
fn powershell_command(script: &Path) -> String {
    format!(
        "try {{ . ([ScriptBlock]::Create([IO.File]::ReadAllText({}))) }} catch {{ }}",
        single_quoted(&script.to_string_lossy())
    )
}

/// 写成 PowerShell 的单引号字符串：里面的单引号写两遍。PowerShell 把弯的单引号（‘ ’ ‚ ‛）也当单引号，一样写两遍
fn single_quoted(text: &str) -> String {
    let mut quoted = String::from("'");
    for ch in text.chars() {
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push(ch);
        }
        quoted.push(ch);
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_scripts_and_only_zsh_gets_environment() {
        let directory = crate::store::tests::scratch("shell-install");
        let store = Store::at(&directory);
        let installed = install(&store).expect("写得下");
        for name in ["nimbus.zsh", "nimbus.bash", "nimbus.ps1", "zdotdir/.zshenv"] {
            assert!(installed.join(name).is_file(), "{name}");
        }
        let script = std::fs::read(installed.join("nimbus.ps1")).expect("读得出来");
        assert!(
            script.starts_with("\u{feff}".as_bytes()),
            "PowerShell 脚本带 BOM"
        );
        // 再装一次不出错、不改内容
        assert_eq!(install(&store), Some(installed.clone()));
        assert!(install(&Store::none()).is_none(), "不落盘时没有地方放");
        let zsh = environment(&installed, "/bin/zsh");
        assert!(zsh.iter().any(|(name, _)| name == "ZDOTDIR"));
        assert!(
            zsh.iter()
                .any(|(name, value)| name == "NIMBUS_ZSH_INTEGRATION"
                    && value.ends_with("nimbus.zsh"))
        );
        assert!(environment(&installed, "/bin/bash").is_empty());
        assert!(environment(&installed, "/usr/local/bin/fish").is_empty());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// 单引号（直的、弯的）写两遍，别的原样
    #[test]
    fn paths_are_single_quoted_for_powershell() {
        assert_eq!(single_quoted(r"C:\Users\me\x.ps1"), r"'C:\Users\me\x.ps1'");
        assert_eq!(
            single_quoted("C:\\O'Brien\\李’s $dir\\x.ps1"),
            "'C:\\O''Brien\\李’’s $dir\\x.ps1'"
        );
    }

    /// Windows 上的 PowerShell（5.1 和 7，不论写法）在最后加上加载脚本的参数，环境变量不加；别的 shell 什么都不加
    #[cfg(windows)]
    #[test]
    fn powershell_loads_the_script_after_its_profile() {
        let directory = Path::new(r"C:\Users\me\AppData\Roaming\Nimbus\shell");
        for program in [
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Program Files\PowerShell\7\PWSH.EXE",
            "pwsh",
        ] {
            let mut config = pty::Config::shell(program, 80, 24);
            integrate(directory, &mut config);
            assert_eq!(
                config.arguments,
                [
                    "-NoLogo",
                    "-NoExit",
                    "-Command",
                    r"try { . ([ScriptBlock]::Create([IO.File]::ReadAllText('C:\Users\me\AppData\Roaming\Nimbus\shell\nimbus.ps1'))) } catch { }"
                ],
                "{program}"
            );
            assert!(config.environment.is_empty(), "{program}");
        }
        for program in [
            r"C:\Windows\System32\cmd.exe",
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\tools\nu.exe",
        ] {
            let mut config = pty::Config::shell(program, 80, 24);
            let before = config.arguments.clone();
            integrate(directory, &mut config);
            assert_eq!(config.arguments, before, "{program}");
            assert!(config.environment.is_empty(), "{program}");
        }
    }

    /// 类 Unix 上不变：只有 zsh 靠环境变量加载，`pwsh` 不加参数
    #[cfg(unix)]
    #[test]
    fn only_zsh_is_integrated_on_unix() {
        let directory = Path::new("/home/me/.config/nimbus-terminal/shell");
        let mut config = pty::Config::shell("/usr/local/bin/pwsh", 80, 24);
        integrate(directory, &mut config);
        assert!(config.arguments.is_empty());
        assert!(config.environment.is_empty());
        let mut config = pty::Config::shell("/bin/zsh", 80, 24);
        integrate(directory, &mut config);
        assert_eq!(config.arguments, ["-l"]);
        assert!(config.environment.iter().any(|(name, _)| name == "ZDOTDIR"));
    }
}
