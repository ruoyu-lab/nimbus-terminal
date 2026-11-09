//! 显示：响铃、光标、配色。

use super::*;

/// 程序响铃：设成画面闪一下时闪（一批输出里响几次也只闪一次），关闭时什么也不做；
/// 设置马上生效，不用重开会话
#[test]
fn the_bell_follows_the_setting() {
    let (_harness, _recorder, _grid, _cell, session) = mounted_with(
        "",
        Settings {
            bell: Bell::Flash,
            ..Settings::default()
        },
    );
    session.feed_for_test("\x07\x07\x07");
    assert_eq!(session.flashes.get_untracked(), 1);
    session.feed_for_test("no bell");
    assert_eq!(session.flashes.get_untracked(), 1);
    session.feed_for_test("\x07");
    assert_eq!(session.flashes.get_untracked(), 2);
    session.form.bell.set(Bell::Off);
    session.feed_for_test("\x07");
    assert_eq!(session.flashes.get_untracked(), 2, "关了就不闪");
    session.form.bell.set(Bell::Flash);
    session.feed_for_test("\x07");
    assert_eq!(session.flashes.get_untracked(), 3);
}

/// 响铃闪一下：底色亮一下，再恢复
#[test]
fn a_flash_lights_the_background_briefly() {
    let (harness, _recorder, grid, _cell, session) = mounted_with(
        "",
        Settings {
            bell: Bell::Flash,
            ..Settings::default()
        },
    );
    let corner = |harness: &Harness| {
        harness
            .screenshot()
            .pixel((grid.x + grid.width + 6.0) as u32, (grid.y + 6.0) as u32)
            .0
    };
    harness.advance(secs(1.0));
    let normal = corner(&harness);
    session.feed_for_test("\x07");
    harness.advance(ms(60));
    let flashing = corner(&harness);
    assert_ne!(normal, flashing, "闪的时候底色变了");
    harness.advance(secs(1.0));
    assert_eq!(corner(&harness), normal, "闪完恢复");
}

/// 默认光标形状、闪烁听设置的；程序自己指定了形状就听程序的，程序说「回到默认」又交还给设置
#[test]
fn the_cursor_shape_defaults_to_the_setting_and_programs_can_override_it() {
    let (_harness, _recorder, _grid, _cell, session) = mounted_with(
        "",
        Settings {
            cursor: CursorStyle::Bar,
            cursor_blink: false,
            ..Settings::default()
        },
    );
    let caret = || session.grid.caret();
    assert_eq!(caret().shape, aureole::CaretShape::Bar);
    assert!(!caret().blinking, "设置里关了闪烁");
    session.feed_for_test("\x1b[4 q");
    assert_eq!(
        caret().shape,
        aureole::CaretShape::Underline,
        "程序指定的下划线"
    );
    session.feed_for_test("\x1b[0 q");
    assert_eq!(
        caret().shape,
        aureole::CaretShape::Bar,
        "回到默认：设置的竖线"
    );
    session.form.cursor.set(CursorStyle::Block);
    session.form.cursor_blink.set(true);
    session.restyle();
    assert_eq!(caret().shape, aureole::CaretShape::Block, "改设置马上换");
    assert!(caret().blinking);
}

/// 配色：同一个 ANSI 红色，浅色和深色配色画出不同的颜色，换配色时屏幕上已有的字也跟着重画
#[test]
fn changing_the_palette_repaints_what_is_already_on_screen() {
    let (_harness, _recorder, _grid, _cell, session) = mounted_with("", Settings::default());
    session.feed_for_test("\x1b[31mred\x1b[0m");
    let red = || session.grid.cell(0, 0).expect("有字").style.foreground;
    let light = red();
    assert!(light.is_some());
    crate::theme::dark().set(true);
    session.restyle();
    assert_ne!(red(), light, "深色配色里红色不一样");
    crate::theme::dark().set(false);
    session.restyle();
    assert_eq!(red(), light);
}
