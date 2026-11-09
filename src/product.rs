//! 软件自己的名字和链接：窗口标题、配置文件夹、关于页都从这里取，改名只改这一处（和 `Cargo.toml` 里的 `name`）。

/// 软件的名字
pub const NAME: &str = "Nimbus";
/// 源码仓库
pub const REPOSITORY: &str = "https://github.com/ruoyu-lab/nimbus-terminal";
/// 用的界面框架的名字
pub const FRAMEWORK: &str = "Aureole";
/// 界面框架的主页
pub const FRAMEWORK_URL: &str = "https://github.com/ruoyu-lab/aureole";
/// 改名以前用过的配置文件夹名，第一次启动时把它改成现在的名字，设置、文件夹、标签都带过来
pub const LEGACY_DIRECTORY: &str = "Aureole 终端";

#[cfg(test)]
mod tests {
    /// 打包设置（`Cargo.toml` 的 `[package.metadata.bundle]`）里的应用名就是这里的名字；包标识符和 macOS 打包脚本里的一样：
    /// Windows 的应用标识（任务栏分组、系统通知认它）和 macOS 的包标识符是同一个
    #[test]
    fn the_bundle_settings_match_the_name_and_the_macos_bundle() {
        let manifest = include_str!("../Cargo.toml");
        let (_, section) = manifest
            .split_once("[package.metadata.bundle]")
            .expect("有打包设置");
        let section = section.split("\n[").next().unwrap_or(section);
        let setting = |key: &str| {
            section.lines().find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name.trim() == key).then(|| value.trim().trim_matches('"').to_string())
            })
        };
        assert_eq!(setting("name").as_deref(), Some(super::NAME));
        let identifier = setting("identifier").expect("有包标识符");
        let script = include_str!("../scripts/bundle-macos.sh");
        assert!(
            script.contains(&format!(
                "<key>CFBundleIdentifier</key><string>{identifier}</string>"
            )),
            "macOS 打包脚本里的包标识符应该也是 {identifier}"
        );
    }
}
