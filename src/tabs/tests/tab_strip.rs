//! 标签和侧栏：打开、关闭、切换、拖动排序、右键菜单、窗口布局。

use super::*;

/// 新终端默认叫「终端」，不是 shell 的名字
#[test]
fn a_new_terminal_is_named_terminal_by_default() {
    let (harness, workspace) = mounted();
    assert_eq!(ids(&workspace), [1]);
    assert_eq!(workspace.active.get_untracked(), 1);
    assert_eq!(harness.text("tab-1").as_deref(), Some("终端"));
    assert_eq!(harness.text("session-1").as_deref(), Some("终端"));
    assert!(harness.bounds("grid").is_some(), "当前标签的终端要挂着");
}

/// 点标签栏上的元素：新建、选中、关闭
#[test]
fn clicking_the_bar_opens_selects_and_closes() {
    let (harness, workspace) = mounted();
    harness.click_named("new-tab");
    harness.frame();
    assert_eq!(ids(&workspace), [1, 2]);
    harness.click_named("tab-1");
    harness.frame();
    assert_eq!(workspace.active.get_untracked(), 1);
    // 点关闭只关，不能顺带把那个标签选中
    harness.click_named("close-2");
    harness.frame();
    assert_eq!(ids(&workspace), [1]);
    assert_eq!(workspace.active.get_untracked(), 1);
}

/// shell 正常退出（退出码 0）标签自己关掉；出错退出的留着
#[test]
fn a_clean_exit_closes_the_tab_but_a_failure_keeps_it() {
    let (_harness, workspace) = mounted();
    workspace.new_tab();
    let tabs = workspace.tabs.get_untracked();
    (tabs[1].session().hooks_for_test().exited)(0);
    assert_eq!(ids(&workspace), [1]);
    let tab = workspace.tabs.get_untracked()[0].clone();
    tab.session().exited_for_test(2);
    assert_eq!(ids(&workspace), [1], "出错退出不自动关");
}

/// 默认标签名固定是「终端」，程序设的标题不改它；设置里选了「跟随程序的标题」才显示，空标题退回「终端」，
/// 切回去又是固定的
#[test]
fn program_titles_show_on_the_tab_only_when_the_setting_asks_for_it() {
    let (harness, workspace) = mounted();
    let session = workspace.tabs.get_untracked()[0].session().clone();
    session.feed_for_test("\x1b]0;vim main.rs\x07");
    harness.frame();
    assert_eq!(harness.text("tab-1").as_deref(), Some("终端"), "默认不跟");
    workspace.form.title.set(TitleSource::Reported);
    harness.frame();
    assert_eq!(
        harness.text("tab-1").as_deref(),
        Some("vim main.rs"),
        "改设置马上生效，已经设过的标题也算"
    );
    session.feed_for_test("\x1b]0;\x07");
    harness.frame();
    assert_eq!(harness.text("tab-1").as_deref(), Some("终端"));
    session.feed_for_test("\x1b]0;less log\x07");
    harness.frame();
    assert_eq!(harness.text("tab-1").as_deref(), Some("less log"));
    workspace.form.title.set(TitleSource::Fixed);
    harness.frame();
    assert_eq!(harness.text("tab-1").as_deref(), Some("终端"));
}

/// 标题栏的空白处按住再动是拖动窗口；双击交给系统（缩放或最小化）
#[test]
fn empty_title_bar_space_drags_and_double_clicks_like_a_title_bar() {
    let (harness, _workspace) = mounted();
    // 标签后面的空白（标签最宽 200，从红绿灯右边起，到 x = 600 还是空的）
    let (x, y) = (600.0, 10.0);
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 20.0, y);
    assert_eq!(
        harness.window_drag_count(),
        1,
        "空白处按住再动，要交给系统去拖窗口"
    );
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    harness.double_click(x, y);
    assert_eq!(harness.title_bar_double_click_count(), 1);
}

/// 第一个标签要从红绿灯右边开始，不能压在它们上面
#[test]
fn tabs_start_to_the_right_of_the_traffic_lights() {
    let (harness, _workspace) = mounted();
    let tab = harness.bounds("tab-1").expect("标签在");
    assert!(tab.x >= 82.0, "标签从 x = {} 开始，红绿灯占到 82", tab.x);
    let button = harness.bounds("new-tab").expect("新建按钮在");
    assert!(
        button.y >= 0.0 && button.y + button.height <= 40.0,
        "按钮在标题栏的高度里"
    );
}

/// 没有系统窗口按钮的平台自己画最小化、最大化、关闭，而且标签仍然从左边的小边距开始
#[test]
fn platforms_without_system_buttons_draw_their_own() {
    let (harness, _workspace) = mounted_with(aureole::TitleBarInfo {
        height: 40.0,
        ..Default::default()
    });
    let tab = harness.bounds("tab-1").expect("标签在");
    assert!(tab.x < 90.0, "没有红绿灯就不用让那么多：{}", tab.x);
    let (x, y) = (WINDOW_SIZE.0 as f32 - 20.0, 20.0);
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 10.0, y);
    harness.mouse_up(MouseButton::Left);
    assert_eq!(harness.window_drag_count(), 0, "窗口按钮上按住不拖窗口");
}

/// 按在标签上拖不动窗口（标签自己要收点击），双击标签也不会缩放窗口
#[test]
fn tabs_do_not_drag_the_window() {
    let (harness, _workspace) = mounted();
    let [x, y] = harness.bounds("tab-1").expect("标签在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x + 20.0, y);
    harness.mouse_up(MouseButton::Left);
    assert_eq!(harness.window_drag_count(), 0);
    harness.advance(secs(1.0));
    harness.double_click(x, y);
    assert_eq!(harness.title_bar_double_click_count(), 0);
}

/// 标签的尺寸：宽 140、离展开按钮 8px；关闭按钮离上、右、下都是 4px
#[test]
fn tab_geometry_is_tight_and_the_close_button_is_evenly_inset() {
    let (harness, _workspace) = mounted();
    let toggle = harness.bounds("toggle-sidebar").expect("按钮在");
    let tab = harness.bounds("tab-1").expect("标签在");
    let close = harness.bounds("close-1").expect("关闭按钮在");
    assert!(
        (tab.x - (toggle.x + toggle.width) - 8.0).abs() < 0.5,
        "标签离按钮 {}",
        tab.x - (toggle.x + toggle.width)
    );
    assert!((tab.width - 140.0).abs() < 0.5, "标签宽 {}", tab.width);
    let (top, right, bottom) = (
        close.y - tab.y,
        tab.x + tab.width - (close.x + close.width),
        tab.y + tab.height - (close.y + close.height),
    );
    assert!(
        (top - 4.0).abs() < 0.5 && (right - 4.0).abs() < 0.5 && (bottom - 4.0).abs() < 0.5,
        "关闭按钮的边距：上 {top} 右 {right} 下 {bottom}"
    );
}

/// 按住标签向右拖过相邻的标签就换位，拖回去换回来；被拖的标签被选中
#[test]
fn dragging_a_tab_reorders_it() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.frame();
    assert_eq!(bar_ids(&workspace), [1, 2, 3]);
    let [x, y] = harness.bounds("tab-1").expect("标签在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x + 30.0, y);
    harness.mouse_move(x + 90.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert_eq!(bar_ids(&workspace), [2, 1, 3], "向右拖了一格多");
    assert_eq!(workspace.active.get_untracked(), 1, "被拖的标签被选中");
    assert_eq!(harness.window_drag_count(), 0);
    harness.advance(secs(1.0));
    let [x, y] = harness.bounds("tab-1").expect("标签在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 30.0, y);
    harness.mouse_move(x - 90.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert_eq!(bar_ids(&workspace), [1, 2, 3], "往回拖换回来");
}

/// 「目标」下面的会话行和别的侧栏行一样宽（选中、悬停的底色铺满整行），不往里缩
#[test]
fn session_rows_are_as_wide_as_the_other_sidebar_rows() {
    let (harness, _workspace) = mounted();
    let nav = harness.bounds("nav-new").expect("新建会话行在");
    let session = harness.bounds("session-1").expect("会话行在");
    assert!(
        (session.x - nav.x).abs() < 0.5 && (session.width - nav.width).abs() < 0.5,
        "会话行 {session:?} 和侧栏行 {nav:?} 一样宽"
    );
}

/// 用键盘切标签（⌘数字、Ctrl+Tab）后，终端拿到焦点，但不在它外面画焦点环
#[test]
fn switching_tabs_from_the_keyboard_draws_no_focus_ring_on_the_terminal() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    let ring_drawn = || {
        harness
            .quads()
            .iter()
            .any(|quad| quad.border_width > 0.0 && quad.fill.is_transparent())
    };
    command(&harness, "1");
    harness.advance(secs(1.0));
    assert_eq!(workspace.active.get_untracked(), 1);
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("grid"),
        "终端拿到焦点，直接能打字"
    );
    assert!(!ring_drawn(), "⌘数字切过去，终端外面没有框");
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    harness.advance(secs(1.0));
    assert_eq!(workspace.active.get_untracked(), 2);
    assert_eq!(harness.focused_name().as_deref(), Some("grid"));
    assert!(!ring_drawn(), "Ctrl+Tab 切过去，终端外面没有框");
}

/// 顶部的拖动只改顶部：侧栏的会话行还是开的先后，不跟着动；快捷键和关闭后换到的邻居按标签条走
#[test]
fn the_order_of_the_top_bar_is_independent_of_the_sidebar() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    // 把 1 拖到最右边：标签条 [2, 3, 1]
    let stride =
        harness.bounds("tab-2").expect("标签在").x - harness.bounds("tab-1").expect("标签在").x;
    let [x, y] = harness.bounds("tab-1").expect("标签在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x + 30.0, y);
    harness.mouse_move(x + 2.0 * stride, y);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 3, 1]);
    assert_eq!(
        bar_order(&harness, &workspace),
        [2, 3, 1],
        "标签条上从左到右"
    );
    assert_eq!(ids(&workspace), [1, 2, 3], "会话的先后没变");
    assert_eq!(
        sidebar_order(&harness, &workspace),
        [1, 2, 3],
        "侧栏的行一个都没挪"
    );
    // 新开的接在两边的最后
    workspace.new_tab();
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 3, 1, 4]);
    assert_eq!(sidebar_order(&harness, &workspace), [1, 2, 3, 4]);
    // ⌘1 是标签条上的第一个，⌘9 是最后一个，Ctrl+Tab 往标签条的右边切
    command(&harness, "1");
    assert_eq!(workspace.active.get_untracked(), 2);
    command(&harness, "9");
    assert_eq!(workspace.active.get_untracked(), 4);
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    assert_eq!(
        workspace.active.get_untracked(),
        2,
        "标签条上最后一个再往右，绕回第一个"
    );
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    assert_eq!(workspace.active.get_untracked(), 3);
    // 关掉当前的 3：换到标签条上它右边的 1（不是侧栏里它下面的 4）
    command(&harness, "w");
    assert_eq!(workspace.active.get_untracked(), 1);
    assert_eq!(bar_ids(&workspace), [2, 1, 4]);
    assert_eq!(
        sidebar_order(&harness, &workspace),
        [1, 2, 4],
        "关掉的行没了，别的不动"
    );
}

/// 拖到标签条的两头：落在头尾的格子里；一直拖到标签条外面再松手就取消，顺序不变、标签落回原处
#[test]
fn dragging_past_the_ends_drops_there_and_dragging_out_cancels() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    let drag_to = |harness: &Harness, name: &str, target_x: f32| {
        let [x, y] = harness.bounds(name).expect("标签在").center();
        harness.mouse_down(x, y, MouseButton::Left);
        for step in 1..=20 {
            harness.mouse_move(x + (target_x - x) * step as f32 / 20.0, y);
            harness.advance(ms(16));
        }
    };
    // 拖到最右边的标签上再松手：换到最后一格
    let last = harness.bounds("tab-3").expect("标签在");
    drag_to(&harness, "tab-1", last.x + last.width - 10.0);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 3, 1]);
    // 拖到最左边：换到第一格
    let first = harness.bounds("tab-2").expect("标签在");
    drag_to(&harness, "tab-1", first.x + 10.0);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [1, 2, 3]);
    // 一直拖到窗口中间（标签条外面）：标签不出标签条，停在最右边那一格；按 Esc 取消，顺序不变、落回原处
    let before = harness.window_bounds("tab-2").expect("在");
    drag_to(&harness, "tab-2", 600.0);
    harness.advance(secs(1.0));
    let held = harness.window_bounds("tab-2").expect("拖着的标签还看得见");
    let strip_right = before.x + 2.0 * 142.0 + 140.0 - 142.0;
    assert!(
        held.x + held.width <= strip_right + 1.0 && held.x >= before.x + 100.0,
        "拖着的标签没有出标签条、停在最右边：{held:?}"
    );
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [1, 2, 3], "Esc 取消不换顺序");
    let after = harness.window_bounds("tab-2").expect("在");
    assert!(
        (after.x - before.x).abs() < 1.0,
        "标签落回原来的格子：{before:?} → {after:?}"
    );
    // 同样拖到窗口中间松手：算放下，换到最右边那一格（手偏出标签条也不会变成取消）
    drag_to(&harness, "tab-2", 600.0);
    harness.advance(secs(1.0));
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [1, 3, 2]);
}

/// 换格按被拖的标签自己的位置算：它的边越过相邻标签的一半才换，和指针在标签的哪里无关
/// （抓在左边、中间、右边，标签走了差不多半格都是同一个时刻换）；手上下偏出标签条也一样
#[test]
fn the_swap_follows_the_dragged_tab_not_the_pointer() {
    let swapped_after = |grab: f32, direction: f32, wander_y: f32| -> f32 {
        let (harness, workspace) = mounted();
        workspace.new_tab();
        workspace.new_tab();
        harness.frame();
        harness.advance(secs(1.0));
        let name = if direction > 0.0 { "tab-1" } else { "tab-3" };
        let neighbour_before = harness.window_bounds("tab-2").expect("在").x;
        let tab = harness.window_bounds(name).expect("在");
        let (x, y) = (tab.x + grab, tab.y + 14.0);
        harness.mouse_down(x, y, MouseButton::Left);
        let mut at = f32::NAN;
        for step in 1..=160 {
            let dx = direction * step as f32;
            harness.mouse_move(x + dx, y + wander_y * (step as f32 / 160.0));
            harness.advance(ms(40));
            let neighbour = harness.window_bounds("tab-2").expect("在").x;
            if (neighbour - neighbour_before).abs() > 70.0 {
                at = dx.abs();
                break;
            }
        }
        harness.mouse_up(MouseButton::Left);
        at
    };
    for grab in [10.0, 70.0, 130.0] {
        for direction in [1.0, -1.0] {
            let at = swapped_after(grab, direction, 0.0);
            assert!(
                (at - 72.0).abs() <= 3.0,
                "抓在 {grab} 处往{}拖：标签走了 {at} 才换，应该是半格（71）上下",
                if direction > 0.0 { "右" } else { "左" }
            );
        }
    }
    // 手一边拖一边往下偏出标签条 60 像素，换格的时机不变
    let at = swapped_after(70.0, 1.0, 60.0);
    assert!((at - 72.0).abs() <= 3.0, "手偏出标签条也一样：{at}");
}

/// 拖标签时手偏出了标签条（往下掉到终端里）再松手：照样换顺序，不会变成取消
#[test]
fn releasing_below_the_tab_strip_still_drops_the_tab() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    let tab = harness.window_bounds("tab-1").expect("在");
    let [x, y] = tab.center();
    harness.mouse_down(x, y, MouseButton::Left);
    for step in 1..=30 {
        harness.mouse_move(x + 6.0 * step as f32, y + 8.0 * step as f32);
        harness.advance(ms(16));
    }
    harness.advance(ms(300));
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 1], "在终端上方松手也换了");
    assert_eq!(workspace.active.get_untracked(), 1);
}

/// 标签多到放不下：标签保持 140 宽、滚动着看，竖向滚轮翻成横向；最右边仍留着空白可以拖窗口
#[test]
fn many_tabs_scroll_and_a_drag_area_stays_on_the_right() {
    let (harness, workspace) = mounted();
    for _ in 0..14 {
        workspace.new_tab();
    }
    harness.frame();
    harness.advance(secs(1.0));
    let tab = harness.bounds("tab-1").expect("标签在");
    assert!(
        (tab.width - 140.0).abs() < 0.5,
        "标签挤满了也不压窄：{}",
        tab.width
    );
    let plus = harness.bounds("new-tab").expect("新建按钮在");
    assert!(
        plus.x + plus.width + 64.0 <= WINDOW_SIZE.0 as f32,
        "新建按钮右边 {} 后面要给拖动区留 64",
        plus.x + plus.width
    );
    // 最后一个标签是当前的，已经滚到看得见
    let last = harness.window_bounds("tab-15").expect("最后的标签在");
    assert!(
        last.x >= 0.0 && last.x + last.width <= plus.x + 0.5,
        "当前标签滚进视口：{last:?}"
    );
    // 竖向滚轮向上（往回）：标签往右走
    harness.wheel(300.0, 20.0, 0.0, -5.0);
    harness.advance(secs(1.0));
    let moved = harness.window_bounds("tab-15").expect("最后的标签在");
    assert!(
        moved.x > last.x + 20.0,
        "滚轮把标签翻回去了：{} → {}",
        last.x,
        moved.x
    );
    // 右边的空白拖动窗口
    let (x, y) = (plus.x + plus.width + 20.0, 10.0);
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 10.0, y);
    assert_eq!(harness.window_drag_count(), 1, "新建按钮后面的空白能拖窗口");
    harness.mouse_up(MouseButton::Left);
}

/// 会话多到侧栏放不下：中间一段滚动，每一行保持原来的高度，不被挤扁，顶上的开关也不动
#[test]
fn many_sessions_scroll_in_the_sidebar_instead_of_squeezing() {
    let (harness, workspace) = mounted();
    let before = (
        harness.bounds("nav-new").expect("在").height,
        harness.bounds("session-1").expect("在").height,
        harness.bounds("nav-search").expect("在").height,
    );
    for _ in 0..30 {
        workspace.new_tab();
    }
    harness.frame();
    harness.advance(secs(1.0));
    let after = (
        harness.bounds("nav-new").expect("在").height,
        harness.bounds("session-1").expect("在").height,
        harness.bounds("nav-search").expect("在").height,
    );
    assert_eq!(before, after, "行高不变");
    let sidebar = harness.bounds("sidebar").expect("在");
    let last = harness.window_bounds("session-31").expect("最后一个会话在");
    assert!(
        last.y + last.height <= sidebar.y + sidebar.height + 0.5,
        "当前会话滚进了侧栏：{last:?}"
    );
    // 滚轮能滚到最前面的行
    harness.wheel(60.0, 300.0, 0.0, -80.0);
    harness.advance(secs(1.0));
    let first = harness.window_bounds("nav-new").expect("在");
    assert!(
        first.y >= sidebar.y && first.y + first.height <= sidebar.y + sidebar.height,
        "滚回顶上：{first:?}"
    );
}

/// 侧栏和卡片的交界处：指针变成左右箭头，按住左右拖改侧栏宽度，有上下限；收起时没有拖动条
#[test]
fn dragging_the_sidebar_edge_resizes_it() {
    let (harness, workspace) = mounted();
    // 拖动条在侧栏右边缘里面 4 像素的地方
    let width = harness.bounds("sidebar").expect("侧栏在").width;
    let edge = width - 4.0;
    harness.mouse_move(edge, 300.0);
    assert_eq!(harness.cursor(), Cursor::ResizeHorizontal);
    harness.mouse_down(edge, 300.0, MouseButton::Left);
    harness.mouse_move(edge + 60.0, 300.0);
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert!(
        (workspace.sidebar_width.get_untracked() - (width + 60.0)).abs() < 1.0,
        "宽度跟着指针走：{}",
        workspace.sidebar_width.get_untracked()
    );
    assert_eq!(harness.window_drag_count(), 0, "拖动条上不能拖窗口");
    harness.mouse_down(edge + 60.0, 300.0, MouseButton::Left);
    harness.mouse_move(2000.0, 300.0);
    harness.mouse_up(MouseButton::Left);
    assert!(workspace.sidebar_width.get_untracked() <= 360.0, "有上限");
    harness.mouse_down(
        workspace.sidebar_width.get_untracked() - 4.0,
        300.0,
        MouseButton::Left,
    );
    harness.mouse_move(0.0, 300.0);
    harness.mouse_up(MouseButton::Left);
    assert!(workspace.sidebar_width.get_untracked() >= 176.0, "有下限");
    command(&harness, "b");
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("sidebar-resize").is_none(),
        "收起时没有拖动条"
    );
}

/// 后台标签有了新输出：「需要我」亮起来，点它跳过去，标记清掉；前台标签的输出不算
#[test]
fn background_output_marks_the_tab_until_it_is_looked_at() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    let tabs = workspace.tabs.get_untracked();
    (tabs[1].session().hooks_for_test().activity)();
    assert!(
        workspace.unseen.get_untracked().is_empty(),
        "前台标签的输出不算"
    );
    (tabs[0].session().hooks_for_test().activity)();
    assert_eq!(workspace.unseen.get_untracked().len(), 1);
    harness.frame();
    assert!(harness.bounds("needs-me").is_none(), "「需要我」已经去掉");
    click(&harness, "tab-1");
    assert_eq!(workspace.active.get_untracked(), 1);
    assert!(
        workspace.unseen.get_untracked().is_empty(),
        "看过了标记就清掉"
    );
}

/// 窗口缩小：卡片和终端要跟着缩进窗口里（以前内容把卡片撑出了窗口，右边和下边都看不到），行列数跟着变少
#[test]
fn shrinking_the_window_keeps_the_card_and_terminal_inside() {
    let (harness, workspace) = mounted();
    let (columns, rows) = workspace.size.get_untracked();
    harness.resize(720.0, 460.0);
    harness.frame();
    harness.advance(secs(1.0));
    let card = harness.bounds("card").expect("卡片在");
    assert!(
        card.x + card.width <= 720.0 + 0.5,
        "卡片右边 {} 超出窗口 720",
        card.x + card.width
    );
    assert!(
        card.y + card.height <= 460.0 + 0.5,
        "卡片下边 {} 超出窗口 460",
        card.y + card.height
    );
    let grid = harness.bounds("grid").expect("终端在");
    assert!(
        grid.x + grid.width <= card.x + card.width + 0.5
            && grid.y + grid.height <= card.y + card.height + 0.5,
        "终端要在卡片里面"
    );
    let (new_columns, new_rows) = workspace.size.get_untracked();
    assert!(
        new_columns < columns && new_rows < rows,
        "行列数跟着变少：{columns}×{rows} → {new_columns}×{new_rows}"
    );
}

/// 收起侧栏：不能露出快捷键提示，宽度窄到只放得下图标
#[test]
fn collapsed_sidebar_shows_only_icons() {
    let (harness, workspace) = mounted();
    let shown = |harness: &Harness| format!("{:?}", harness.element_tree());
    assert!(
        shown(&harness).contains("⌘T") && shown(&harness).contains("⌘K"),
        "展开时有快捷键提示"
    );
    command(&harness, "b");
    harness.advance(secs(1.0));
    let collapsed = shown(&harness);
    assert!(
        !collapsed.contains("⌘T") && !collapsed.contains("⌘K"),
        "收起后快捷键提示不能露出来"
    );
    assert!(
        !collapsed.contains("新建会话")
            || harness.text("nav-new").as_deref().unwrap_or("").is_empty(),
        "收起后文字也不留"
    );
    let sidebar = harness.bounds("sidebar").expect("侧栏在").width;
    assert!(
        sidebar <= 50.0,
        "收起后的宽度 {sidebar}，只放图标不用这么宽"
    );
    // 后台有新输出时，收起后「需要我」也要看得出来（一个小点），不能因为文字没了就没有提示
    workspace.new_tab();
    (workspace.tabs.get_untracked()[0]
        .session()
        .hooks_for_test()
        .activity)();
    harness.frame();
    assert_eq!(workspace.unseen.get_untracked().len(), 1);
}

/// 右键点标签、侧栏的会话行都弹出菜单，点「删除」关掉那个会话；右键本身不切换当前会话
#[test]
fn the_context_menu_on_a_tab_or_a_session_row_deletes_it() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.advance(secs(1.0));
    workspace.select(3);
    harness.advance(secs(1.0));
    assert!(harness.bounds("menu-tab-1").is_none(), "没右键时没有菜单");
    right_click_named(&harness, "tab-1");
    assert!(harness.bounds("menu-tab-1").is_some(), "右键点标签弹出菜单");
    assert_eq!(workspace.active.get_untracked(), 3, "右键不切换当前标签");
    click_menu(&harness, "menu-delete-tab-1");
    assert_eq!(ids(&workspace), [2, 3]);
    assert!(harness.bounds("menu-tab-1").is_none(), "删完菜单也没了");
    right_click_named(&harness, "session-2");
    assert!(
        harness.bounds("menu-session-2").is_some(),
        "右键点侧栏的会话行也弹出菜单"
    );
    click_menu(&harness, "menu-delete-session-2");
    assert_eq!(ids(&workspace), [3]);
}

/// 菜单里的「重命名」打开重命名框，回车后标签、侧栏、状态条都是新名字；程序之后设的标题不再覆盖它；留空退回程序设的标题
#[test]
fn renaming_from_the_menu_sticks_until_it_is_cleared() {
    let (harness, workspace) = mounted();
    workspace.form.title.set(TitleSource::Reported);
    let session = workspace.tabs.get_untracked()[0].session().clone();
    session.feed_for_test("\x1b]0;vim main.rs\x07");
    harness.frame();
    right_click_named(&harness, "tab-1");
    click_menu(&harness, "menu-rename-tab-1");
    assert_eq!(workspace.renaming.get_untracked(), Some(1));
    assert!(harness.bounds("rename").is_some(), "重命名框开着");
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("rename-field"),
        "焦点在输入框里"
    );
    assert!(
        harness
            .text_state("rename-field")
            .unwrap_or_default()
            .contains("vim main.rs"),
        "框里是现在的名字"
    );
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("  构建  ");
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert_eq!(workspace.renaming.get_untracked(), None);
    assert!(harness.bounds("rename").is_none(), "回车后框关掉");
    for name in ["tab-1", "session-1"] {
        assert_eq!(
            harness.text(name).as_deref(),
            Some("构建"),
            "{name} 显示新名字（首尾空白去掉）"
        );
    }
    session.feed_for_test("\x1b]0;less log\x07");
    harness.frame();
    assert_eq!(
        harness.text("tab-1").as_deref(),
        Some("构建"),
        "程序设的标题不盖用户起的名字"
    );
    // 从侧栏的菜单再改一次，留空：退回程序最近设的标题
    right_click_named(&harness, "session-1");
    click_menu(&harness, "menu-rename-session-1");
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.key_down(NamedKey::Backspace);
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    for name in ["tab-1", "session-1"] {
        assert_eq!(
            harness.text(name).as_deref(),
            Some("less log"),
            "{name} 退回程序设的标题"
        );
    }
}

/// Esc、点外面都只是关掉菜单和重命名框，什么也不改
#[test]
fn escape_and_outside_clicks_close_the_menu_and_the_rename_box_without_changes() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    right_click_named(&harness, "tab-1");
    assert!(harness.bounds("menu-tab-1").is_some());
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    assert!(harness.bounds("menu-tab-1").is_none(), "Esc 关菜单");
    right_click_named(&harness, "session-1");
    assert!(harness.bounds("menu-session-1").is_some());
    harness.click(600.0, 300.0);
    harness.advance(secs(1.0));
    assert!(harness.bounds("menu-session-1").is_none(), "点外面关菜单");
    assert_eq!(ids(&workspace), [1, 2], "没有删任何会话");
    right_click_named(&harness, "tab-2");
    click_menu(&harness, "menu-rename-tab-2");
    harness.type_text("别的");
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    assert_eq!(workspace.renaming.get_untracked(), None);
    assert!(harness.bounds("rename").is_none(), "Esc 关重命名框");
    assert_eq!(harness.text("tab-2").as_deref(), Some("终端"), "取消不改名");
}

/// 有程序在前台跑的标签，关闭前先问（设置里可以关掉这个确认）；shell 在等输入时直接关
#[test]
fn closing_a_busy_terminal_asks_first() {
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    workspace.new_tab();
    harness.advance(secs(1.0));
    click(&harness, "close-1");
    assert_eq!(ids(&workspace), [2], "没有程序在跑：直接关");
    fake.pty(1).set_busy(true);
    click(&harness, "close-2");
    assert_eq!(ids(&workspace), [2], "有程序在跑：先问");
    assert_eq!(workspace.closing.get_untracked(), Some(2));
    assert!(harness.bounds("close-dialog").is_some());
    click_menu(&harness, "close-cancel");
    assert_eq!(ids(&workspace), [2]);
    assert!(harness.bounds("close-dialog").is_none());
    command(&harness, "w");
    assert_eq!(workspace.closing.get_untracked(), Some(2), "⌘W 也一样");
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    assert_eq!(ids(&workspace), [2], "Esc 不关");
    right_click_named(&harness, "tab-2");
    click_menu(&harness, "menu-delete-tab-2");
    assert!(harness.bounds("close-dialog").is_some(), "右键删除也问");
    click_menu(&harness, "close-confirm");
    assert!(ids(&workspace).is_empty(), "确认了才关");
    assert_eq!(workspace.closing.get_untracked(), None);
    // 关掉确认以后直接关
    workspace.new_tab();
    fake.pty(2).set_busy(true);
    workspace.form.confirm_close.set(false);
    workspace.close_active();
    assert!(ids(&workspace).is_empty());
}

/// 拖标签换位置：拖的过程中标签条的顺序不变（不重新布局、不重建标签，只有平移），
/// 当前标签也不在拖的中途切换（切换会把终端卸下再挂上）；松手才换顺序、选中被拖的，只通知一次
#[test]
fn dragging_a_tab_only_translates_until_it_is_dropped() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    let changes = Rc::new(Cell::new(0u32));
    let counter = changes.clone();
    let bar = workspace.bar;
    let _watcher = Effect::new(move || {
        let _ = bar.get();
        counter.set(counter.get() + 1);
    });
    let baseline = changes.get();
    let before = (
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("tab-2").expect("在"),
    );
    let [x, y] = before.0.center();
    harness.mouse_down(x, y, MouseButton::Left);
    for step in 1..=24 {
        harness.mouse_move(x + 5.0 * step as f32, y);
        harness.advance(ms(16));
        assert_eq!(bar_ids(&workspace), [1, 2, 3], "拖的过程中顺序不变");
        assert_eq!(changes.get(), baseline, "标签条的列表一次也没有重算");
        assert_eq!(workspace.active.get_untracked(), 3, "当前标签没有中途切换");
    }
    harness.advance(secs(1.0));
    let (dragged, neighbour) = (
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("tab-2").expect("在"),
    );
    assert!(
        dragged.x > before.0.x + 100.0,
        "被拖的跟着指针：{} → {}",
        before.0.x,
        dragged.x
    );
    assert!(
        neighbour.x < before.1.x - 100.0,
        "被越过的标签让出了一格：{} → {}",
        before.1.x,
        neighbour.x
    );
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 1, 3]);
    assert_eq!(changes.get(), baseline + 1, "松手时只换了一次");
    assert_eq!(workspace.active.get_untracked(), 1);
    let (settled, other) = (
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("tab-2").expect("在"),
    );
    assert!(
        (other.x - before.0.x).abs() < 1.0 && (settled.x - before.1.x).abs() < 1.0,
        "落进了各自新的格子：{settled:?} {other:?}"
    );
}

/// 把标签拖回原来的格子再松手，或者拖了一点没过相邻的：顺序不变
#[test]
fn a_short_drag_keeps_the_order() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    let [x, y] = harness.bounds("tab-1").expect("在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x + 20.0, y);
    harness.mouse_move(x + 30.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [1, 2]);
    assert_eq!(workspace.active.get_untracked(), 1);
    let (a, b) = (
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("tab-2").expect("在"),
    );
    assert!(b.x - a.x > 140.0, "各回各的格子：{a:?} {b:?}");
}

/// 目标下面的空白处右键也能创建会话：不归任何文件夹，直接列在「目标」下；当前在文件夹里也一样
#[test]
fn the_blank_area_of_the_targets_creates_a_terminal() {
    let (harness, workspace, fake) = blank(Store::none());
    let folder = workspace.create_folder("工作");
    workspace.new_tab_in(folder);
    harness.advance(secs(1.0));
    right_click_named(&harness, "targets-blank");
    click_menu(&harness, "menu-new-session");
    harness.advance(secs(1.0));
    assert_eq!(ids(&workspace), [1, 2]);
    let tab = workspace.tabs.get_untracked()[1].clone();
    assert_eq!(tab.folder, None, "不归文件夹，即使当前在文件夹里");
    assert_eq!(workspace.active.get_untracked(), 2);
    assert_eq!(fake.launches.borrow().len(), 2);
    assert!(harness.bounds("session-2").is_some());
}

/// 拖标签只左右动（上下不跟着指针走）；松手时从指针所在的位置平滑落进新的格子，不是先跳回原来的位置再滑过去
#[test]
fn a_dragged_tab_moves_only_sideways_and_lands_from_where_it_was_released() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    let start = harness.window_bounds("tab-1").expect("在");
    let [x, y] = start.center();
    harness.mouse_down(x, y, MouseButton::Left);
    // 斜着拖：横向走了一格半，纵向也走了 12 像素
    for step in 1..=20 {
        harness.mouse_move(x + 10.0 * step as f32, y + 0.6 * step as f32);
        harness.advance(ms(16));
    }
    harness.advance(ms(100));
    let held = harness.window_bounds("tab-1").expect("在");
    assert!(held.x > start.x + 150.0, "横向跟着走：{start:?} → {held:?}");
    assert!(
        (held.y - start.y).abs() < 1.0,
        "纵向不动：{} → {}",
        start.y,
        held.y
    );
    // 松手：下一帧还在松手时的位置附近（从那里往新格子落），不是跳回原来的格子
    harness.mouse_up(MouseButton::Left);
    harness.advance(ms(16));
    let landing = harness.window_bounds("tab-1").expect("在");
    assert!(
        (landing.x - held.x).abs() < 60.0,
        "松手后从松手的位置落下去，没有先跳一下：{} → {}",
        held.x,
        landing.x
    );
    assert!((landing.y - start.y).abs() < 2.0, "落的过程中纵向也不动");
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [2, 1, 3]);
    let settled = harness.window_bounds("tab-1").expect("在");
    assert!((settled.x - (start.x + 142.0)).abs() < 1.0 && (settled.y - start.y).abs() < 0.5);
}

/// 当前是标签 2，拖标签 1 越过它换位置、松手：选中的样式从 2 直接换到 1，
/// 标签 2 不能在松手那一刻闪回选中（白底）再淡出
#[test]
fn dropping_a_tab_does_not_flash_the_previously_active_tab() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(workspace.active.get_untracked(), 2);
    let [x, y] = harness.window_bounds("tab-1").expect("在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    for step in 1..=20 {
        harness.mouse_move(x + 9.0 * step as f32, y);
        harness.advance(ms(16));
    }
    harness.advance(ms(300));
    // 标签 2 现在不是白底（拖动中只有被拖的是白底）
    let white = |harness: &Harness, name: &str| {
        let bounds = harness.window_bounds(name).expect("在");
        let shot = harness.screenshot();
        shot.pixel(
            (bounds.x + 3.0) as u32,
            (bounds.y + bounds.height / 2.0) as u32,
        )
        .0[0]
            > 253
    };
    assert!(!white(&harness, "tab-2"), "拖动中标签 2 不是选中的样子");
    harness.mouse_up(MouseButton::Left);
    for frame in 0..30 {
        harness.advance(ms(16));
        assert!(
            !white(&harness, "tab-2"),
            "松手后第 {frame} 帧：标签 2 闪回了选中的样子"
        );
    }
    assert_eq!(bar_ids(&workspace), [2, 1]);
    assert_eq!(workspace.active.get_untracked(), 1);
    assert!(white(&harness, "tab-1"), "最后标签 1 是选中的");
}

/// 从命令面板里新建的终端：面板一关，它的作用域就释放了。会话里的信号要是记在面板的作用域上，
/// 之后一读就是「使用了已释放的信号」，在鼠标事件的回调里 panic 就直接 abort（窗口崩溃）。
/// 会话比任何界面都活得久，要在面板关了以后还能响铃、闪、重画
#[test]
fn a_terminal_opened_from_the_palette_outlives_the_palette() {
    let (harness, workspace) = mounted();
    command(&harness, "k");
    let panel = harness.bounds("palette").expect("面板在");
    let row = harness.bounds("cmd-new-tab").expect("命令在");
    let origin = (
        (WINDOW_SIZE.0 as f32 - panel.width) / 2.0,
        (WINDOW_SIZE.1 as f32 - panel.height) / 2.0,
    );
    let [x, y] = row.center();
    harness.click(origin.0 + x, origin.1 + y);
    harness.advance(secs(1.0));
    assert!(harness.bounds("palette").is_none(), "面板关了");
    assert_eq!(ids(&workspace), [1, 2]);
    let session = workspace.tabs.get_untracked()[1].session();
    workspace.form.bell.set(crate::settings::Bell::Flash);
    session.feed_for_test("\x07");
    harness.advance(secs(1.0));
    assert_eq!(session.flashes_for_test(), 1);
    // 从右键菜单、重命名框这些短命的界面里点出来的也一样
    workspace.select(1);
    workspace.new_tab();
    harness.advance(secs(1.0));
    assert!(harness.bounds("grid").is_some());
}
