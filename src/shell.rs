//! Shell 集成：把 `OSC 133` 的脚本放到配置目录里，本机的 zsh 起来时自动加载（靠 `ZDOTDIR` 垫片，
//! 不改用户的任何配置文件）。别的 shell 手动 `source` 同目录下的脚本。
//! 加一个 shell（比如 Windows 的 PowerShell）：在 `assets/shell/` 放它的脚本，再在 [`environment`] 里认出它、让它起来时加载。

use std::path::{Path, PathBuf};

use crate::store::Store;

const ZSH: &str = include_str!("../assets/shell/nimbus.zsh");
const BASH: &str = include_str!("../assets/shell/nimbus.bash");
const ZSHENV: &str = include_str!("../assets/shell/zdotdir/.zshenv");

/// 把脚本写到配置目录下的 `shell/`，返回那个目录。内容没变就不重写；写不了（没有配置目录、只读）是 `None`
pub fn install(store: &Store) -> Option<PathBuf> {
    let directory = store.directory()?.join("shell");
    let zdotdir = directory.join("zdotdir");
    std::fs::create_dir_all(&zdotdir).ok()?;
    let files: [(PathBuf, &str); 3] = [
        (directory.join("nimbus.zsh"), ZSH),
        (directory.join("nimbus.bash"), BASH),
        (zdotdir.join(".zshenv"), ZSHENV),
    ];
    for (path, content) in files {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
            std::fs::write(&path, content).ok()?;
        }
    }
    Some(directory)
}

/// 起 `program` 时要加的环境变量。只有 zsh 能自动加载；别的 shell 是空的
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_scripts_and_only_zsh_gets_environment() {
        let directory = crate::store::tests::scratch("shell-install");
        let store = Store::at(&directory);
        let installed = install(&store).expect("写得下");
        for name in ["nimbus.zsh", "nimbus.bash", "zdotdir/.zshenv"] {
            assert!(installed.join(name).is_file(), "{name}");
        }
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
}
