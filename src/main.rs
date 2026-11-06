//! Aureole 终端。
//!
//! 分层，从下往上看：
//! - `pty`：起子进程、搬字节、改窗口大小（跨平台的接缝在 `pty::PtyBackend`，目前只有类 Unix 的实现）
//! - `vt`：把字节流解析成屏幕模型（不认识界面）；键盘、鼠标的编码也在这里
//! - `view`：把屏幕模型同步到 `CharGrid`（不认识字节）
//! - `app`：一个终端会话：把三者接到一块网格上（按键、指针、滚轮、输入法、改大小）
//! - `tabs`：窗口的状态：标签（每个一个会话）、侧栏、页面、命令面板
//! - `native`：macOS 上一处原生补丁（标题栏区域的拖动不让系统把窗口拖走）
//! - `chrome`：窗口的外观：标题栏、侧栏、主卡片、状态条、命令面板（`theme` 是颜色，`icons` 是图标）

mod app;
mod chrome;
mod icons;
mod native;
mod pty;
mod tabs;
mod theme;
mod view;
mod vt;

use aureole::{App, TitleBar, WindowMaterial};

fn main() {
    App::new(tabs::root)
        .title("终端")
        .size(tabs::WINDOW_SIZE.0, tabs::WINDOW_SIZE.1)
        .window(|window| {
            window
                // 标题栏自己画成标签栏，内容铺满整个窗口；macOS 的红绿灯留着
                .title_bar(TitleBar::Hidden)
                .title_bar_controls_position(tabs::CONTROLS_POSITION.0, tabs::CONTROLS_POSITION.1)
                // 外框（标题栏和侧栏）透出系统的窗口材质，白色的主卡片浮在上面；
                // 平台不支持时照常不透明（底色由 `chrome` 按能力决定）
                .backdrop(WindowMaterial::Sidebar)
                // 再小就放不下侧栏和标签了
                .min_size(720, 460)
        })
        .run();
}
