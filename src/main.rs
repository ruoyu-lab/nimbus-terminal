//! Nimbus：用 Aureole 框架写的终端。
//!
//! 分层，从下往上看：
//! - `pty`：起子进程、搬字节、改窗口大小（跨平台的接缝在 `pty::PtyBackend`，目前只有类 Unix 的实现）
//! - `target`：会话连到哪里（本机、SSH 服务器、Docker 容器）；`ssh`、`docker` 是各自的命令层，`remote` 按目标分发
//! - `vt`：把字节流解析成屏幕模型（不认识界面）；键盘、鼠标的编码也在这里
//! - `view`：把屏幕模型同步到 `CharGrid`（不认识字节）；终端配色也在这里
//! - `app`：一个终端会话：把三者接到一块网格上（按键、指针、滚轮、输入法、改大小）
//! - `store`、`settings`、`display`：落盘、设置项、窗口位置按显示器分别记住
//! - `tabs`：窗口的状态：标签（每个一个会话）、文件夹、侧栏、页面、命令面板
//! - `native`：平台补丁（macOS 上标题栏区域的拖动不让系统把窗口拖走；提示音）
//! - `chrome`：窗口的外观：标题栏、侧栏、主卡片、状态条、命令面板（`theme` 是颜色，`icons` 是图标）

mod app;
mod channel;
mod chrome;
mod display;
mod docker;
mod files;
mod icons;
mod monitor;
mod native;
mod pane;
mod product;
mod pty;
mod remote;
mod settings;
mod ssh;
mod store;
mod tabs;
mod target;
mod theme;
mod view;
mod vt;

use aureole::{App, TitleBar, WindowMaterial};

/// 出了 panic 把原因记到配置目录的 `crash.log`（时间、线程、位置、调用栈）。
/// 在事件回调里 panic 会直接 abort，系统的崩溃报告里只有「中止」，看不到 panic 的原因，所以自己留一份
fn log_panics(store: &store::Store) {
    let Some(directory) = store.directory().map(std::path::Path::to_path_buf) else {
        return;
    };
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        use std::io::Write;
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::create_dir_all(&directory);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("crash.log"))
        {
            let _ = writeln!(
                file,
                "[unix {seconds}] 线程 {} panic：{info}\n{backtrace}\n",
                thread.name().unwrap_or("?")
            );
        }
    }));
}

fn main() {
    log_panics(&store::Store::system());
    // 上次窗口在哪块显示器、多大、在哪：显示器变了（拔掉、换分辨率、换缩放）就按现在的重新摆，不会开到屏幕外面
    let plan = display::Memory::load(&store::Store::system()).plan(
        &display::displays(),
        (tabs::WINDOW_SIZE.0 as f32, tabs::WINDOW_SIZE.1 as f32),
    );
    App::new(tabs::root)
        .title(product::NAME)
        .size(plan.width, plan.height)
        .window(move |window| {
            let window = match plan.position {
                Some((x, y)) => window.position(x, y),
                None => window,
            };
            window
                // 标题栏自己画成标签栏，内容铺满整个窗口；macOS 的红绿灯留着
                .title_bar(TitleBar::Hidden)
                .title_bar_controls_position(tabs::CONTROLS_POSITION.0, tabs::CONTROLS_POSITION.1)
                // 外框（标题栏和侧栏）透出系统的窗口材质，白色的主卡片浮在上面；
                // 平台不支持时照常不透明（底色由 `chrome` 按能力决定）
                .backdrop(WindowMaterial::Sidebar)
                // 再小就放不下侧栏和标签了
                .min_size(display::MIN_SIZE.0, display::MIN_SIZE.1)
                // 点关闭先把工作区存一遍：之后窗口一关，shell 就收到 SIGHUP，屏幕上的内容会变
                .on_close_request(|_| tabs::save_on_exit())
        })
        // 退出前把工作区存最后一次（shell 的当前目录是这一刻的）
        .on_exit(tabs::save_on_exit)
        .run();
}
