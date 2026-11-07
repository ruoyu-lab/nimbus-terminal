//! 落盘：设置、侧栏的文件夹和标签、窗口的位置，各是配置目录里的一个 JSON 文件。
//!
//! 写是尽力而为：写不进去（只读磁盘、没有权限）只在标准错误里留一句话，不打断使用；
//! 读到坏文件、旧版本多出来的字段就退回默认值，不会因为配置坏了起不来。
//! 先写临时文件再改名，写到一半断电也不会留下半个文件。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 想把配置放到别处（试用、排查问题）时设这个环境变量
const OVERRIDE: &str = "NIMBUS_HOME";

/// 配置放在哪个目录。`None` 是不落盘（测试里用）
#[derive(Clone, Debug, Default)]
pub struct Store {
    directory: Option<PathBuf>,
}

impl Store {
    /// 系统约定的位置：macOS 是 `~/Library/Application Support/Nimbus`，
    /// Windows 是 `%APPDATA%\Nimbus`，别的是 `$XDG_CONFIG_HOME/nimbus-terminal`（没有就 `~/.config`）。
    /// 改名之前的旧文件夹还在、新的还没有时，把旧的改成新的名字（设置、文件夹、标签都不丢）
    pub fn system() -> Self {
        let directory = location();
        if let (Some(new), Some(old)) = (&directory, legacy_location()) {
            adopt_legacy(&old, new);
        }
        Self { directory }
    }

    /// 放在指定目录
    #[cfg(test)]
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: Some(directory.into()),
        }
    }

    /// 不落盘：读什么都是默认值，写什么都丢掉
    #[cfg(test)]
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_persistent(&self) -> bool {
        self.directory.is_some()
    }

    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    /// 读 `name`；没有、读不了、格式不对都是默认值
    pub fn load<T: DeserializeOwned + Default>(&self, name: &str) -> T {
        let Some(directory) = &self.directory else {
            return T::default();
        };
        let Ok(text) = std::fs::read_to_string(directory.join(name)) else {
            return T::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            eprintln!("配置 {name} 读不懂，用默认值：{error}");
            T::default()
        })
    }

    /// 写 `name`。内容和文件里已有的一样就不写；返回是否真的写了
    pub fn save<T: Serialize>(&self, name: &str, value: &T) -> bool {
        let Some(directory) = &self.directory else {
            return false;
        };
        let Ok(text) = serde_json::to_string_pretty(value) else {
            return false;
        };
        let path = directory.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return false;
        }
        match write_atomically(directory, &path, &text) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("配置 {name} 存不下：{error}");
                false
            }
        }
    }
}

impl Store {
    /// 读一个文本文件；没有、读不了是 `None`
    pub fn load_text(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.directory.as_ref()?.join(name)).ok()
    }

    /// 写一个文本文件。内容和已有的一样就不写；返回是否真的写了
    pub fn save_text(&self, name: &str, text: &str) -> bool {
        let Some(directory) = &self.directory else {
            return false;
        };
        let path = directory.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return false;
        }
        match write_atomically(directory, &path, text) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("{name} 存不下：{error}");
                false
            }
        }
    }

    /// 删一个文件；本来就没有也不算错
    pub fn remove(&self, name: &str) {
        if let Some(directory) = &self.directory {
            let _ = std::fs::remove_file(directory.join(name));
        }
    }
}

fn write_atomically(directory: &Path, path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

fn location() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(OVERRIDE).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        user_home().map(|home| {
            home.join("Library/Application Support")
                .join(crate::product::NAME)
        })
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join(crate::product::NAME))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| user_home().map(|home| home.join(".config")))
            .map(|path| path.join("nimbus-terminal"))
    }
}

/// 改名以前的配置文件夹在哪（用环境变量指定了位置时没有旧的可言）
fn legacy_location() -> Option<PathBuf> {
    if std::env::var_os(OVERRIDE).is_some_and(|path| !path.is_empty()) {
        return None;
    }
    location().and_then(|new| {
        new.parent()
            .map(|parent| parent.join(crate::product::LEGACY_DIRECTORY))
    })
}

/// 旧文件夹在、新的不在：改名过来。改不了（权限、跨盘）就算了，新的从空白开始
fn adopt_legacy(old: &Path, new: &Path) {
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

/// 用户的主目录。先看 `HOME`（类 Unix，以及有人在 Windows 上自己设了的），没有再用 `USERPROFILE`
pub(crate) fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
    #[serde(default)]
    struct Sample {
        name: String,
        count: u32,
    }

    /// 每个测试一个自己的临时目录，跑完删掉
    pub(crate) fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("nimbus-test-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn what_was_saved_comes_back_and_a_missing_file_is_the_default() {
        let directory = scratch("roundtrip");
        let store = Store::at(&directory);
        assert_eq!(store.load::<Sample>("a.json"), Sample::default());
        let value = Sample {
            name: "终端".into(),
            count: 3,
        };
        assert!(store.save("a.json", &value));
        assert_eq!(store.load::<Sample>("a.json"), value);
        assert!(!store.save("a.json", &value), "没变就不写");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_broken_file_falls_back_and_unknown_fields_are_ignored() {
        let directory = scratch("broken");
        std::fs::create_dir_all(&directory).expect("建得了目录");
        std::fs::write(directory.join("a.json"), "{坏的").expect("写得进");
        std::fs::write(directory.join("b.json"), r#"{"name":"x","future":1}"#).expect("写得进");
        let store = Store::at(&directory);
        assert_eq!(store.load::<Sample>("a.json"), Sample::default());
        assert_eq!(store.load::<Sample>("b.json").name, "x");
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// 改名以后第一次启动：旧文件夹改成新名字，里面的文件原样带过来；新的已经有了就不动
    #[test]
    fn the_old_config_folder_is_adopted_once() {
        let root = scratch("legacy");
        let (old, new) = (root.join("old"), root.join("new"));
        std::fs::create_dir_all(&old).expect("建得了目录");
        std::fs::write(old.join("state.json"), "{\"sidebar_width\":300.0}").expect("写得进");
        adopt_legacy(&old, &new);
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("state.json")).expect("带过来了"),
            "{\"sidebar_width\":300.0}"
        );
        // 新的已经有了：旧的（再出现的）不碰
        std::fs::create_dir_all(&old).expect("建得了目录");
        std::fs::write(old.join("other.json"), "x").expect("写得进");
        adopt_legacy(&old, &new);
        assert!(old.join("other.json").exists());
        assert!(!new.join("other.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_store_means_nothing_is_written() {
        let store = Store::none();
        assert!(!store.save("a.json", &Sample::default()));
        assert!(!store.is_persistent());
    }
}
