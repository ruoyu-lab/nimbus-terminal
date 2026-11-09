//! 鼠标：上报、滚轮、右键。

use super::*;

/// 程序开了鼠标上报（1000 + SGR）：点一下，按下和松开都要报，坐标是点中的那一格
#[test]
fn click_is_reported_with_the_cell_under_the_pointer() {
    let (harness, recorder, grid, cell) = mounted("\x1b[?1000h\x1b[?1006h");
    let (x, y) = center(grid, cell, 4, 9);
    harness.click(x, y);
    assert_eq!(recorder.take(), "\x1b[<0;10;5M\x1b[<0;10;5m");
}

/// 没开鼠标上报时，点击不能漏给 shell（会在提示符里打出乱码）
#[test]
fn click_is_not_reported_when_the_program_did_not_ask() {
    let (harness, recorder, grid, cell) = mounted("");
    let (x, y) = center(grid, cell, 4, 9);
    harness.click(x, y);
    assert_eq!(recorder.take(), "");
}

/// 按着 Shift 点：永远归终端（选字），不上报
#[test]
fn shift_click_bypasses_mouse_reporting() {
    let (harness, recorder, grid, cell) = mounted("\x1b[?1000h\x1b[?1006h");
    let (x, y) = center(grid, cell, 1, 1);
    harness.click_with(x, y, Modifiers::SHIFT);
    assert_eq!(recorder.take(), "");
}

/// 1003 下没按键的移动也报，而且同一格里动不重复报
#[test]
fn motion_is_reported_once_per_cell_in_any_motion_mode() {
    let (harness, recorder, grid, cell) = mounted("\x1b[?1003h\x1b[?1006h");
    let (x, y) = center(grid, cell, 2, 3);
    harness.mouse_move(x, y);
    harness.mouse_move(x + 1.0, y);
    assert_eq!(recorder.take(), "\x1b[<35;4;3M");
    let (x, y) = center(grid, cell, 2, 4);
    harness.mouse_move(x, y);
    assert_eq!(recorder.take(), "\x1b[<35;5;3M");
}

/// 滚轮一格滚几行听设置的（默认 3）
#[test]
fn the_wheel_scrolls_as_many_lines_as_the_setting_says() {
    for (setting, expected) in [(3.0, 3), (10.0, 10), (1.0, 1)] {
        let (harness, _recorder, grid, _cell, session) = mounted_with(
            "",
            Settings {
                wheel: setting,
                ..Settings::default()
            },
        );
        session.feed_for_test(&lines(120));
        let [x, y] = grid.center();
        // 滚轮往回拨一格：往更早的历史翻
        harness.wheel(x, y, 0.0, -1.0);
        assert_eq!(session.offset.get(), expected, "滚轮设成 {setting}");
    }
}

/// 历史行数听设置的：写得再多也只留设定的行数；调小马上丢掉多出来的
#[test]
fn the_scrollback_setting_limits_the_history() {
    let (_harness, _recorder, _grid, _cell, session) = mounted_with(
        "",
        Settings {
            scrollback: 200.0,
            ..Settings::default()
        },
    );
    session.feed_for_test(&lines(1000));
    assert_eq!(session.terminal.borrow().scrollback_len(), 200);
    session.form.scrollback.set(100.0);
    session.apply_scrollback();
    assert_eq!(session.terminal.borrow().scrollback_len(), 100);
    // 整台终端复位以后上限还在
    session.feed_for_test("\x1bc");
    session.feed_for_test(&lines(500));
    assert_eq!(session.terminal.borrow().scrollback_len(), 100);
}

/// 右键：没抓鼠标、没开「右键粘贴」时弹菜单（回调拿到点的位置）；开了右键粘贴就粘贴、不弹；
/// 程序抓着鼠标时归程序、不弹（按着 Shift 例外）
#[test]
fn right_click_opens_the_menu_unless_pasting_or_the_program_wants_the_mouse() {
    let menus = |setup: &str, paste: bool, modifiers: Modifiers| {
        let recorder = Arc::new(Recorder::default());
        let session = Session::new(
            {
                let mut terminal = Terminal::new(80, 24);
                terminal.feed(setup.as_bytes());
                terminal
            },
            CharGrid::new(80, 24),
            Some(Pty::from_backend(recorder)),
            Hooks::none(),
            Form::new(&Settings {
                right_click_paste: paste,
                ..Settings::default()
            }),
        );
        let opened = Rc::new(Cell::new(None));
        let seen = opened.clone();
        let harness = Harness::new(
            960.0,
            600.0,
            view(session, || true, move |at| seen.set(Some(at))),
        );
        harness.frame();
        let grid = harness.bounds("grid").expect("网格在");
        let [x, y] = grid.center();
        harness.set_modifiers(modifiers);
        harness.mouse_down(x, y, MouseButton::Right);
        harness.mouse_up(MouseButton::Right);
        opened
            .get()
            .map(|at| (at[0] - x).abs() < 1.0 && (at[1] - y).abs() < 1.0)
    };
    assert_eq!(
        menus("", false, Modifiers::default()),
        Some(true),
        "默认弹菜单，位置是点的地方"
    );
    assert_eq!(
        menus("", true, Modifiers::default()),
        None,
        "右键粘贴时不弹"
    );
    assert_eq!(
        menus("\x1b[?1000h", false, Modifiers::default()),
        None,
        "程序抓着鼠标不弹"
    );
    assert_eq!(
        menus("\x1b[?1000h", false, Modifiers::SHIFT),
        Some(true),
        "按着 Shift 还是弹"
    );
}
