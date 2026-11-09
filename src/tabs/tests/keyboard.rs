//! 快捷键、命令面板、查找条。

use super::*;

/// ⌘T 新开一个并切过去；⌘W 关掉当前的，换到邻居
#[test]
fn command_t_and_w_open_and_close_tabs() {
    let (harness, workspace) = mounted();
    command(&harness, "t");
    command(&harness, "t");
    assert_eq!(ids(&workspace), [1, 2, 3]);
    assert_eq!(workspace.active.get_untracked(), 3);
    command(&harness, "w");
    assert_eq!(ids(&workspace), [1, 2]);
    assert_eq!(
        workspace.active.get_untracked(),
        2,
        "关了最右的，换到左边的"
    );
    workspace.select(1);
    command(&harness, "w");
    assert_eq!(
        workspace.active.get_untracked(),
        2,
        "关了最左的，换到右边的"
    );
}

#[test]
fn number_keys_and_ctrl_tab_switch_tabs() {
    let (harness, workspace) = mounted();
    command(&harness, "t");
    command(&harness, "t");
    command(&harness, "1");
    assert_eq!(workspace.active.get_untracked(), 1);
    command(&harness, "9");
    assert_eq!(workspace.active.get_untracked(), 3, "⌘9 是最后一个");
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    assert_eq!(workspace.active.get_untracked(), 1, "绕回第一个");
    harness.key_down_with(
        Key::Named(NamedKey::Tab),
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    assert_eq!(workspace.active.get_untracked(), 3, "反方向绕回最后一个");
}

/// ⌘B 收起侧栏：宽度变窄，再按展开
#[test]
fn command_b_collapses_and_expands_the_sidebar() {
    let (harness, workspace) = mounted();
    let open = harness.bounds("sidebar").expect("侧栏在").width;
    command(&harness, "b");
    harness.advance(secs(1.0));
    assert!(workspace.collapsed.get_untracked());
    let closed = harness.bounds("sidebar").expect("侧栏在").width;
    assert!(closed < open / 2.0, "收起后只剩图标：{open} → {closed}");
    harness.click_named("toggle-sidebar");
    harness.advance(secs(1.0));
    assert!(!workspace.collapsed.get_untracked());
    assert!(harness.bounds("sidebar").expect("侧栏在").width > closed * 2.0);
}

/// ⌘K 打开命令面板，点命令执行并关掉面板
#[test]
fn command_k_opens_the_palette_and_a_command_runs() {
    let (harness, workspace) = mounted();
    command(&harness, "k");
    assert!(workspace.palette_open.get_untracked());
    // 浮层里的元素，边界是浮层自己的坐标；面板放在窗口正中，换算成窗口坐标再点
    let panel = harness.bounds("palette").expect("面板在");
    let row = harness.bounds("cmd-new-tab").expect("命令在");
    let origin = (
        (WINDOW_SIZE.0 as f32 - panel.width) / 2.0,
        (WINDOW_SIZE.1 as f32 - panel.height) / 2.0,
    );
    let [x, y] = row.center();
    harness.click(origin.0 + x, origin.1 + y);
    harness.frame();
    assert_eq!(ids(&workspace), [1, 2]);
    assert!(!workspace.palette_open.get_untracked());
}

/// ⌘F 打开查找条，打字就找：命中高亮，当前那一处是最新的；回车往更早的找，会翻回历史；Esc 关掉并撤掉高亮
#[test]
fn find_bar_highlights_steps_through_history_and_closes() {
    let (harness, workspace) = mounted();
    let session = workspace.focused_session().expect("有终端");
    for index in 0..120 {
        let marker = if matches!(index, 5 | 60 | 115) {
            "needle"
        } else {
            "hay"
        };
        session.feed_for_test(&format!("row {index} {marker}\r\n"));
    }
    harness.frame();
    assert!(harness.bounds("find-bar").is_none());
    command(&harness, "f");
    harness.advance(secs(1.0));
    assert!(harness.bounds("find-bar").is_some(), "⌘F 打开查找条");
    harness.type_text("needle");
    harness.advance(secs(1.0));
    assert_eq!(harness.text("find-status").as_deref(), Some("3/3"));
    // 最新的一处（115）在屏幕上，是当前；屏幕上只有它
    let lit = session.highlighted_for_test();
    assert_eq!(lit.len(), "needle".len(), "{lit:?}");
    assert!(lit.iter().all(|&(_, _, current)| current));
    assert_eq!(session.offset_for_test(), 0);

    // 回车：上一处（60），在历史里，翻回去后看得见
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert_eq!(harness.text("find-status").as_deref(), Some("2/3"));
    assert!(session.offset_for_test() > 0, "翻回历史");
    let lit = session.highlighted_for_test();
    assert_eq!(lit.len(), "needle".len(), "{lit:?}");
    assert!(lit.iter().all(|&(_, _, current)| current));
    // ⌘⇧G 回到下一处
    harness.key_down_with(
        Key::Character("G".into()),
        Modifiers::SUPER | Modifiers::SHIFT,
    );
    harness.advance(secs(1.0));
    assert_eq!(harness.text("find-status").as_deref(), Some("3/3"));
    assert_eq!(session.offset_for_test(), 0);

    // Esc 关掉：高亮撤掉
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    assert!(harness.bounds("find-bar").is_none(), "Esc 关查找条");
    assert!(session.highlighted_for_test().is_empty());
}

/// 没有命中显示「无结果」；新输出让命中跟着变
#[test]
fn find_bar_reports_no_match_and_follows_new_output() {
    let (harness, workspace) = mounted();
    let session = workspace.focused_session().expect("有终端");
    session.feed_for_test("alpha\r\n");
    command(&harness, "f");
    harness.advance(secs(1.0));
    harness.type_text("zzz");
    harness.advance(secs(1.0));
    assert_eq!(harness.text("find-status").as_deref(), Some("无结果"));
    assert!(session.highlighted_for_test().is_empty());
    // 查找条开着时又来了输出：新的命中也要高亮
    session.feed_for_test("zzz again\r\n");
    harness.advance(secs(1.0));
    assert_eq!(harness.text("find-status").as_deref(), Some("1/1"));
    assert_eq!(session.highlighted_for_test().len(), 3);
}

/// ⌘↑ / ⌘↓ 在命令之间跳：有命令标记时选中那条命令；没有标记时按键不被吞掉，照常交给终端
#[test]
fn command_arrow_keys_jump_between_commands() {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    workspace.new_tab();
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    let session = workspace.focused_session().expect("有终端");
    // 没有命令标记：⌘↑ 没人处理，终端收到它（编码成方向键）
    harness.key_down_with(Key::Named(NamedKey::ArrowUp), Modifiers::SUPER);
    harness.frame();
    assert!(session.selected_text().is_none());
    for index in 0..3 {
        session.feed_for_test(&format!(
            "\x1b]133;A\x07$ \x1b]133;B\x07cmd{index}\r\n\x1b]133;C\x07out{index}\r\n\x1b]133;D;0\x07"
        ));
    }
    session.feed_for_test("\x1b]133;A\x07$ \x1b]133;B\x07");
    harness.key_down_with(Key::Named(NamedKey::ArrowUp), Modifiers::SUPER);
    harness.frame();
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd2") && text.contains("out2"))
    );
    harness.key_down_with(Key::Named(NamedKey::ArrowUp), Modifiers::SUPER);
    harness.frame();
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd1"))
    );
    harness.key_down_with(Key::Named(NamedKey::ArrowDown), Modifiers::SUPER);
    harness.frame();
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd2"))
    );
}

/// Windows、Linux：Ctrl+Shift+D 是左右分（Shift 本来就按着，不再算上下分）；Alt+Shift+加号、减号左右分、上下分
/// （Windows Terminal 的按法）。这几个键一个字节也不写给终端：Ctrl+Shift+D 漏过去就是 Ctrl+D，shell 会退出
#[cfg(not(target_os = "macos"))]
#[test]
fn ctrl_shift_d_and_alt_shift_plus_minus_split_panes() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    harness.frame();
    let leaves = || {
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len()
    };
    let (ctrl_shift, alt_shift) = (
        Modifiers::CONTROL | Modifiers::SHIFT,
        Modifiers::ALT | Modifiers::SHIFT,
    );
    harness.key_down_with(Key::Character("D".into()), ctrl_shift);
    harness.frame();
    assert_eq!(leaves(), 2);
    assert!(harness.bounds("pane-divider-columns").is_some(), "左右分");
    assert!(harness.bounds("pane-divider-rows").is_none());
    harness.key_down_with(Key::Character("_".into()), alt_shift);
    harness.frame();
    assert_eq!(leaves(), 3);
    assert!(harness.bounds("pane-divider-rows").is_some(), "上下分");
    harness.key_down_with(Key::Character("+".into()), alt_shift);
    harness.frame();
    assert_eq!(leaves(), 4);
    for index in 0..fake.ptys.borrow().len() {
        assert_eq!(fake.pty(index).take(), "", "第 {index} 个终端什么也没收到");
    }
    // 按着 Ctrl 的不算（AltGr 报上来就是 Ctrl+Alt，那是在打字）
    harness.key_down_with(
        Key::Character("+".into()),
        Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT,
    );
    harness.frame();
    assert_eq!(leaves(), 4);
}

/// Windows、Linux：查找条开着时 F3 往上（更早的）找、Shift+F3 往下找，输入框里 Shift+回车也往下找；
/// Ctrl+Shift+G 往上（Shift 本来就按着，不算往下）。查找条关着时 F3 照常交给终端里的程序
#[cfg(not(target_os = "macos"))]
#[test]
fn f3_and_shift_enter_step_through_matches() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    harness.frame();
    let session = workspace.focused_session().expect("有终端");
    for index in 0..120 {
        let marker = if matches!(index, 5 | 60 | 115) {
            "needle"
        } else {
            "hay"
        };
        session.feed_for_test(&format!("row {index} {marker}\r\n"));
    }
    harness.frame();
    let pty = fake.pty(0);
    pty.take();
    harness.key_down(NamedKey::F3);
    harness.frame();
    assert_eq!(pty.take(), "\x1bOR", "查找条关着：F3 是给程序的");

    let ctrl_shift = Modifiers::CONTROL | Modifiers::SHIFT;
    harness.key_down_with(Key::Character("F".into()), ctrl_shift);
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("find-bar").is_some(),
        "Ctrl+Shift+F 打开查找条"
    );
    harness.type_text("needle");
    harness.advance(secs(1.0));
    let status = |harness: &Harness| harness.text("find-status").unwrap_or_default();
    assert_eq!(status(&harness), "3/3");
    let steps: [(Key, Modifiers, &str); 5] = [
        (Key::Named(NamedKey::F3), Modifiers::empty(), "2/3"),
        (Key::Named(NamedKey::F3), Modifiers::SHIFT, "3/3"),
        (Key::Character("G".into()), ctrl_shift, "2/3"),
        (Key::Named(NamedKey::Enter), Modifiers::SHIFT, "3/3"),
        (Key::Named(NamedKey::Enter), Modifiers::empty(), "2/3"),
    ];
    for (key, modifiers, expected) in steps {
        let label = format!("{modifiers:?} {key:?}");
        harness.key_down_with(key, modifiers);
        harness.advance(secs(1.0));
        assert_eq!(status(&harness), expected, "{label}");
    }
    assert_eq!(pty.take(), "", "查找时一个键也没漏给终端");
}

/// 提示写的是这个平台的按法：侧栏、命令面板、设置页的快捷键列表都从同一张表来；Windows、Linux 上哪儿都没有 ⌘
#[test]
fn shortcut_hints_show_this_platforms_keys() {
    let (harness, workspace) = mounted();
    let tree = |harness: &Harness| format!("{:?}", harness.element_tree());
    let sidebar = tree(&harness);
    for shortcut in [Shortcut::NewTab, Shortcut::Palette, Shortcut::Settings] {
        assert!(sidebar.contains(shortcut.keys()), "侧栏里有 {shortcut:?}");
    }
    workspace.palette_open.set(true);
    harness.advance(secs(1.0));
    let palette = tree(&harness);
    for shortcut in [Shortcut::SplitColumns, Shortcut::SplitRows, Shortcut::Find] {
        assert!(
            palette.contains(shortcut.keys()),
            "命令面板里有 {shortcut:?}"
        );
    }
    workspace.palette_open.set(false);
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Shortcuts);
    harness.advance(secs(1.0));
    let page = tree(&harness);
    for shortcut in [
        Shortcut::SplitColumns,
        Shortcut::SplitRows,
        Shortcut::FindOlder,
        Shortcut::FindNewer,
        Shortcut::Copy,
        Shortcut::Paste,
    ] {
        assert!(page.contains(shortcut.keys()), "设置页里有 {shortcut:?}");
    }
    if !Shortcut::MAC {
        assert!(
            !sidebar.contains('⌘') && !palette.contains('⌘') && !page.contains('⌘'),
            "Windows、Linux 上不出现 ⌘"
        );
    }
}
