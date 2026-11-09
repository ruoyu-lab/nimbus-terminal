//! 选区、复制、粘贴。

use super::*;

/// 选中后自动复制：开了才复制；程序抓着鼠标时（Shift 之外）不碰剪贴板
#[test]
fn copy_on_select_copies_when_enabled_only() {
    let select = |copy: bool| {
        let (harness, _recorder, grid, cell, session) = mounted_with(
            "hello world",
            Settings {
                copy_on_select: copy,
                ..Settings::default()
            },
        );
        session.feed_for_test("");
        aureole::clipboard::write_text("旧的");
        let (from, to) = (center(grid, cell, 0, 0), center(grid, cell, 0, 4));
        harness.drag([from.0 - cell.0 * 0.4, from.1], [to.0 + cell.0 * 0.4, to.1]);
        aureole::clipboard::read_text()
    };
    assert_eq!(select(true).as_deref(), Some("hello"));
    assert_eq!(select(false).as_deref(), Some("旧的"));
}

/// 右键粘贴：开了才粘贴
#[test]
fn right_click_pastes_when_enabled_only() {
    for (enabled, expected) in [(true, "from clipboard"), (false, "")] {
        let (harness, recorder, grid, cell, _session) = mounted_with(
            "",
            Settings {
                right_click_paste: enabled,
                ..Settings::default()
            },
        );
        aureole::clipboard::write_text("from clipboard");
        let (x, y) = center(grid, cell, 3, 3);
        harness.right_click(x, y);
        assert_eq!(recorder.take(), expected, "右键粘贴 {enabled}");
    }
}

/// 复制只复制选中的；全选选满屏幕；清屏连历史一起清并让 shell 重画
#[test]
fn copy_select_all_and_clear() {
    let (_harness, recorder, _grid, _cell, session) = mounted_with("", Settings::default());
    assert_eq!(session.selected_text(), None);
    session.feed_for_test("hello\r\nworld");
    aureole::clipboard::write_text("旧的");
    session.copy_selection();
    assert_eq!(
        aureole::clipboard::read_text().as_deref(),
        Some("旧的"),
        "没选中不动剪贴板"
    );
    session.select_all();
    let selected = session.selected_text().expect("全选了");
    assert!(
        selected.contains("hello") && selected.contains("world"),
        "{selected:?}"
    );
    session.copy_selection();
    assert!(
        aureole::clipboard::read_text()
            .expect("有")
            .contains("hello")
    );
    session.feed_for_test(&lines(100));
    assert!(session.terminal.borrow().scrollback_len() > 0);
    session.clear_screen();
    assert_eq!(session.terminal.borrow().scrollback_len(), 0, "历史清了");
    assert_eq!(
        session
            .screen_text_for_test()
            .replace(['\0', ' ', '\n'], ""),
        ""
    );
    assert_eq!(recorder.take(), "\x0c", "让 shell 重画提示符");
}
