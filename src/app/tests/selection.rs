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

/// macOS：⌘C 复制、⌘A 全选是网格自己做的（终端不收 ⌘ 的组合键），什么也不写给 shell
#[test]
fn command_c_and_a_copy_and_select_through_the_grid() {
    let (harness, recorder, _grid, _cell, session) = mounted_with("", Settings::default());
    session.feed_for_test("hello\r\nworld");
    harness.frame();
    aureole::clipboard::write_text("旧的");
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("world"))
    );
    harness.key_down_with(Key::Character("c".into()), Modifiers::SUPER);
    assert!(
        aureole::clipboard::read_text().is_some_and(|text| text.contains("hello")),
        "⌘C 复制了"
    );
    assert_eq!(recorder.take(), "");
}

/// Windows、Linux：Ctrl+Shift+C、Ctrl+Insert 复制选中的字，不给 shell 发 Ctrl+C；
/// 单独的 Ctrl+C 照常是给 shell 的中断，不动剪贴板
#[cfg(not(target_os = "macos"))]
#[test]
fn ctrl_shift_c_and_ctrl_insert_copy_without_interrupting_the_shell() {
    let (harness, recorder, _grid, _cell, session) = mounted_with("", Settings::default());
    harness.set_conventions(aureole::Conventions::WINDOWS);
    session.feed_for_test("hello\r\nworld");
    harness.frame();
    let copy_keys = [
        (
            Key::Character("C".into()),
            Modifiers::CONTROL | Modifiers::SHIFT,
        ),
        (Key::Named(NamedKey::Insert), Modifiers::CONTROL),
    ];
    for (key, modifiers) in copy_keys {
        let label = format!("{modifiers:?} {key:?}");
        aureole::clipboard::write_text("旧的");
        session.select_all();
        assert!(harness.key_down_with(key, modifiers), "{label} 收下了");
        assert!(
            aureole::clipboard::read_text().is_some_and(|text| text.contains("hello")),
            "{label} 复制了"
        );
        assert_eq!(recorder.take(), "", "{label} 不给 shell 发东西");
        assert!(session.selected_text().is_some(), "{label} 复制完选区还在");
    }
    // 没选中时按复制键：剪贴板不动，也不漏给 shell
    session.grid.set_selection(None);
    aureole::clipboard::write_text("旧的");
    harness.key_down_with(
        Key::Character("C".into()),
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    assert_eq!(aureole::clipboard::read_text().as_deref(), Some("旧的"));
    assert_eq!(recorder.take(), "");
    // 单独的 Ctrl+C：中断
    session.select_all();
    harness.key_down_with(Key::Character("c".into()), Modifiers::CONTROL);
    assert_eq!(recorder.take(), "\x03");
    assert_eq!(aureole::clipboard::read_text().as_deref(), Some("旧的"));
}

/// Windows、Linux：Shift+Insert 粘贴；Ctrl+Shift+A 全选不发东西；单独的 Ctrl+A 只给 shell（回行首），
/// 网格不再把它也当全选
#[cfg(not(target_os = "macos"))]
#[test]
fn shift_insert_pastes_and_only_ctrl_shift_a_selects_the_screen() {
    let (harness, recorder, _grid, _cell, session) = mounted_with("", Settings::default());
    harness.set_conventions(aureole::Conventions::WINDOWS);
    session.feed_for_test("hello\r\nworld");
    harness.frame();
    aureole::clipboard::write_text("echo hi");
    harness.key_down_with(Key::Named(NamedKey::Insert), Modifiers::SHIFT);
    assert_eq!(recorder.take(), "echo hi");
    harness.key_down_with(Key::Character("a".into()), Modifiers::CONTROL);
    assert_eq!(recorder.take(), "\x01");
    assert_eq!(session.selected_text(), None, "Ctrl+A 不选中整屏");
    harness.key_down_with(
        Key::Character("A".into()),
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("world"))
    );
    assert_eq!(recorder.take(), "");
}
