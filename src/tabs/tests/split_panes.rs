//! 拆分：聚焦、关闭、拖分隔条、右键菜单。

use super::*;

/// 点「左右分」：聚焦的终端拆成左右两个，新的在右边、从原来那个现在的目录起、拿到焦点；点「上下分」新的在下面；
/// 可以接着拆（拆出来的再拆）
#[test]
fn the_split_buttons_split_the_focused_terminal() {
    let project = directory("split");
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    fake.pty(0).set_directory(&project);
    harness.advance(secs(1.0));
    assert_eq!(
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len(),
        1
    );
    assert!(
        harness.bounds("pane-divider-columns").is_none(),
        "没拆时没有分隔条"
    );
    click(&harness, "split-columns");
    let tab = workspace.tabs.get_untracked()[0].clone();
    let panes = tab.layout.get_untracked().leaves();
    assert_eq!(panes.len(), 2);
    assert_eq!(ids(&workspace), [1], "还是一个标签");
    assert_eq!(
        fake.launch(1).directory,
        Some(project.clone()),
        "新终端从聚焦的那个现在的目录起"
    );
    assert_eq!(tab.focus.get_untracked(), panes[1].0, "聚焦到新的");
    let (left, right) = (
        harness
            .window_bounds(&format!("pane-{}", panes[0].0))
            .expect("左边在"),
        harness
            .window_bounds(&format!("pane-{}", panes[1].0))
            .expect("右边在"),
    );
    assert!(
        right.x > left.x + left.width - 0.5,
        "新的在右边：{left:?} {right:?}"
    );
    assert!((left.y - right.y).abs() < 0.5 && (left.height - right.height).abs() < 0.5);
    assert!(
        (left.width - right.width).abs() < 8.0,
        "各一半：{left:?} {right:?}"
    );
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("grid"),
        "新终端拿到键盘焦点"
    );
    // 再把右边的上下分
    click(&harness, "split-rows");
    let panes = tab.layout.get_untracked().leaves();
    assert_eq!(panes.len(), 3);
    let (top, bottom) = (
        harness
            .window_bounds(&format!("pane-{}", panes[1].0))
            .expect("上在"),
        harness
            .window_bounds(&format!("pane-{}", panes[2].0))
            .expect("下在"),
    );
    assert!(
        bottom.y > top.y + top.height - 0.5 && (top.x - bottom.x).abs() < 0.5,
        "新的在下面：{top:?} {bottom:?}"
    );
    assert_eq!(fake.launches.borrow().len(), 3);
    let _ = std::fs::remove_dir_all(&project);
}

/// ⌘D 左右分、⌘⇧D 上下分；命令面板里也有；设置页开着、没有标签时不拆
#[test]
fn split_shortcuts_and_the_palette_work() {
    let (harness, workspace, _fake) = blank(Store::none());
    command(&harness, "d");
    assert!(ids(&workspace).is_empty(), "没有标签不拆");
    workspace.new_tab();
    command(&harness, "d");
    harness.key_down_with(
        Key::Character("D".into()),
        Modifiers::SUPER | Modifiers::SHIFT,
    );
    harness.frame();
    let layout = workspace.tabs.get_untracked()[0].layout.get_untracked();
    assert_eq!(layout.leaves().len(), 3);
    assert!(harness.bounds("pane-divider-columns").is_some());
    assert!(harness.bounds("pane-divider-rows").is_some());
    workspace.open_settings();
    command(&harness, "d");
    assert_eq!(
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len(),
        3,
        "设置页里不拆"
    );
    workspace.close_settings();
    command(&harness, "k");
    let panel = harness.bounds("palette").expect("面板在");
    let row = harness.bounds("cmd-split-rows").expect("命令在");
    let origin = (
        (WINDOW_SIZE.0 as f32 - panel.width) / 2.0,
        (WINDOW_SIZE.1 as f32 - panel.height) / 2.0,
    );
    let [x, y] = row.center();
    harness.click(origin.0 + x, origin.1 + y);
    harness.advance(secs(1.0));
    assert_eq!(
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len(),
        4,
        "命令面板里也能拆"
    );
}

/// 点哪个终端就聚焦哪个；⌘W 在拆分过的标签里只关聚焦的那个，剩一个时才关标签；
/// 一个终端里的 shell 正常退出只关它自己
#[test]
fn closing_panes_keeps_the_rest_of_the_tab() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    workspace.split(Axis::Columns);
    workspace.split(Axis::Columns);
    harness.advance(secs(1.0));
    let tab = workspace.tabs.get_untracked()[0].clone();
    let panes: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(panes.len(), 3);
    // 点第一个：聚焦它
    click(&harness, &format!("pane-{}", panes[0]));
    assert_eq!(tab.focus.get_untracked(), panes[0]);
    // ⌘W 关聚焦的（第一个），焦点交给它的兄弟
    command(&harness, "w");
    let rest: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(rest, [panes[1], panes[2]]);
    assert_eq!(ids(&workspace), [1], "标签还在");
    assert_eq!(tab.focus.get_untracked(), panes[1]);
    // 第二个里的 shell 正常退出：只关它
    (tab.layout.get_untracked().leaves()[0]
        .1
        .hooks_for_test()
        .exited)(0);
    harness.frame();
    assert_eq!(tab.layout.get_untracked().leaves().len(), 1);
    assert_eq!(ids(&workspace), [1]);
    assert!(
        harness.bounds("pane-divider-columns").is_none(),
        "只剩一个，分隔条没了"
    );
    assert_eq!(tab.focus.get_untracked(), panes[2]);
    // 出错退出的不关
    let _ = fake;
    // 最后一个终端 ⌘W：关标签
    command(&harness, "w");
    assert!(ids(&workspace).is_empty());
}

/// 拆分过的标签：有程序在跑的那个终端要关时先问；关标签时只要有一个在跑就问
#[test]
fn closing_a_busy_pane_asks_first() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    workspace.split(Axis::Rows);
    harness.advance(secs(1.0));
    fake.pty(1).set_busy(true);
    command(&harness, "w");
    assert_eq!(workspace.closing.get_untracked(), Some(1), "问了");
    assert_eq!(
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len(),
        2
    );
    click_menu(&harness, "close-confirm");
    assert_eq!(
        workspace.tabs.get_untracked()[0]
            .layout
            .get_untracked()
            .leaves()
            .len(),
        1,
        "确认后只关那个终端"
    );
    assert_eq!(ids(&workspace), [1]);
    // 关整个标签：别的终端在跑也要问
    workspace.split(Axis::Columns);
    fake.pty(0).set_busy(true);
    workspace.request_close(1);
    assert_eq!(workspace.closing.get_untracked(), Some(1));
    harness.advance(secs(1.0));
    click_menu(&harness, "close-confirm");
    assert!(ids(&workspace).is_empty());
}

/// 拖分隔条改比例（有上下限）；拖的过程中不重建终端（排布的结构没变）
#[test]
fn dragging_the_divider_resizes_the_panes() {
    let (harness, workspace, _fake) = blank(Store::none());
    workspace.new_tab();
    workspace.split(Axis::Columns);
    harness.advance(secs(1.0));
    let tab = workspace.tabs.get_untracked()[0].clone();
    let signature = tab.layout.get_untracked().signature();
    let panes: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let width = |harness: &Harness, pane: u64| {
        harness
            .window_bounds(&format!("pane-{pane}"))
            .expect("在")
            .width
    };
    let (before_left, before_right) = (width(&harness, panes[0]), width(&harness, panes[1]));
    let [x, y] = harness
        .window_bounds("pane-divider-columns")
        .expect("分隔条在")
        .center();
    assert_eq!(harness.cursor(), Cursor::Default);
    harness.mouse_move(x, y);
    assert_eq!(harness.cursor(), Cursor::ResizeHorizontal);
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x + 60.0, y);
    harness.mouse_move(x + 120.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    let (left, right) = (width(&harness, panes[0]), width(&harness, panes[1]));
    assert!(
        (left - (before_left + 120.0)).abs() < 3.0,
        "左边宽了 120：{before_left} → {left}"
    );
    assert!((right - (before_right - 120.0)).abs() < 3.0, "右边窄了 120");
    assert_eq!(
        tab.layout.get_untracked().signature(),
        signature,
        "结构没变，没有重建"
    );
    // 拖过头：有下限
    harness.mouse_down(x + 120.0, y, MouseButton::Left);
    harness.mouse_move(5000.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert!(
        width(&harness, panes[1]) > 40.0,
        "右边留着最小的一块：{}",
        width(&harness, panes[1])
    );
    assert_eq!(harness.window_drag_count(), 0, "分隔条上不拖窗口");
}

/// 终端里右键：弹出菜单（复制、粘贴、全选、清屏、左右分屏、上下分屏）；
/// 没拆分的没有「关闭此分屏」，拆分过的有，点了只关这一个，剩下的补上位置
#[test]
fn the_terminal_context_menu_copies_pastes_splits_and_closes_a_pane() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    harness.advance(secs(1.0));
    let session = workspace.tabs.get_untracked()[0].session();
    let pane = workspace.tabs.get_untracked()[0].focus.get_untracked();
    let right_click_pane = |harness: &Harness, pane: u64| {
        let [x, y] = harness
            .window_bounds(&format!("pane-{pane}"))
            .expect("终端在")
            .center();
        harness.right_click(x, y);
        harness.advance(secs(1.0));
    };
    right_click_pane(&harness, pane);
    assert!(
        harness.bounds(&format!("menu-pane-{pane}")).is_some(),
        "弹出了菜单"
    );
    for item in [
        "paste",
        "select-all",
        "clear",
        "split-columns",
        "split-rows",
    ] {
        assert!(
            harness
                .bounds(&format!("menu-pane-{item}-{pane}"))
                .is_some(),
            "菜单里有 {item}"
        );
    }
    assert!(
        harness.bounds(&format!("menu-pane-copy-{pane}")).is_none(),
        "没选中没有「复制」"
    );
    assert!(
        harness.bounds(&format!("menu-pane-close-{pane}")).is_none(),
        "没拆分没有「关闭此分屏」"
    );
    // 粘贴
    aureole::clipboard::write_text("from the menu");
    click_menu(&harness, &format!("menu-pane-paste-{pane}"));
    assert_eq!(fake.pty(0).take(), "from the menu");
    // 全选之后再右键：有「复制」，点了复制到剪贴板
    session.feed_for_test("hello menu");
    session.select_all();
    right_click_pane(&harness, pane);
    click_menu(&harness, &format!("menu-pane-copy-{pane}"));
    assert!(
        aureole::clipboard::read_text()
            .expect("有")
            .contains("hello menu")
    );
    // 清屏：发 Ctrl-L
    right_click_pane(&harness, pane);
    click_menu(&harness, &format!("menu-pane-clear-{pane}"));
    assert_eq!(fake.pty(0).take(), "\x0c");
    // 从菜单里左右分屏
    right_click_pane(&harness, pane);
    click_menu(&harness, &format!("menu-pane-split-columns-{pane}"));
    let tab = workspace.tabs.get_untracked()[0].clone();
    let panes: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(panes.len(), 2);
    // 拆分过的有「关闭此分屏」：右键第一个，关掉它
    right_click_pane(&harness, panes[0]);
    assert!(
        harness
            .bounds(&format!("menu-pane-close-{}", panes[0]))
            .is_some(),
        "拆分过的有「关闭此分屏」"
    );
    assert_eq!(tab.focus.get_untracked(), panes[0], "右键的那个终端聚焦了");
    click_menu(&harness, &format!("menu-pane-close-{}", panes[0]));
    harness.advance(secs(1.0));
    let rest: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(rest, [panes[1]], "只关了那一个");
    assert_eq!(ids(&workspace), [1], "标签还在");
    assert_eq!(tab.focus.get_untracked(), panes[1]);
    assert!(harness.bounds("pane-divider-columns").is_none());
    // 设置里开了右键粘贴：右键直接粘贴，不弹菜单
    workspace.form.right_click_paste.set(true);
    aureole::clipboard::write_text("direct");
    right_click_pane(&harness, panes[1]);
    assert!(
        harness.bounds(&format!("menu-pane-{}", panes[1])).is_none(),
        "不弹菜单"
    );
    assert!(fake.pty(1).take().contains("direct"), "直接粘贴");
}

/// 拆分过时聚焦的那个终端顶上有一条线；点另一个，线跟着走
#[test]
fn the_focused_pane_has_a_marker_that_follows_the_focus() {
    let (harness, workspace, _fake) = blank(Store::none());
    workspace.new_tab();
    assert!(harness.bounds("pane-focus-1").is_none(), "没拆时没有标记");
    workspace.split(Axis::Columns);
    harness.advance(secs(1.0));
    let tab = workspace.tabs.get_untracked()[0].clone();
    let panes: Vec<u64> = tab
        .layout
        .get_untracked()
        .leaves()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let marker = |harness: &Harness, pane: u64| {
        let bounds = harness
            .window_bounds(&format!("pane-focus-{pane}"))
            .expect("在");
        harness
            .screenshot()
            .pixel(
                (bounds.x + bounds.width / 2.0) as u32,
                (bounds.y + 1.0) as u32,
            )
            .0[0]
    };
    assert!(marker(&harness, panes[1]) < 200, "聚焦的有线");
    assert!(marker(&harness, panes[0]) > 240, "没聚焦的没有");
    click(&harness, &format!("pane-{}", panes[0]));
    assert!(
        marker(&harness, panes[0]) < 200 && marker(&harness, panes[1]) > 240,
        "线跟着走"
    );
}
