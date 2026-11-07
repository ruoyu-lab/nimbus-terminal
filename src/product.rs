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
