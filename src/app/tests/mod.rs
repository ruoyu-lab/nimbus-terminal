//! 会话的测试：按功能分文件，这里放大家共用的辅助函数。

mod alerts;
mod display;
mod hyperlinks;
mod images;
mod marks;
mod mouse;
mod selection;

use std::sync::Arc;

use aureole::testing::Harness;

use super::*;
use crate::pty::Recorder;

/// 离屏挂一个会话（假 pty），先喂 `setup`（让程序开鼠标模式），布局一帧，返回网格的位置和一格的大小
pub(super) fn mounted(setup: &str) -> (Harness, Arc<Recorder>, Rect, (f32, f32)) {
    let (harness, recorder, grid, cell, _session) =
        mounted_with(setup, crate::settings::Settings::default());
    (harness, recorder, grid, cell)
}

/// 同上，用指定的设置，并把会话也交出来
pub(super) fn mounted_with(
    setup: &str,
    settings: crate::settings::Settings,
) -> (Harness, Arc<Recorder>, Rect, (f32, f32), Rc<Session>) {
    let recorder = Arc::new(Recorder::default());
    let (columns, rows) = (80, 24);
    let mut terminal = Terminal::new(columns, rows);
    terminal.feed(setup.as_bytes());
    let session = Session::new(
        terminal,
        CharGrid::new(columns, rows),
        Some(Pty::from_backend(recorder.clone())),
        Hooks::none(),
        Form::new(&settings),
    );
    session.apply_scrollback();
    let harness = Harness::new(960.0, 600.0, view(session.clone(), || true, |_| {}));
    harness.frame();
    let grid = harness.bounds("grid").expect("网格要在元素树里");
    let [columns, rows] = session.grid.size();
    let cell = (grid.width / columns as f32, grid.height / rows as f32);
    (harness, recorder, grid, cell, session)
}

pub(super) fn lines(count: usize) -> String {
    (0..count).map(|n| format!("line {n}\r\n")).collect()
}

/// 第 `row` 行第 `column` 列（0 起算）那一格的中心，窗口坐标
pub(super) fn center(grid: Rect, cell: (f32, f32), row: usize, column: usize) -> (f32, f32) {
    (
        grid.x + (column as f32 + 0.5) * cell.0,
        grid.y + (row as f32 + 0.5) * cell.1,
    )
}

/// 给一个会话喂几条带命令标记的命令
pub(super) fn session_with_commands(count: usize) -> Rc<Session> {
    let session = Session::new(
        Terminal::new(40, 5),
        CharGrid::new(40, 5),
        None,
        Hooks::none(),
        Form::new(&crate::settings::Settings::default()),
    );
    for index in 0..count {
        session.feed_for_test(&format!(
            "\x1b]133;A\x07$ \x1b]133;B\x07cmd{index}\r\n\x1b]133;C\x07out{index}\r\n\x1b]133;D;0\x07"
        ));
    }
    session.feed_for_test("\x1b]133;A\x07$ \x1b]133;B\x07");
    session
}

pub(super) fn session_with_alerts() -> (Rc<Session>, Rc<RefCell<Vec<Alert>>>) {
    let alerts = Rc::new(RefCell::new(Vec::new()));
    let mut hooks = Hooks::none();
    let sink = alerts.clone();
    hooks.alert = Rc::new(move |alert| sink.borrow_mut().push(alert));
    let session = Session::new(
        Terminal::new(40, 5),
        CharGrid::new(40, 5),
        None,
        hooks,
        Form::new(&crate::settings::Settings::default()),
    );
    (session, alerts)
}

use crate::settings::{Bell, CursorStyle, Settings};
