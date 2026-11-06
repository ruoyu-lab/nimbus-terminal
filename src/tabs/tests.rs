use std::sync::Arc;

use aureole::MouseButton;
use aureole::testing::Harness;

use super::*;
use crate::pane::{Axis, Pane};
use crate::pty::{Pty, Recorder};

/// 假的起会话：每次都记下是怎么起的，给一个只记录写入的假 pty（可以假装它在某个目录、前台有程序在跑）
#[derive(Clone, Default)]
struct Fake {
    launches: Rc<RefCell<Vec<Launch>>>,
    ptys: Rc<RefCell<Vec<Arc<Recorder>>>>,
}

impl Fake {
    fn spawn(&self) -> SpawnFn {
        let (launches, ptys) = (self.launches.clone(), self.ptys.clone());
        Rc::new(move |launch| {
            let pty = Arc::new(Recorder::default());
            launches.borrow_mut().push(launch.clone());
            ptys.borrow_mut().push(pty.clone());
            Spawned {
                pty: Some(Pty::from_backend(pty)),
                queue: None,
                error: None,
            }
        })
    }

    fn launch(&self, index: usize) -> Launch {
        self.launches.borrow()[index].clone()
    }

    fn pty(&self, index: usize) -> Arc<Recorder> {
        self.ptys.borrow()[index].clone()
    }
}

fn title_bar() -> aureole::TitleBarInfo {
    aureole::TitleBarInfo {
        height: 40.0,
        controls: Some(Rect {
            x: 16.0,
            y: 13.0,
            width: 54.0,
            height: 14.0,
        }),
        inset_left: 82.0,
        inset_right: 0.0,
    }
}

/// 一个空的工作区（没有标签，和第一次启动一样），挂上界面
fn blank(store: Store) -> (Harness, Rc<Workspace>, Fake) {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, store);
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    (harness, workspace, fake)
}

/// 同 [`blank`]，但在左上角画上 macOS 的红绿灯（真机上是系统画的，离屏渲染里没有）：
/// 位置和 `title_bar()` 里给的一致。README 的截图要带上它们
fn blank_with_traffic_lights(store: Store) -> (Harness, Rc<Workspace>, Fake) {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, store);
    let light = |fill: u32, edge: u32| {
        rect()
            .size(12, 12)
            .radius(6)
            .background(Color::hex(fill))
            .border(1, Color::hex(edge))
    };
    let lights = row()
        .inset_left(16)
        .inset_top(14)
        .gap(8)
        .child(light(0xff5f57, 0xe0443e))
        .child(light(0xfebc2e, 0xdea123))
        .child(light(0x28c840, 0x1aab29));
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        column()
            .grow(1)
            .child(chrome::build(workspace.clone()))
            .child(lights),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    (harness, workspace, fake)
}

/// 不起真 shell 的工作区，已经开着一个标签：每个标签一个只记录写入的假 pty
fn mounted() -> (Harness, Rc<Workspace>) {
    mounted_with(title_bar())
}

/// 用给定的标题栏度量挂（macOS 的红绿灯，或者没有系统按钮的 Windows、Linux）
fn mounted_with(info: aureole::TitleBarInfo) -> (Harness, Rc<Workspace>) {
    let workspace = Workspace::new(Fake::default().spawn(), None, Store::none());
    workspace.new_tab();
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(info);
    harness.frame();
    (harness, workspace)
}

/// 每个测试自己的临时目录（会被创建出来），用完删掉
fn directory(label: &str) -> PathBuf {
    let directory = crate::store::tests::scratch(label);
    std::fs::create_dir_all(&directory).expect("建得了目录");
    directory
}

/// 侧栏的顺序（开的先后）
fn ids(workspace: &Workspace) -> Vec<u64> {
    workspace
        .tabs
        .get_untracked()
        .iter()
        .map(|tab| tab.id)
        .collect()
}

/// 顶部标签条的顺序
fn bar_ids(workspace: &Workspace) -> Vec<u64> {
    workspace.bar.get_untracked()
}

/// 侧栏里的会话行，从上到下的 id
fn sidebar_order(harness: &Harness, workspace: &Workspace) -> Vec<u64> {
    let mut rows: Vec<(f32, u64)> = ids(workspace)
        .into_iter()
        .map(|id| {
            (
                harness
                    .window_bounds(&format!("session-{id}"))
                    .expect("会话行在")
                    .y,
                id,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, id)| id).collect()
}

/// 标签条上的标签，从左到右的 id
fn bar_order(harness: &Harness, workspace: &Workspace) -> Vec<u64> {
    let mut tabs: Vec<(f32, u64)> = ids(workspace)
        .into_iter()
        .map(|id| {
            (
                harness
                    .window_bounds(&format!("tab-{id}"))
                    .expect("标签在")
                    .x,
                id,
            )
        })
        .collect();
    tabs.sort_by(|a, b| a.0.total_cmp(&b.0));
    tabs.into_iter().map(|(_, id)| id).collect()
}

fn command(harness: &Harness, key: &str) {
    harness.key_down_with(Key::Character(key.into()), Modifiers::SUPER);
    harness.frame();
}

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

/// 关掉最后一个标签：窗口还在，面板上写没有标签；再新建就又能用
#[test]
fn closing_the_last_tab_leaves_an_empty_panel() {
    let (harness, workspace) = mounted();
    command(&harness, "w");
    assert!(ids(&workspace).is_empty(), "会话都关了");
    assert!(harness.bounds("card").is_some(), "窗口还在");
    assert!(harness.bounds("grid").is_none(), "没有终端");
    assert_eq!(harness.text("no-tabs").as_deref(), Some("没有标签"));
    assert!(
        harness.bounds("status-target").is_none(),
        "没有会话就不显示状态条"
    );
    command(&harness, "t");
    assert_eq!(ids(&workspace), [2]);
    assert!(harness.bounds("grid").is_some(), "新建以后终端回来了");
    assert!(harness.bounds("no-tabs").is_none());
}

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
    // 一直拖到窗口中间（标签条外面）再松手：取消
    let before = harness.window_bounds("tab-2").expect("在");
    drag_to(&harness, "tab-2", 600.0);
    harness.advance(secs(1.0));
    // 拖着的时候标签不出标签条（框架的 drag_preview_contained），停在最右边那一格里
    let held = harness.window_bounds("tab-2").expect("拖着的标签还看得见");
    let last = harness.window_bounds("tab-3").expect("在");
    assert!(
        held.x + held.width <= last.x + last.width + 1.0,
        "拖着的标签没有出标签条：{held:?} {last:?}"
    );
    harness.mouse_up(MouseButton::Left);
    harness.advance(secs(1.0));
    assert_eq!(bar_ids(&workspace), [1, 2, 3], "拖出去松手不换顺序");
    let after = harness.window_bounds("tab-2").expect("在");
    assert!(
        (after.x - before.x).abs() < 1.0,
        "标签落回原来的格子：{before:?} → {after:?}"
    );
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

/// 右键点一下命名元素的中心
fn right_click_named(harness: &Harness, name: &str) {
    let [x, y] = harness.window_bounds(name).expect("元素在").center();
    harness.right_click(x, y);
    harness.frame();
}

/// 点浮层里的菜单项（浮层里的元素用窗口坐标点）
fn click_menu(harness: &Harness, item: &str) {
    let [x, y] = harness
        .window_bounds(item)
        .unwrap_or_else(|| panic!("菜单里没有 {item}"))
        .center();
    harness.click(x, y);
    harness.advance(secs(1.0));
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

fn click(harness: &Harness, name: &str) {
    harness.click_named(name);
    harness.advance(secs(1.0));
}

/// 侧栏没有「本机」。目标下面的空白处右键能新建文件夹，文件夹上右键「创建会话」开一个终端并归在它下面
#[test]
fn a_folder_made_from_the_blank_area_holds_a_new_session() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    assert!(harness.text("folder-1").is_none(), "一开始没有文件夹");
    right_click_named(&harness, "targets-blank");
    assert!(
        harness.bounds("menu-targets").is_some(),
        "空白处右键是新建文件夹"
    );
    click_menu(&harness, "menu-new-folder");
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("folder-field"),
        "焦点在名字框里"
    );
    assert!(
        harness
            .text_state("folder-field")
            .unwrap_or_default()
            .contains("文件夹"),
        "默认名字"
    );
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(harness.bounds("folder-dialog").is_none());
    assert_eq!(harness.text("folder-1").as_deref(), Some("文件夹"));
    assert_eq!(ids(&workspace), [1], "新建文件夹不开终端");
    right_click_named(&harness, "folder-1");
    click_menu(&harness, "menu-new-session-folder-1");
    assert_eq!(ids(&workspace), [1, 2]);
    assert_eq!(workspace.tabs.get_untracked()[1].folder, Some(1));
    assert_eq!(workspace.active.get_untracked(), 2);
    let (folder, session) = (
        harness.window_bounds("folder-1").expect("文件夹在"),
        harness
            .window_bounds("session-2")
            .expect("新会话在文件夹下面"),
    );
    assert!(folder.y < session.y, "会话排在自己的文件夹下面");
    click(&harness, "folder-1");
    assert!(
        harness.bounds("session-2").is_none(),
        "再点一下文件夹，里面的会话收起来"
    );
}

/// 真 shell：从文件夹里新建的终端，屏幕上有提示符，而且 shell 真的站在文件夹对应的目录里
#[test]
fn a_real_shell_starts_in_the_folders_directory_and_shows_a_prompt() {
    let project = directory("real-shell");
    let workspace = Workspace::new(Rc::new(crate::app::spawn), None, Store::none());
    let folder = workspace.add_directory(&project).expect("是目录");
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.frame();
    assert!(harness.bounds("no-tabs").is_some(), "启动是空的");
    workspace.new_tab_in(folder);
    harness.frame();
    assert!(harness.bounds("no-tabs").is_none() && harness.bounds("grid").is_some());
    let session = workspace.tabs.get_untracked()[0].session().clone();
    let mut saw = String::new();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        harness.advance(ms(50));
        saw = session.screen_text_for_test();
        if saw.chars().any(|c| !c.is_whitespace() && c != '\0') {
            break;
        }
    }
    assert!(
        saw.chars().any(|c| !c.is_whitespace() && c != '\0'),
        "屏幕是空的：{saw:?}"
    );
    let expected = project.canonicalize().expect("目录在");
    let mut actual = None;
    for _ in 0..40 {
        actual = session
            .working_directory()
            .and_then(|path| path.canonicalize().ok());
        if actual.as_ref() == Some(&expected) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(actual, Some(expected), "shell 起在文件夹的目录里");
    let _ = std::fs::remove_dir_all(&project);
}

/// 第一次启动：一个标签也没有，面板上写「没有标签」，下面是「选择目录」；平时的终端、监控开关已经没有了
#[test]
fn the_first_launch_is_empty_with_a_choose_directory_button_below_the_label() {
    let (harness, workspace, fake) = blank(Store::none());
    assert!(ids(&workspace).is_empty());
    assert!(fake.launches.borrow().is_empty(), "启动时一个终端也没起");
    assert_eq!(harness.text("no-tabs").as_deref(), Some("没有标签"));
    let (label, button) = (
        harness.bounds("no-tabs").expect("标题在"),
        harness
            .bounds("choose-directory")
            .expect("选择目录的按钮在"),
    );
    assert!(
        button.y > label.y + label.height - 0.5,
        "按钮在「没有标签」下面"
    );
    assert_eq!(
        harness.text("choose-directory").as_deref(),
        Some("选择目录")
    );
    assert!(harness.bounds("grid").is_none());
    assert!(harness.bounds("folder-1").is_none(), "目标里是空的");
    for gone in ["mode-terminal", "mode-monitor", "mode-terminal-icon"] {
        assert!(harness.bounds(gone).is_none(), "{gone} 已经去掉");
    }
}

/// 点「选择目录」弹出系统的选目录对话框；选好后目录进「目标」，并在里面开一个终端，从那个目录起
#[test]
fn choosing_a_directory_adds_a_folder_and_opens_a_terminal_in_it() {
    let project = directory("choose");
    let (harness, workspace, fake) = blank(Store::none());
    click(&harness, "choose-directory");
    let requests = harness.file_dialogs();
    assert_eq!(requests.len(), 1, "弹出了一个对话框");
    assert_eq!(requests[0].kind, aureole::FileDialogKind::OpenDirectory);
    assert!(ids(&workspace).is_empty(), "还没选，不开终端");
    assert!(harness.respond_file_dialog(vec![project.clone()]));
    harness.advance(secs(1.0));
    let folders = workspace.folders.get_untracked();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path.as_deref(), Some(project.as_path()));
    assert_eq!(
        Some(folders[0].name.as_str()),
        project.file_name().and_then(|name| name.to_str()),
        "文件夹用目录的名字"
    );
    assert_eq!(ids(&workspace), [1]);
    assert_eq!(
        workspace.tabs.get_untracked()[0].folder,
        Some(folders[0].id)
    );
    assert_eq!(fake.launch(0).directory, Some(project.clone()));
    assert!(harness.bounds("grid").is_some(), "终端挂上了");
    assert!(harness.bounds("no-tabs").is_none());
    // 取消对话框什么也不做
    workspace.close_tab(1);
    harness.advance(secs(1.0));
    click(&harness, "choose-directory");
    assert!(harness.respond_file_dialog(Vec::new()));
    assert!(ids(&workspace).is_empty());
    assert_eq!(workspace.folders.get_untracked().len(), 1);
    let _ = std::fs::remove_dir_all(&project);
}

/// 把目录从访达拖到「目标」：加成一个文件夹；文件和已经有的目录不重复加；在里面创建终端就从那个目录起
#[test]
fn dropping_a_directory_on_the_targets_makes_a_folder_that_starts_terminals_there() {
    let project = directory("drop");
    let file = project.join("a.txt");
    std::fs::write(&file, "x").expect("写得进");
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    harness.advance(secs(1.0));
    let [x, y] = harness
        .window_bounds("targets-zone")
        .expect("目标区域在")
        .center();
    harness.hover_files(Some([x, y]));
    harness.advance(ms(50));
    harness.drop_files(x, y, vec![file.clone(), project.clone()]);
    harness.advance(secs(1.0));
    let folders = workspace.folders.get_untracked();
    assert_eq!(folders.len(), 1, "文件不算，目录加了一个");
    assert_eq!(folders[0].path.as_deref(), Some(project.as_path()));
    assert!(harness.bounds("folder-1").is_some(), "侧栏里出现了");
    assert_eq!(ids(&workspace), [1], "只加文件夹，不自己开终端");
    // 同一个目录再拖一次：还是一个
    harness.drop_files(x, y, vec![project.clone()]);
    harness.advance(secs(1.0));
    assert_eq!(workspace.folders.get_untracked().len(), 1);
    // 在文件夹里创建终端：从拖进来的目录起；再 ⌘T 新建（当前在这个文件夹里）也一样
    right_click_named(&harness, "folder-1");
    click_menu(&harness, "menu-new-session-folder-1");
    assert_eq!(fake.launch(1).directory, Some(project.clone()));
    workspace.new_tab();
    assert_eq!(fake.launch(2).directory, Some(project.clone()));
    // 不在文件夹里的终端不受影响
    assert_eq!(fake.launch(0).directory, None);
    let _ = std::fs::remove_dir_all(&project);
}

/// 起始目录的优先：文件夹自己的目录，其次设置里指定的，都不是目录就用默认（主目录）
#[test]
fn the_start_directory_comes_from_the_folder_then_the_settings() {
    let (folder_dir, settings_dir) = (directory("folder-dir"), directory("settings-dir"));
    let (_harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    assert_eq!(fake.launch(0).directory, None, "默认是主目录");
    workspace
        .form
        .directory
        .set(crate::settings::DirectoryKind::Custom);
    workspace
        .form
        .directory_path
        .set(settings_dir.display().to_string());
    workspace.new_tab();
    assert_eq!(fake.launch(1).directory, Some(settings_dir.clone()));
    let folder = workspace.add_directory(&folder_dir).expect("是目录");
    workspace.new_tab_in(folder);
    assert_eq!(
        fake.launch(2).directory,
        Some(folder_dir.clone()),
        "文件夹的目录优先"
    );
    workspace
        .form
        .directory_path
        .set("/不存在的目录/xyz".to_string());
    workspace.new_tab();
    assert_eq!(
        fake.launch(3).directory,
        Some(folder_dir.clone()),
        "在文件夹里：沿用文件夹的"
    );
    workspace.select(1);
    workspace.new_tab();
    assert_eq!(
        fake.launch(4).directory,
        None,
        "指定的目录不存在：退回主目录"
    );
    assert!(
        workspace
            .add_directory(Path::new("/不存在的目录/xyz"))
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&folder_dir);
    let _ = std::fs::remove_dir_all(&settings_dir);
}

/// 设置里的 shell、配色会带到新终端上
#[test]
fn settings_choices_reach_new_terminals() {
    let (_harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    let first = fake.launch(0);
    assert_eq!(first.shell, None);
    assert!(!first.dark_background);
    workspace.form.shell.set(ShellKind::Program);
    workspace.new_tab();
    assert_eq!(fake.launch(1).shell, None, "指定程序但路径是空的：还是默认");
    workspace.form.shell_program.set("  /bin/bash ".to_string());
    workspace.form.appearance.set(ThemeChoice::Dark);
    workspace.new_tab();
    let third = fake.launch(2);
    assert_eq!(third.shell.as_deref(), Some("/bin/bash"));
    assert!(third.dark_background, "深色配色告诉程序底色是深的");
}

/// 点设置：顶上的标签条、新建按钮、左右分、上下分都收起来，换成「设置」；侧栏顶上是「返回」，
/// 在平时放第一行的位置；点返回回到进来之前的会话
#[test]
fn settings_hides_the_tab_strip_and_splits_and_the_back_button_returns() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    for shown in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "toggle-sidebar",
    ] {
        assert!(harness.bounds(shown).is_some(), "平时有 {shown}");
    }
    assert!(harness.bounds("settings-title").is_none());
    let first_row = harness.window_bounds("nav-new").expect("平时侧栏的第一行");
    click(&harness, "nav-settings");
    assert!(workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2, "当前会话没变");
    for gone in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "nav-new",
        "nav-settings",
        "session-1",
        "grid",
    ] {
        assert!(harness.bounds(gone).is_none(), "设置页里 {gone} 不在了");
    }
    assert!(harness.bounds("toggle-sidebar").is_some(), "侧栏开关还在");
    assert_eq!(harness.text("settings-title").as_deref(), Some("设置"));
    assert!(harness.bounds("settings-view").is_some(), "卡片里是设置页");
    let back = harness.window_bounds("settings-back").expect("返回在");
    assert!(
        (back.x - first_row.x).abs() < 1.5 && (back.y - first_row.y).abs() < 1.5,
        "返回在侧栏最上面、平时第一行的位置：{back:?} vs {first_row:?}"
    );
    assert_eq!(harness.text("settings-back").as_deref(), Some("返回"));
    for page in SettingsPage::ALL {
        let bounds = harness.window_bounds(page.name()).expect("分类在");
        assert!(bounds.y > back.y, "「{}」在返回下面", page.label());
    }
    click(&harness, "settings-back");
    assert!(!workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2);
    for shown in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "nav-new",
        "grid",
    ] {
        assert!(harness.bounds(shown).is_some(), "回来以后 {shown} 又在了");
    }
    assert!(harness.bounds("settings-title").is_none());
    assert!(harness.bounds("settings-back").is_none());
}

/// 设置页里 ⌘, 再按一次、⌘W、点标签/快捷键切标签，都是退出设置；设置页不占标签
#[test]
fn settings_can_be_left_from_the_keyboard_and_is_not_a_tab() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    command(&harness, ",");
    assert!(workspace.settings_open.get_untracked());
    command(&harness, ",");
    assert!(!workspace.settings_open.get_untracked(), "再按一次退出");
    command(&harness, ",");
    command(&harness, "w");
    assert!(
        !workspace.settings_open.get_untracked(),
        "⌘W 在设置页里是返回"
    );
    assert_eq!(ids(&workspace), [1, 2], "没有误关会话");
    command(&harness, ",");
    command(&harness, "1");
    assert!(
        !workspace.settings_open.get_untracked(),
        "⌘1 切到标签就离开设置"
    );
    assert_eq!(workspace.active.get_untracked(), 1);
    command(&harness, ",");
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    assert!(!workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2);
    assert_eq!(bar_ids(&workspace), [1, 2], "标签条里从来没有设置");
    // 命令面板里也能开
    command(&harness, "k");
    let panel = harness.bounds("palette").expect("面板在");
    let row = harness.bounds("cmd-settings").expect("命令在");
    let origin = (
        (WINDOW_SIZE.0 as f32 - panel.width) / 2.0,
        (WINDOW_SIZE.1 as f32 - panel.height) / 2.0,
    );
    let [x, y] = row.center();
    harness.click(origin.0 + x, origin.1 + y);
    harness.frame();
    assert!(workspace.settings_open.get_untracked());
}

/// 设置的侧栏和平时的侧栏换面板有交接动画：刚换的时候两块都在，动画完了旧的才卸掉；
/// 收起侧栏时设置的分类只剩图标
#[test]
fn the_settings_sidebar_hands_over_and_collapses_to_icons() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    workspace.open_settings();
    harness.advance(ms(40));
    assert!(
        harness.bounds("nav-new").is_some() && harness.bounds("settings-general").is_some(),
        "交接中两块面板都在"
    );
    harness.advance(secs(1.0));
    assert!(harness.bounds("nav-new").is_none() && harness.bounds("settings-general").is_some());
    command(&harness, "b");
    harness.advance(secs(1.0));
    for page in SettingsPage::ALL {
        let bounds = harness.window_bounds(page.name()).expect("分类图标在");
        assert!(bounds.width <= 48.0, "收起时只剩图标，宽 {}", bounds.width);
    }
    let back = harness.window_bounds("settings-back").expect("返回在");
    assert!(back.width <= 48.0, "返回也只剩图标");
    workspace.close_settings();
    harness.advance(secs(1.0));
    assert!(harness.bounds("nav-new").is_some() && harness.bounds("settings-general").is_none());
}

/// 每一类设置页都能点开，点的选择真的写进设置、换页回来还在；设置页的内容占满卡片的宽度
#[test]
fn every_settings_page_opens_and_its_choices_stick() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    harness.advance(secs(1.0));
    for page in SettingsPage::ALL {
        click(&harness, page.name());
        assert_eq!(workspace.settings_page.get_untracked(), page);
        assert!(
            harness.bounds(&format!("{}-page", page.name())).is_some(),
            "「{}」的页面在",
            page.label()
        );
        for other in SettingsPage::ALL.into_iter().filter(|&other| other != page) {
            assert!(
                harness.bounds(&format!("{}-page", other.name())).is_none(),
                "别的页面卸掉"
            );
        }
        let (card, list) = (
            harness.window_bounds("card").expect("卡片在"),
            harness.window_bounds("settings-list").expect("列表在"),
        );
        assert!(
            (list.width - (card.width - 64.0)).abs() < 1.5,
            "「{}」列表宽 {} 应是卡片宽 {} 去掉两边各 32",
            page.label(),
            list.width,
            card.width
        );
    }
    let form = workspace.form;
    click(&harness, "settings-general");
    assert!(
        harness.bounds("set-shell-path").is_none(),
        "没选指定程序时不出现路径框"
    );
    click(&harness, "set-startup-empty");
    click(&harness, "set-shell-custom");
    assert!(
        harness.bounds("set-shell-path").is_some(),
        "指定程序下面出现路径框"
    );
    assert!(
        harness.bounds("set-shell-path-pick").is_some(),
        "还有「选择…」"
    );
    click(&harness, "set-directory-custom");
    assert!(harness.bounds("set-directory-path").is_some());
    click(&harness, "set-title-reported");
    click(&harness, "set-confirm-close");
    click(&harness, "set-last-tab-quit");
    click(&harness, "set-sidebar-closed");
    click(&harness, "settings-appearance");
    click(&harness, "set-appearance-dark");
    click(&harness, "set-cursor-bar");
    click(&harness, "set-cursor-blink");
    // 外观页比卡片高，最下面的要滚下去才点得到
    harness.wheel(600.0, 300.0, 0.0, 20.0);
    harness.advance(secs(1.0));
    // 滚过的页面里元素的位置要按窗口坐标点
    click_menu(&harness, "set-status");
    click(&harness, "settings-terminal");
    click(&harness, "set-bell-flash");
    click(&harness, "set-copy-on-select");
    click(&harness, "set-right-click");
    click(&harness, "settings-general");
    assert_eq!(form.startup.get_untracked(), Startup::Empty);
    assert_eq!(form.shell.get_untracked(), ShellKind::Program);
    assert_eq!(form.title.get_untracked(), TitleSource::Reported);
    assert!(!form.confirm_close.get_untracked(), "默认开着，点一下关掉");
    assert_eq!(form.last_tab.get_untracked(), LastTab::Quit);
    assert_eq!(form.sidebar.get_untracked(), SidebarStart::Closed);
    assert_eq!(form.appearance.get_untracked(), ThemeChoice::Dark);
    assert_eq!(
        form.cursor.get_untracked(),
        crate::settings::CursorStyle::Bar
    );
    assert!(!form.cursor_blink.get_untracked());
    assert!(!form.status_bar.get_untracked());
    assert_eq!(form.bell.get_untracked(), crate::settings::Bell::Flash);
    assert!(form.copy_on_select.get_untracked());
    assert!(form.right_click_paste.get_untracked());
    assert!(
        harness.bounds("set-shell-path").is_some(),
        "换页回来，指定程序还开着"
    );
    assert_eq!(ids(&workspace), [1], "点设置不新开、不关掉会话");
    // 恢复默认
    click(&harness, "settings-about");
    click(&harness, "set-reset");
    assert_eq!(form.snapshot(), Settings::default());
}

/// 「选择…」弹系统对话框（程序是选文件、目录是选文件夹），选好填进路径框
#[test]
fn the_pick_buttons_fill_the_path_fields() {
    let (harness, workspace) = mounted();
    workspace.form.shell.set(ShellKind::Program);
    workspace
        .form
        .directory
        .set(crate::settings::DirectoryKind::Custom);
    workspace.open_settings();
    harness.advance(secs(1.0));
    click(&harness, "set-shell-path-pick");
    let request = harness.file_dialogs().remove(0);
    assert_eq!(request.kind, aureole::FileDialogKind::OpenFile);
    assert!(harness.respond_file_dialog(vec![PathBuf::from("/bin/bash")]));
    assert_eq!(workspace.form.shell_program.get_untracked(), "/bin/bash");
    click(&harness, "set-directory-path-pick");
    let request = harness.file_dialogs().remove(0);
    assert_eq!(request.kind, aureole::FileDialogKind::OpenDirectory);
    assert!(harness.respond_file_dialog(vec![PathBuf::from("/tmp")]));
    assert_eq!(workspace.form.directory_path.get_untracked(), "/tmp");
}

/// 换配色：终端区域的底色马上跟着变（浅色白、深色深）；换字号：行列数跟着重算
#[test]
fn palette_and_font_settings_change_the_terminal_right_away() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    let corner = |harness: &Harness| {
        let card = harness.bounds("card").expect("卡片在");
        let shot = harness.screenshot();
        shot.pixel((card.x + 4.0) as u32, (card.y + 4.0) as u32).0
    };
    let bright = corner(&harness);
    assert!(
        bright[..3].iter().all(|&c| c > 240),
        "浅色是白底：{bright:?}"
    );
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let dark = corner(&harness);
    assert!(dark[..3].iter().all(|&c| c < 80), "深色是深底：{dark:?}");
    workspace.form.appearance.set(ThemeChoice::Light);
    harness.advance(secs(1.0));
    assert!(corner(&harness)[..3].iter().all(|&c| c > 240), "切回浅色");
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let (columns, rows) = workspace.size.get_untracked();
    workspace.form.font_size.set(22.0);
    harness.advance(secs(1.0));
    let (bigger_columns, bigger_rows) = workspace.size.get_untracked();
    assert!(
        bigger_columns < columns && bigger_rows < rows,
        "字大了放得下的行列少了：{columns}×{rows} → {bigger_columns}×{bigger_rows}"
    );
    workspace.form.font_size.set(13.0);
    harness.advance(secs(1.0));
    assert_eq!(
        workspace.size.get_untracked(),
        (columns, rows),
        "改回来行列也回来"
    );
    workspace.form.line_height.set(1.6);
    harness.advance(secs(1.0));
    assert!(workspace.size.get_untracked().1 < rows, "行距大了行数少了");
}

/// 外观页顶上的小样是真的：它的底色跟着配色走
#[test]
fn the_appearance_page_has_a_live_preview() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    let preview = harness.window_bounds("settings-preview").expect("小样在");
    let sample = |harness: &Harness| {
        harness
            .screenshot()
            .pixel(
                (preview.x + preview.width - 6.0) as u32,
                (preview.y + 6.0) as u32,
            )
            .0
    };
    assert!(sample(&harness)[..3].iter().all(|&c| c > 240));
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    assert!(sample(&harness)[..3].iter().all(|&c| c < 80));
}

/// 设置里关掉状态条：状态条消失
#[test]
fn the_status_bar_can_be_turned_off() {
    let (harness, workspace) = mounted();
    assert!(harness.bounds("status-target").is_some());
    workspace.form.status_bar.set(false);
    harness.advance(secs(1.0));
    assert!(harness.bounds("status-target").is_none());
    workspace.form.status_bar.set(true);
    harness.advance(secs(1.0));
    assert!(harness.bounds("status-target").is_some());
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

/// 文件夹、开着的标签（顺序、名字、关掉时的目录）、侧栏宽度都存下来，下次启动恢复；
/// 设置里「启动时」选空白就不恢复标签（文件夹还在）
#[test]
fn folders_and_tabs_survive_a_restart() {
    let root = directory("persist");
    let store = Store::at(root.join("config"));
    let (project, nested) = (root.join("project"), root.join("project").join("sub"));
    std::fs::create_dir_all(&nested).expect("建得了目录");
    {
        let (_harness, workspace, fake) = blank(store.clone());
        let folder = workspace.add_directory(&project).expect("是目录");
        workspace.create_folder("临时");
        workspace.new_tab_in(folder);
        workspace.new_tab();
        workspace.new_tab();
        workspace.rename_tab(2, "构建");
        // 第一个标签里 shell cd 进了子目录；标签条里把第三个拖到最前面
        fake.pty(0).set_directory(&nested);
        workspace.move_tab(3, 0);
        workspace.select(2);
        workspace.sidebar_width.set(280.0);
        workspace.save_now();
    }
    assert!(root.join("config").join("state.json").exists());
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(
            workspace
                .folders
                .get_untracked()
                .iter()
                .map(|folder| (folder.name.clone(), folder.path.clone()))
                .collect::<Vec<_>>(),
            [
                ("project".to_string(), Some(project.clone())),
                ("临时".to_string(), None)
            ],
            "文件夹和它们的目录回来了"
        );
        assert_eq!(workspace.sidebar_width.get_untracked(), 280.0);
        assert!(ids(&workspace).is_empty(), "标签要 restore 才开");
        assert_eq!(workspace.restore(), 3);
        // 标签条的顺序存下来了：原来的第三个在最前面
        let launches: Vec<_> = (0..3).map(|index| fake.launch(index).directory).collect();
        assert_eq!(
            launches,
            [
                Some(project.clone()),
                Some(nested.clone()),
                Some(project.clone())
            ],
            "各自回到关掉时的目录（没问到目录的用文件夹的）"
        );
        // 开的先后和标签条一致：原来标签条里第三个（id 3）现在是 1，「构建」（原来的 2）现在是 3
        let titles = workspace.titles.get_untracked();
        assert_eq!(titles[&3].as_str(), "构建", "起过的名字也在");
        assert_eq!(titles[&1].as_str(), "终端");
        assert_eq!(workspace.active.get_untracked(), 3, "选中的还是原来那个");
        assert_eq!(
            workspace
                .tabs
                .get_untracked()
                .iter()
                .map(|tab| tab.folder)
                .collect::<Vec<_>>(),
            [Some(1), Some(1), Some(1)]
        );
        // 再关一次、改设置：下次启动不恢复标签
        workspace.save_now();
        workspace.form.startup.set(Startup::Empty);
    }
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(
            workspace.form.startup.get_untracked(),
            Startup::Empty,
            "设置也存了"
        );
        assert_eq!(workspace.restore(), 0);
        assert!(
            fake.launches.borrow().is_empty(),
            "空白启动：一个终端也不起"
        );
        assert_eq!(workspace.folders.get_untracked().len(), 2, "文件夹还在");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 启动时侧栏收起的设置：下次启动侧栏是收起的
#[test]
fn the_sidebar_start_setting_is_applied_at_launch() {
    let root = directory("sidebar-start");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert!(!workspace.collapsed.get_untracked());
        workspace.form.sidebar.set(SidebarStart::Closed);
    }
    let (_harness, workspace, _fake) = blank(store);
    assert!(workspace.collapsed.get_untracked());
    let _ = std::fs::remove_dir_all(&root);
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

/// 终端里的内容跟着标签存下来，下次恢复标签时先放回去（带颜色），新 shell 的提示符接在后面；
/// 设置里关掉就不存、不恢复，已经存的文件也删掉
#[test]
fn terminal_output_is_saved_and_comes_back_with_the_tab() {
    let root = directory("output");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        workspace.new_tab();
        workspace.new_tab();
        workspace.tabs.get_untracked()[0]
            .session()
            .feed_for_test("\x1b[32mhello\x1b[0m 世界\r\nsecond line");
        workspace.save_now();
        assert!(root.join("config").join("output-0.ansi").exists());
        assert!(
            !root.join("config").join("output-1.ansi").exists(),
            "没有内容的标签不存文件"
        );
        // 再来新输出才重写；关掉一个标签后多出来的文件要删
        workspace.tabs.get_untracked()[1]
            .session()
            .feed_for_test("another");
        workspace.save_now();
        assert!(root.join("config").join("output-1.ansi").exists());
        workspace.close_tab(2);
        workspace.save_now();
        assert!(!root.join("config").join("output-1.ansi").exists());
    }
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 1);
        let session = workspace.tabs.get_untracked()[0].session().clone();
        let screen = session.screen_text_for_test();
        let screen = screen.replace('\0', "");
        let mut lines = screen.lines().map(str::trim_end);
        assert_eq!(lines.next(), Some("hello 世界"));
        assert_eq!(lines.next(), Some("second line"));
        assert!(
            session.transcript(100).contains("\x1b[0;38;5;2mhello"),
            "颜色也回来了：{:?}",
            session.transcript(100)
        );
        // 关掉「恢复时带上终端里的内容」：文件删掉，下次不恢复内容
        workspace.form.save_output.set(false);
        workspace.save_now();
        assert!(!root.join("config").join("output-0.ansi").exists());
    }
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 1);
        let screen = workspace.tabs.get_untracked()[0]
            .session()
            .screen_text_for_test();
        assert!(screen.trim().is_empty(), "没存内容：{screen:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
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

/// 终端都有图标：直接列在「目标」下的，图标和文件夹的图标同一列、一样大，名字也和文件夹的名字对齐；文件夹里的缩进一级
#[test]
fn loose_terminals_have_an_icon_aligned_with_the_folder_icon() {
    let (harness, workspace, _fake) = blank(Store::none());
    let folder = workspace.create_folder("工作");
    workspace.new_tab();
    workspace.new_tab_in(folder);
    harness.advance(secs(1.0));
    let (folder_icon, session_icon) = (
        harness
            .window_bounds("folder-icon-1")
            .expect("文件夹图标在"),
        harness.window_bounds("session-icon-1").expect("终端图标在"),
    );
    assert!((folder_icon.x - session_icon.x).abs() < 0.5, "同一列");
    assert!(
        (folder_icon.width - session_icon.width).abs() < 0.5,
        "一样大"
    );
    let (row_name, folder_row) = (
        harness.window_bounds("session-1").expect("行在"),
        harness.window_bounds("folder-1").expect("文件夹行在"),
    );
    assert!(
        (row_name.x - folder_row.x).abs() < 0.5 && (row_name.width - folder_row.width).abs() < 0.5
    );
    // 文件夹里的终端也有图标，缩进到文件夹名字的那一列
    let (child_icon, folder_row) = (
        harness
            .window_bounds("session-icon-2")
            .expect("文件夹里的终端也有图标"),
        harness.window_bounds("folder-icon-1").expect("在"),
    );
    assert!(
        (child_icon.x - (folder_row.x + folder_row.width + 8.0)).abs() < 0.5,
        "图标和文件夹的名字同一列：{child_icon:?} {folder_row:?}"
    );
    assert!((child_icon.width - folder_row.width).abs() < 0.5, "一样大");
    // 收起侧栏：目标下的终端还在，只剩图标
    command(&harness, "b");
    harness.advance(secs(1.0));
    let icon = harness
        .window_bounds("session-icon-1")
        .expect("收起后图标还在");
    let sidebar = harness.window_bounds("sidebar").expect("侧栏在");
    assert!(icon.x + icon.width < sidebar.x + sidebar.width);
    assert!(
        harness.text("session-1").is_none() || harness.text("session-1").as_deref() == Some("")
    );
}

/// 字体：终端里的字是等宽的，一格的宽度大约是字号的 0.6 倍，不是退到界面字体后撑开的两倍宽
#[test]
fn the_terminal_font_is_monospace_for_every_choice() {
    use crate::settings::FontChoice;
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    for font in [
        FontChoice::System,
        FontChoice::Menlo,
        FontChoice::Monaco,
        FontChoice::CourierNew,
    ] {
        workspace.form.font.set(font);
        harness.advance(secs(1.0));
        let grid = harness.bounds("grid").expect("终端在");
        let columns = workspace.size.get_untracked().0 as f32;
        let cell = grid.width / columns;
        assert!(
            (6.0..9.5).contains(&cell),
            "{font:?}：一格宽 {cell}，13 号等宽字应该在 8 上下"
        );
    }
}

/// 深色：整个窗口（标题栏、侧栏、卡片、终端）一起换；换的时候状态一点不丢（标签、设置页停在原来的页）；
/// 选「跟随系统」就跟着系统的外观走
#[test]
fn the_whole_window_follows_the_appearance_setting() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    let brightness = |harness: &Harness, x: f32, y: f32| {
        let pixel = harness.screenshot().pixel(x as u32, y as u32).0;
        pixel[..3].iter().map(|&c| c as u32).sum::<u32>() / 3
    };
    let card = harness.window_bounds("card").expect("卡片在");
    let probes = |harness: &Harness| {
        [
            brightness(harness, 600.0, 8.0),                   // 标题栏
            brightness(harness, 8.0, 400.0),                   // 侧栏
            brightness(harness, card.x + 4.0, card.y + 300.0), // 卡片
        ]
    };
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "默认（跟随系统，系统是浅色）是浅的"
    );
    click(&harness, "set-appearance-dark");
    assert!(
        probes(&harness).iter().all(|&b| b < 60),
        "深色：标题栏、侧栏、卡片都是深的 {:?}",
        probes(&harness)
    );
    assert!(workspace.settings_open.get_untracked(), "还在设置页");
    assert_eq!(
        workspace.settings_page.get_untracked(),
        SettingsPage::Appearance
    );
    assert!(harness.bounds("settings-appearance-page").is_some());
    assert_eq!(ids(&workspace), [1, 2], "标签一个没丢");
    assert!(
        harness.bounds("set-appearance-dark").is_some(),
        "控件在，点得动"
    );
    // 回到终端：终端也是深的，字是浅色
    click(&harness, "settings-back");
    let terminal = harness.window_bounds("grid").expect("终端在");
    assert!(
        brightness(
            &harness,
            terminal.x + terminal.width - 4.0,
            terminal.y + 300.0
        ) < 60
    );
    // 跟随系统
    workspace.close_settings();
    workspace.open_settings();
    harness.advance(secs(1.0));
    click(&harness, "set-appearance-system");
    harness.set_system_appearance(aureole::Appearance::Light);
    harness.advance(secs(1.0));
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "系统浅色就是浅色"
    );
    harness.set_system_appearance(aureole::Appearance::Dark);
    harness.advance(secs(1.0));
    // 离屏渲染里没有系统的材质，半透明的底盖在浅色的清屏色上，比真机里浅一点
    assert!(
        probes(&harness).iter().all(|&b| b < 130),
        "系统深色就是深色 {:?}",
        probes(&harness)
    );
    click(&harness, "set-appearance-light");
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "选浅色就不跟系统"
    );
}

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

/// 拆分的排布、每个终端的目录和里面的内容、聚焦的是哪个、分隔条的位置，都存下来，下次启动照样恢复
#[test]
fn split_layouts_survive_a_restart() {
    let root = directory("split-persist");
    let store = Store::at(root.join("config"));
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(&a).expect("建得了目录");
    std::fs::create_dir_all(&b).expect("建得了目录");
    {
        let (_harness, workspace, fake) = blank(store.clone());
        workspace.new_tab();
        fake.pty(0).set_directory(&a);
        workspace.split(Axis::Columns);
        fake.pty(1).set_directory(&b);
        workspace.split(Axis::Rows);
        let tab = workspace.tabs.get_untracked()[0].clone();
        let leaves = tab.layout.get_untracked().leaves();
        leaves[0].1.feed_for_test("first pane output");
        leaves[1].1.feed_for_test("second pane output");
        leaves[2].1.feed_for_test("third pane output");
        workspace.focus_pane(1, leaves[1].0);
        if let Pane::Split { ratio, .. } = tab.layout.get_untracked() {
            ratio.set(0.3);
        }
        workspace.new_tab();
        workspace.tabs.get_untracked()[1]
            .session()
            .feed_for_test("other tab");
        workspace.finish();
    }
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 2);
        let tabs = workspace.tabs.get_untracked();
        let leaves = tabs[0].layout.get_untracked().leaves();
        assert_eq!(leaves.len(), 3, "三个终端回来了");
        assert_eq!(fake.launch(0).directory, Some(a.clone()));
        assert_eq!(fake.launch(1).directory, Some(b.clone()));
        let texts: Vec<String> = leaves
            .iter()
            .map(|(_, session)| session.screen_text_for_test().replace('\0', ""))
            .collect();
        assert!(texts[0].contains("first pane output"));
        assert!(texts[1].contains("second pane output"));
        assert!(texts[2].contains("third pane output"));
        assert!(
            tabs[1]
                .session()
                .screen_text_for_test()
                .contains("other tab")
        );
        assert_eq!(
            tabs[0].focus.get_untracked(),
            leaves[1].0,
            "聚焦的还是第二个"
        );
        match tabs[0].layout.get_untracked() {
            Pane::Split {
                axis,
                ratio,
                second,
                ..
            } => {
                assert_eq!(axis, Axis::Columns);
                assert!((ratio.get_untracked() - 0.3).abs() < 0.001, "分隔条的位置");
                assert!(
                    matches!(
                        *second,
                        Pane::Split {
                            axis: Axis::Rows,
                            ..
                        }
                    ),
                    "右边又上下分了"
                );
            }
            Pane::Leaf { .. } => panic!("应该是拆分的"),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 退出时（窗口已经关了、界面已经卸载）还要能把终端里最后的内容存下来；存过一次之后不再存
/// （窗口关了以后 shell 收到 SIGHUP 退出，屏幕上多出来的「进程已退出」不该被存进去）
#[test]
fn the_workspace_saves_its_last_output_after_the_window_is_gone() {
    let root = directory("exit-save");
    let store = Store::at(root.join("config"));
    let (harness, workspace, _fake) = blank(store.clone());
    workspace.new_tab();
    let session = workspace.tabs.get_untracked()[0].session();
    session.feed_for_test("npm run dev\r\nready in 300 ms");
    drop(harness);
    workspace.finish();
    let saved = std::fs::read_to_string(root.join("config").join("output-0.ansi")).expect("存了");
    assert!(
        saved.contains("ready in 300 ms"),
        "最后的输出也在：{saved:?}"
    );
    session.feed_for_test("\r\n[进程已退出，退出码 129]");
    workspace.save_now();
    let again = std::fs::read_to_string(root.join("config").join("output-0.ansi")).expect("在");
    assert!(!again.contains("进程已退出"), "退出之后不再存");
    let _ = std::fs::remove_dir_all(&root);
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

/// 切换深浅色时界面整个重建，但入场动画（侧栏滑进来、标签淡入）不能重放：刚切完的头几帧，
/// 侧栏的行就在原来的位置、完全不透明
#[test]
fn switching_the_appearance_does_not_replay_the_enter_animations() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    let before = (
        harness.window_bounds("nav-new").expect("在"),
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("session-1").expect("在"),
    );
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(ms(30));
    let after = (
        harness.window_bounds("nav-new").expect("在"),
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("session-1").expect("在"),
    );
    assert_eq!(
        before, after,
        "刚切完，各处还在原来的位置，没有从旁边滑进来"
    );
    // 正常的入场动画还在：新开的标签照样淡入
    // 豁免只管换色后那一瞬间：过一会儿新开的东西照样有入场动画
    assert!(crate::theme::just_switched());
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(!crate::theme::just_switched());
    // 设成和现在一样的不算「换」
    crate::theme::set_dark(true);
    assert!(!crate::theme::just_switched(), "没变就不算");
}

/// 外观页有「粗细」（常规、粗两档），点了存得住
#[test]
fn the_appearance_page_has_a_font_weight_switch() {
    use crate::settings::FontWeight;
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    assert!(
        harness.window_bounds("set-font-weight-regular").is_some(),
        "常规在"
    );
    assert!(
        harness.window_bounds("set-font-weight-light").is_none(),
        "只有两档"
    );
    click_menu(&harness, "set-font-weight-bold");
    assert_eq!(workspace.form.snapshot().font_weight, FontWeight::Bold);
    click_menu(&harness, "set-font-weight-regular");
    assert_eq!(workspace.form.snapshot().font_weight, FontWeight::Regular);
}

/// 字体粗细真的改变终端里字的粗细：粗的比细的墨多，格子大小不变
#[test]
fn the_font_weight_setting_changes_how_heavy_the_terminal_text_is() {
    use crate::settings::FontWeight;
    let (harness, workspace) = mounted();
    let session = workspace.tabs.get_untracked()[0].session();
    session.feed_for_test(&"MMMMMMMMMMMMMMMMMMMMMMMM\r\n".repeat(10));
    harness.advance(secs(1.0));
    let ink = |harness: &Harness| {
        let grid = harness.window_bounds("grid").expect("在");
        let shot = harness.screenshot();
        let mut sum = 0u64;
        for y in grid.y as u32..(grid.y + 130.0) as u32 {
            for x in grid.x as u32..(grid.x + 260.0) as u32 {
                sum += 255 - u64::from(shot.pixel(x, y).0[0]);
            }
        }
        sum
    };
    let columns = workspace.size.get_untracked();
    workspace.form.font_weight.set(FontWeight::Regular);
    harness.advance(secs(1.0));
    let light = ink(&harness);
    workspace.form.font_weight.set(FontWeight::Bold);
    harness.advance(secs(1.0));
    let bold = ink(&harness);
    assert!(
        bold as f64 > light as f64 * 1.1,
        "粗的字墨更多：{light} → {bold}"
    );
    assert_eq!(workspace.size.get_untracked(), columns, "粗细不改格子大小");
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

/// 关于页只留有用的：版本、框架的链接、GitHub 图标（源码）、配置文件夹、恢复默认；
/// 链接点了交给系统打开
#[test]
fn the_about_page_links_to_the_framework_and_the_source() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::About);
    harness.advance(secs(1.0));
    let tree = format!("{:?}", harness.element_tree());
    for useless in ["终端仿真", "上的终端模拟器", "自研 pty"] {
        assert!(!tree.contains(useless), "关于里不该有「{useless}」");
    }
    assert!(tree.contains(crate::product::NAME));
    assert_eq!(harness.text("about-framework").as_deref(), Some("Aureole"));
    assert_eq!(harness.text("about-github").as_deref(), Some("GitHub"));
    assert!(harness.bounds("set-reset").is_some());
    click(&harness, "about-framework");
    click(&harness, "about-github");
    assert_eq!(
        harness.opened_urls(),
        [
            crate::product::FRAMEWORK_URL.to_string(),
            crate::product::REPOSITORY.to_string()
        ]
    );
}

/// 深色里的控件看得清：关着的开关是深色的槽（不是一条白的）、数字框的加减按钮悬停不发白；
/// 浅色里关着的开关还是浅灰的槽
#[test]
fn switches_and_steppers_are_readable_in_dark() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Terminal);
    harness.advance(secs(1.0));
    let brightness = |harness: &Harness, x: f32, y: f32| {
        let pixel = harness.screenshot().pixel(x as u32, y as u32).0;
        pixel[..3].iter().map(|&c| c as u32).sum::<u32>() / 3
    };
    let track = |harness: &Harness| {
        let bounds = harness.window_bounds("set-copy-on-select").expect("开关在");
        brightness(
            harness,
            bounds.x + bounds.width - 5.0,
            bounds.y + bounds.height / 2.0,
        )
    };
    let light_off = track(&harness);
    assert!(light_off > 190, "浅色里关着的槽是浅灰：{light_off}");
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let dark_off = track(&harness);
    assert!(dark_off < 90, "深色里关着的槽是深的：{dark_off}");
    workspace.form.copy_on_select.set(true);
    harness.advance(secs(1.0));
    let dark_on = track(&harness);
    assert!(
        dark_on > dark_off + 30,
        "开着的比关着的亮得多：{dark_off} → {dark_on}"
    );
    // 数字框的加号：鼠标放上去是一层淡灰，不是白的
    let stepper = harness.window_bounds("set-scrollback").expect("数字框在");
    let (x, y) = (
        stepper.x + stepper.width - 12.0,
        stepper.y + stepper.height / 2.0,
    );
    harness.mouse_move(x, y);
    harness.advance(secs(1.0));
    let hovered = brightness(&harness, x + 6.0, y + 8.0);
    assert!(hovered < 110, "悬停在加号上不发白：{hovered}");
}

/// 评审界面用：把几种状态存成 PNG。平时不跑；
/// `UI_SHOTS=目录 cargo test screenshots_for_review -- --ignored` 出图（离屏渲染，没有系统材质）
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_review() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let save = |harness: &Harness, name: &str| {
        harness.advance(secs(1.0));
        harness
            .screenshot()
            .save_png(directory.join(name))
            .expect("存得下 PNG");
    };
    // 第一次启动
    let (harness, workspace, _fake) = blank(Store::none());
    harness.set_scale_factor(2.0);
    save(&harness, "0-first-launch.png");
    let project = directory.join("项目");
    std::fs::create_dir_all(&project).expect("建得了目录");
    workspace.add_directory(&project);
    workspace.create_folder("临时");
    workspace.new_tab_in(1);
    workspace.tabs.get_untracked()[0].session().feed_for_test("\x1b[32mlicha\x1b[0m \x1b[34m~/终端\x1b[0m % cargo test\r\nrunning 77 tests\r\ntest result: \x1b[32mok\x1b[0m. 77 passed\r\n");
    workspace.new_tab();
    save(&harness, "1-expanded.png");
    workspace.split(Axis::Columns);
    workspace.split(Axis::Rows);
    save(&harness, "1b-split.png");
    workspace.close_pane(2, workspace.tabs.get_untracked()[1].focus.get_untracked());
    workspace.close_pane(2, workspace.tabs.get_untracked()[1].focus.get_untracked());
    right_click_named(&harness, "targets-blank");
    save(&harness, "2-targets-menu.png");
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    command(&harness, "b");
    save(&harness, "3-collapsed.png");
    command(&harness, "b");
    harness.click_named("nav-settings");
    save(&harness, "4-settings-general.png");
    harness.click_named("set-shell-custom");
    harness.click_named("set-directory-custom");
    save(&harness, "4b-settings-fields.png");
    harness.click_named("settings-appearance");
    save(&harness, "5-settings-appearance.png");
    harness.click_named("set-appearance-dark");
    save(&harness, "5b-settings-dark.png");
    harness.click_named("settings-terminal");
    save(&harness, "6-settings-terminal.png");
    harness.click_named("settings-shortcuts");
    save(&harness, "7-settings-shortcuts.png");
    harness.click_named("settings-about");
    save(&harness, "8-settings-about.png");
    command(&harness, "b");
    save(&harness, "9-settings-collapsed.png");
    command(&harness, "b");
    harness.click_named("settings-back");
    save(&harness, "10-back-with-dark-terminal.png");
    let _ = std::fs::remove_dir_all(&project);
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

/// README 里的截图：`README_SHOTS=docs/screenshots cargo test screenshots_for_readme -- --ignored`
/// （离屏渲染，没有系统的窗口材质，和真机上半透明的侧栏略有不同）
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_readme() {
    let Ok(directory) = std::env::var("README_SHOTS") else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("建得了目录");
    let save = |harness: &Harness, name: &str| {
        harness.advance(secs(1.0));
        harness
            .screenshot()
            .save_png(directory.join(name))
            .expect("存得下 PNG");
    };
    const BUILD: &str = "\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m \x1b[1;34mgit:(\x1b[31mmain\x1b[34m)\x1b[0m cargo test\r\n   \x1b[1;32mCompiling\x1b[0m nimbus-terminal v0.1.0\r\n    \x1b[1;32mFinished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 4.21s\r\n     \x1b[1;32mRunning\x1b[0m unittests src/main.rs\r\n\r\nrunning 170 tests\r\ntest vt::tests::mouse_modes_switch_on_and_off_independently ... \x1b[32mok\x1b[0m\r\ntest tabs::tests::split_layouts_survive_a_restart ... \x1b[32mok\x1b[0m\r\ntest tabs::tests::folders_and_tabs_survive_a_restart ... \x1b[32mok\x1b[0m\r\n\r\ntest result: \x1b[32mok\x1b[0m. 170 passed; 0 failed\r\n\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m \x1b[1;34mgit:(\x1b[31mmain\x1b[34m)\x1b[0m ";
    const LISTING: &str = "\x1b[1;32m➜\x1b[0m \x1b[36m~/项目\x1b[0m ls -la\r\ndrwxr-xr-x   9 licha  staff   288 Nov  6 22:30 \x1b[1;34m.\x1b[0m\r\ndrwxr-xr-x  14 licha  staff   448 Nov  6 22:30 \x1b[1;34mnimbus-terminal\x1b[0m\r\ndrwxr-xr-x   6 licha  staff   192 Nov  6 21:02 \x1b[1;34maureole-framework\x1b[0m\r\n-rw-r--r--   1 licha  staff  1179 Nov  6 22:12 Cargo.toml\r\n\x1b[1;32m➜\x1b[0m \x1b[36m~/项目\x1b[0m ";
    const LOG: &str = "\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m tail -f server.log\r\n\x1b[90m22:31:07\x1b[0m \x1b[32mINFO\x1b[0m  listening on 127.0.0.1:3000\r\n\x1b[90m22:31:12\x1b[0m \x1b[32mINFO\x1b[0m  GET /api/tabs 200 3ms\r\n\x1b[90m22:31:14\x1b[0m \x1b[33mWARN\x1b[0m  slow query 412ms\r\n\x1b[90m22:31:19\x1b[0m \x1b[32mINFO\x1b[0m  GET /api/panes 200 5ms\r\n";
    let project = directory.join("项目");
    let scene = |dark: bool| {
        let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
        harness.set_scale_factor(1.5);
        workspace.form.appearance.set(if dark {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        });
        std::fs::create_dir_all(&project).expect("建得了目录");
        let folder = workspace.add_directory(&project).expect("是目录");
        workspace.create_folder("实验");
        workspace.new_tab_in(folder);
        workspace.new_tab_in(folder);
        workspace.new_tab();
        let tabs = workspace.tabs.get_untracked();
        tabs[0].session().feed_for_test(LISTING);
        tabs[1].session().feed_for_test(BUILD);
        tabs[2].session().feed_for_test(LOG);
        workspace.select(2);
        (harness, workspace)
    };
    let (harness, _workspace) = scene(false);
    save(&harness, "light.png");
    let (harness, workspace) = scene(true);
    save(&harness, "dark.png");
    // 分屏
    workspace.select(2);
    workspace.split(Axis::Columns);
    workspace.tabs.get_untracked()[1]
        .session()
        .feed_for_test(LISTING);
    workspace.split(Axis::Rows);
    workspace.tabs.get_untracked()[1]
        .session()
        .feed_for_test(LOG);
    save(&harness, "split.png");
    // 设置
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    save(&harness, "settings.png");
    let _ = std::fs::remove_dir_all(&project);
}
