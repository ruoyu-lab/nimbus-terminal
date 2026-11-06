//! 工作区的状态：有哪些标签（每个一个会话）、哪个是当前的、侧栏收起了没有、在哪个页面、
//! 命令面板开着没有、哪些后台标签有新输出。界面（`chrome`）只读这些信号、调这里的方法。
//!
//! 只有当前标签挂着视图；别的标签的会话照样在后台读输出、更新自己的屏幕，切回来时视图重新挂上，
//! 网格里的内容是最新的。

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use aureole::prelude::*;

use crate::app::{Hooks, PADDING, Session, Spawned};
use crate::{chrome, view};

/// 打开窗口时的内容区大小
pub const WINDOW_SIZE: (u32, u32) = (1040, 680);
/// macOS 红绿灯的位置，要让它在标题栏里垂直居中
pub const CONTROLS_POSITION: (f32, f32) = (16.0, 13.0);

/// 主卡片里显示哪个页面
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Terminal,
    Monitor,
}

/// 一个标签
#[derive(Clone)]
pub struct Tab {
    pub id: u64,
    pub session: Rc<Session>,
}

/// 一个窗口里的全部状态
pub struct Workspace {
    pub tabs: Signal<Vec<Tab>>,
    /// 每个标签现在显示的名字
    pub titles: Signal<HashMap<u64, String>>,
    pub active: Signal<u64>,
    /// 不在前台、读到了新输出还没看过的标签
    pub unseen: Signal<HashSet<u64>>,
    /// 最近一次的行列数：新标签按它起，起来就是对的大小；状态条也显示它
    pub size: Signal<(usize, usize)>,
    pub collapsed: Signal<bool>,
    /// 侧栏展开时的宽度，拖交界处改它
    pub sidebar_width: Signal<f32>,
    pub mode: Signal<Mode>,
    pub palette_open: Signal<bool>,
    next_id: Cell<u64>,
    spawn: Rc<dyn Fn(usize, usize) -> Spawned>,
    pub window: Option<WindowHandle>,
    this: Weak<Workspace>,
}

impl Workspace {
    pub fn new(spawn: Rc<dyn Fn(usize, usize) -> Spawned>, window: Option<WindowHandle>) -> Rc<Self> {
        let (width, height) = chrome::terminal_area();
        let size = view::fit(width - 2.0 * PADDING, height - 2.0 * PADDING, (0.0, 0.0));
        Rc::new_cyclic(|this| Self {
            tabs: signal(Vec::new()),
            titles: signal(HashMap::new()),
            active: signal(0),
            unseen: signal(HashSet::new()),
            size: signal(size),
            collapsed: signal(false),
            sidebar_width: signal(chrome::SIDEBAR_DEFAULT),
            mode: signal(Mode::Terminal),
            palette_open: signal(false),
            next_id: Cell::new(1),
            spawn,
            window,
            this: this.clone(),
        })
    }

    /// 新开一个标签并切过去
    pub fn new_tab(&self) {
        let (columns, rows) = self.size.get_untracked();
        let spawned = (self.spawn)(columns, rows);
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let fallback = spawned.name.clone();
        let (for_title, for_size, for_exit, for_activity) = (self.this.clone(), self.this.clone(), self.this.clone(), self.this.clone());
        let hooks = Hooks {
            title: Rc::new(move |title| {
                if let Some(workspace) = for_title.upgrade() {
                    let shown = if title.is_empty() { fallback.clone() } else { title.to_string() };
                    workspace.titles.update(|titles| {
                        titles.insert(id, shown);
                    });
                }
            }),
            resized: Rc::new(move |columns, rows| {
                if let Some(workspace) = for_size.upgrade() {
                    workspace.size.set((columns, rows));
                }
            }),
            exited: Rc::new(move |code| {
                // 正常退出（`exit`、Ctrl-D）标签就关；出错退出留着，屏幕上有原因
                if code == 0
                    && let Some(workspace) = for_exit.upgrade()
                {
                    workspace.close_tab(id);
                }
            }),
            activity: Rc::new(move || {
                if let Some(workspace) = for_activity.upgrade()
                    && workspace.active.get_untracked() != id
                {
                    workspace.unseen.update(|unseen| {
                        unseen.insert(id);
                    });
                }
            }),
        };
        let name = spawned.name.clone();
        let session = Session::start(spawned, columns, rows, hooks);
        self.titles.update(|titles| {
            titles.entry(id).or_insert(name);
        });
        self.tabs.update(|tabs| tabs.push(Tab { id, session }));
        self.activate(id);
    }

    /// 切到这个标签：回到终端页面，有新输出的标记清掉
    fn activate(&self, id: u64) {
        self.active.set(id);
        self.mode.set(Mode::Terminal);
        self.unseen.update(|unseen| {
            unseen.remove(&id);
        });
    }

    /// 关一个标签。最后一个关掉时关窗口
    pub fn close_tab(&self, id: u64) {
        let mut tabs = self.tabs.get_untracked();
        let Some(index) = tabs.iter().position(|tab| tab.id == id) else { return };
        let closed = tabs.remove(index);
        closed.session.close();
        self.unseen.update(|unseen| {
            unseen.remove(&id);
        });
        if tabs.is_empty() {
            if let Some(window) = self.window {
                window.close();
            }
        } else if self.active.get_untracked() == id {
            // 关的是当前这个：换到原来右边的那个，没有就换到左边的
            self.activate(tabs[index.min(tabs.len() - 1)].id);
        }
        self.titles.update(|titles| {
            titles.remove(&id);
        });
        self.tabs.set(tabs);
    }

    pub fn close_active(&self) {
        self.close_tab(self.active.get_untracked());
    }

    pub fn select(&self, id: u64) {
        if self.tabs.get_untracked().iter().any(|tab| tab.id == id) {
            self.activate(id);
        }
    }

    /// 切到第 `index` 个（0 起算）标签
    fn select_index(&self, index: usize) {
        if let Some(tab) = self.tabs.get_untracked().get(index) {
            self.activate(tab.id);
        }
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.get_untracked().iter().position(|tab| tab.id == id)
    }

    /// 把标签挪到第 `to` 个位置（拖动换位置）；已经在那里就什么也不做
    pub fn move_tab(&self, id: u64, to: usize) {
        let Some(from) = self.index_of(id) else { return };
        if from == to {
            return;
        }
        self.tabs.update(|tabs| {
            let tab = tabs.remove(from);
            tabs.insert(to.min(tabs.len()), tab);
        });
    }

    /// 往右（`1`）或往左（`-1`）切一个，到头了绕回去
    fn cycle(&self, step: isize) {
        let tabs = self.tabs.get_untracked();
        let active = self.active.get_untracked();
        let Some(index) = tabs.iter().position(|tab| tab.id == active) else { return };
        let next = (index as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.activate(tabs[next].id);
    }

    /// 跳到第一个有新输出的标签（侧栏的「需要我」）。没有就什么也不做
    pub fn jump_to_unseen(&self) {
        let unseen = self.unseen.get_untracked();
        if let Some(tab) = self.tabs.get_untracked().iter().find(|tab| unseen.contains(&tab.id)) {
            self.activate(tab.id);
        }
    }

    pub fn toggle_sidebar(&self) {
        self.collapsed.update(|collapsed| *collapsed = !*collapsed);
    }

    pub fn show_monitor(&self) {
        self.mode.set(Mode::Monitor);
    }

    /// 快捷键。macOS 用 ⌘，别的平台用 Ctrl+Shift（Ctrl 单独要留给 shell）。收下了返回 `true`
    pub fn shortcut(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if modifiers.has_alt() {
            return false;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab：各平台都一样
        if modifiers.has_control() && !modifiers.has_super() && matches!(event.key, Key::Named(NamedKey::Tab)) {
            self.cycle(if modifiers.has_shift() { -1 } else { 1 });
            return true;
        }
        let command = if modifiers.has_super() { !modifiers.has_control() } else { modifiers.has_control() && modifiers.has_shift() };
        if !command {
            return false;
        }
        let Key::Character(text) = &event.base else { return false };
        match text.as_str() {
            "t" | "T" => self.new_tab(),
            "w" | "W" => self.close_active(),
            "b" | "B" => self.toggle_sidebar(),
            "k" | "K" => self.palette_open.update(|open| *open = !*open),
            // ⌘⇧[ / ⌘⇧]
            "[" | "{" => self.cycle(-1),
            "]" | "}" => self.cycle(1),
            // ⌘1..⌘8 是第几个，⌘9 是最后一个（浏览器的习惯）
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => self.select_index(digit.parse::<usize>().unwrap_or(1) - 1),
            "9" => self.select_index(self.tabs.get_untracked().len().saturating_sub(1)),
            _ => return false,
        }
        true
    }
}

/// 窗口的根视图
pub fn root() -> Element {
    if std::env::var("TERM_NOPATCH").is_err() {
        crate::native::keep_window_still_unless_asked();
    }
    let workspace = Workspace::new(Rc::new(crate::app::spawn_login_shell), current_window());
    workspace.new_tab();
    if std::env::var("TERM_DRAGREC").is_ok() {
        let ws = workspace.clone();
        aureole::spawn_local(async move {
            aureole::sleep(ms(1200)).await;
            ws.new_tab();
            ws.new_tab();
            aureole::sleep(ms(1000)).await;
            crate::native::activate();
            aureole::sleep(ms(600)).await;
            let inset = aureole::title_bar_info().inset_left;
            let left = (inset - 8.0).max(10.0) + 2.0 + 28.0 + 2.0 + 6.0;
            let (x0, y) = (left + 82.0 + 40.0, 20.0f32);
            let epoch = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            crate::native::post_event(0, [x0, y]);
            aureole::sleep(ms(100)).await;
            crate::native::post_event(1, [x0, y]);
            aureole::sleep(ms(50)).await;
            eprintln!("DRAGREC start {}", epoch());
            let step: f32 = std::env::var("TERM_DRAGSTEP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.6);
            let amp: f32 = std::env::var("TERM_DRAGAMP").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0);
            let n = (amp / step) as usize;
            let mut x = x0;
            let t0 = std::time::Instant::now();
            let mut legs: Vec<isize> = vec![n as isize];
            for _ in 0..3 { legs.push(-2 * n as isize); legs.push(2 * n as isize); }
            legs.push(-(n as isize));
            for leg in legs {
                let dir = if leg > 0 { 1.0 } else { -1.0 };
                for _ in 0..leg.unsigned_abs() {
                    x += dir * step;
                    crate::native::post_event(2, [x, y]);
                    eprintln!("EV {} {:.1}", t0.elapsed().as_micros(), x - x0);
                    aureole::sleep(ms(8)).await;
                }
            }
            crate::native::post_event(3, [x, y]);
            eprintln!("DRAGREC end {}", epoch());
        });
    }
    if std::env::var("TERM_SELFTEST").is_ok() {
        let ws = workspace.clone();
        aureole::spawn_local(async move {
            aureole::sleep(ms(1500)).await;
            ws.new_tab();
            ws.new_tab();
            aureole::sleep(ms(800)).await;
            crate::native::activate();
            aureole::sleep(ms(500)).await;
            let inset = aureole::title_bar_info().inset_left;
            let left = (inset - 8.0).max(10.0) + 2.0 + 28.0 + 2.0 + 6.0;
            eprintln!("SELFTEST inset_left {inset} first tab left {left} frame {:?}", crate::native::window_frame());
            let (x0, y) = (left + 82.0 + 40.0, 20.0f32);
            crate::native::post_event(0, [left + 40.0, y]);
            aureole::sleep(ms(100)).await;
            crate::native::post_event(1, [left + 40.0, y]);
            aureole::sleep(ms(50)).await;
            crate::native::post_event(3, [left + 40.0, y]);
            aureole::sleep(ms(300)).await;
            eprintln!("SELFTEST click tab1 -> active {}", ws.active.get_untracked());
            let t = std::time::Instant::now();
            let before = crate::native::window_frame();
            crate::native::post_event(0, [x0, y]);
            aureole::sleep(ms(100)).await;
            crate::native::post_event(1, [x0, y]);
            aureole::sleep(ms(50)).await;
            let mut worst = std::time::Duration::ZERO;
            let mut total = std::time::Duration::ZERO;
            for i in 1..=40 {
                let t1 = std::time::Instant::now();
                crate::native::post_event(2, [x0 + i as f32 * 3.0, y]);
                aureole::sleep(ms(16)).await;
                let e = t1.elapsed();
                total += e;
                worst = worst.max(e);
            }
            eprintln!("SELFTEST step avg {:?} worst {:?} (想要 16ms)", total / 40, worst);
            crate::native::post_event(3, [x0 + 120.0, y]);
            aureole::sleep(ms(800)).await;
            // 空白处拖动窗口
            let before = crate::native::window_frame();
            let (bx, by) = (700.0f32, 20.0f32);
            crate::native::post_event(0, [bx, by]);
            aureole::sleep(ms(100)).await;
            crate::native::post_event(1, [bx, by]);
            aureole::sleep(ms(50)).await;
            for i in 1..=30 {
                crate::native::post_event(2, [bx + i as f32 * 3.0, by + i as f32 * 2.0]);
                aureole::sleep(ms(16)).await;
            }
            crate::native::post_event(3, [bx + 90.0, by + 60.0]);
            aureole::sleep(ms(600)).await;
            eprintln!("SELFTEST blank drag before {before:?} after {:?}", crate::native::window_frame());
            eprintln!("SELFTEST before {before:?} after {:?} order {:?} took {:?}", crate::native::window_frame(), ws.tabs.get_untracked().iter().map(|t| t.id).collect::<Vec<_>>(), t.elapsed());
        });
    }
    chrome::build(workspace)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aureole::MouseButton;
    use aureole::testing::Harness;

    use super::*;
    use crate::pty::{Pty, Recorder};

    /// 不起真 shell 的工作区：每个标签一个只记录写入的假 pty
    fn mounted() -> (Harness, Rc<Workspace>) {
        mounted_with(aureole::TitleBarInfo {
            height: 40.0,
            controls: Some(Rect { x: 16.0, y: 13.0, width: 54.0, height: 14.0 }),
            inset_left: 82.0,
            inset_right: 0.0,
        })
    }

    /// 用给定的标题栏度量挂（macOS 的红绿灯，或者没有系统按钮的 Windows、Linux）
    fn mounted_with(info: aureole::TitleBarInfo) -> (Harness, Rc<Workspace>) {
        let spawn: Rc<dyn Fn(usize, usize) -> Spawned> = Rc::new(|_, _| Spawned {
            pty: Some(Pty::from_backend(Arc::new(Recorder::default()))),
            queue: None,
            error: None,
            name: "zsh".to_string(),
        });
        let workspace = Workspace::new(spawn, None);
        workspace.new_tab();
        let harness = Harness::new(WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32, chrome::build(workspace.clone()));
        harness.set_title_bar_info(info);
        harness.frame();
        (harness, workspace)
    }

    fn ids(workspace: &Workspace) -> Vec<u64> {
        workspace.tabs.get_untracked().iter().map(|tab| tab.id).collect()
    }

    fn command(harness: &Harness, key: &str) {
        harness.key_down_with(Key::Character(key.into()), Modifiers::SUPER);
        harness.frame();
    }

    #[test]
    fn starts_with_one_tab_named_after_the_shell() {
        let (harness, workspace) = mounted();
        assert_eq!(ids(&workspace), [1]);
        assert_eq!(workspace.active.get_untracked(), 1);
        assert_eq!(harness.text("tab-1").as_deref(), Some("zsh"));
        assert!(harness.bounds("grid").is_some(), "当前标签的终端要挂着");
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
        assert_eq!(workspace.active.get_untracked(), 2, "关了最右的，换到左边的");
        workspace.select(1);
        command(&harness, "w");
        assert_eq!(workspace.active.get_untracked(), 2, "关了最左的，换到右边的");
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
        harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL | Modifiers::SHIFT);
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
        (tabs[1].session.hooks_for_test().exited)(0);
        assert_eq!(ids(&workspace), [1]);
        let tab = workspace.tabs.get_untracked()[0].clone();
        tab.session.exited_for_test(2);
        assert_eq!(ids(&workspace), [1], "出错退出不自动关");
    }

    /// 程序设的标题显示在标签上；空标题退回 shell 的名字
    #[test]
    fn osc_title_shows_on_the_tab() {
        let (harness, workspace) = mounted();
        let session = workspace.tabs.get_untracked()[0].session.clone();
        session.feed_for_test("\x1b]0;vim main.rs\x07");
        harness.frame();
        assert_eq!(harness.text("tab-1").as_deref(), Some("vim main.rs"));
        session.feed_for_test("\x1b]0;\x07");
        harness.frame();
        assert_eq!(harness.text("tab-1").as_deref(), Some("zsh"));
    }

    /// 标题栏的空白处按住再动是拖动窗口；双击交给系统（缩放或最小化）
    #[test]
    fn empty_title_bar_space_drags_and_double_clicks_like_a_title_bar() {
        let (harness, _workspace) = mounted();
        // 标签后面的空白（标签最宽 200，从红绿灯右边起，到 x = 600 还是空的）
        let (x, y) = (600.0, 10.0);
        harness.mouse_down(x, y, MouseButton::Left);
        harness.mouse_move(x - 20.0, y);
        assert_eq!(harness.window_drag_count(), 1, "空白处按住再动，要交给系统去拖窗口");
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
        assert!(button.y >= 0.0 && button.y + button.height <= 40.0, "按钮在标题栏的高度里");
    }

    /// 没有系统窗口按钮的平台自己画最小化、最大化、关闭，而且标签仍然从左边的小边距开始
    #[test]
    fn platforms_without_system_buttons_draw_their_own() {
        let (harness, _workspace) = mounted_with(aureole::TitleBarInfo { height: 40.0, ..Default::default() });
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

    /// 标签的尺寸：宽 80、离展开按钮 8px；关闭按钮离上、右、下都是 4px
    #[test]
    fn tab_geometry_is_tight_and_the_close_button_is_evenly_inset() {
        let (harness, _workspace) = mounted();
        let toggle = harness.bounds("toggle-sidebar").expect("按钮在");
        let tab = harness.bounds("tab-1").expect("标签在");
        let close = harness.bounds("close-1").expect("关闭按钮在");
        assert!((tab.x - (toggle.x + toggle.width) - 8.0).abs() < 0.5, "标签离按钮 {}", tab.x - (toggle.x + toggle.width));
        assert!((tab.width - 80.0).abs() < 0.5, "标签宽 {}", tab.width);
        let (top, right, bottom) = (close.y - tab.y, tab.x + tab.width - (close.x + close.width), tab.y + tab.height - (close.y + close.height));
        assert!((top - 4.0).abs() < 0.5 && (right - 4.0).abs() < 0.5 && (bottom - 4.0).abs() < 0.5, "关闭按钮的边距：上 {top} 右 {right} 下 {bottom}");
    }

    /// 按住标签向右拖过相邻的标签就换位，拖回去换回来；被拖的标签被选中
    #[test]
    fn dragging_a_tab_reorders_it() {
        let (harness, workspace) = mounted();
        workspace.new_tab();
        workspace.new_tab();
        harness.frame();
        assert_eq!(ids(&workspace), [1, 2, 3]);
        let [x, y] = harness.bounds("tab-1").expect("标签在").center();
        harness.mouse_down(x, y, MouseButton::Left);
        harness.mouse_move(x + 30.0, y);
        harness.mouse_move(x + 90.0, y);
        harness.mouse_up(MouseButton::Left);
        harness.frame();
        assert_eq!(ids(&workspace), [2, 1, 3], "向右拖了一格多");
        assert_eq!(workspace.active.get_untracked(), 1, "被拖的标签被选中");
        assert_eq!(harness.window_drag_count(), 0);
        harness.advance(secs(1.0));
        let [x, y] = harness.bounds("tab-1").expect("标签在").center();
        harness.mouse_down(x, y, MouseButton::Left);
        harness.mouse_move(x - 30.0, y);
        harness.mouse_move(x - 90.0, y);
        harness.mouse_up(MouseButton::Left);
        harness.frame();
        assert_eq!(ids(&workspace), [1, 2, 3], "往回拖换回来");
    }


    /// 拖到标签条的两头再往外拖：被拖的标签停在头尾的格子里，不被标签条裁掉
    #[test]
    fn a_dragged_tab_stays_visible_at_both_ends() {
        let (harness, workspace) = mounted();
        workspace.new_tab();
        workspace.new_tab();
        harness.frame();
        harness.advance(secs(1.0));
        let [x, y] = harness.bounds("tab-2").expect("标签在").center();
        harness.mouse_down(x, y, MouseButton::Left);
        // 选中的标签是白底：数它在标签这一行上有多少白色像素
        let white_width = |harness: &Harness| {
            let shot = harness.screenshot();
            let row = (harness.bounds("tab-2").expect("标签在").y + 8.0) as u32;
            (0..shot.width.min(600)).filter(|&px| shot.pixel(px, row).0[..3].iter().all(|&c| c > 250)).count()
        };
        for far in [200.0, -300.0] {
            for step in 1..=20 {
                harness.mouse_move(x + far * step as f32 / 20.0, y);
                harness.advance(ms(16));
            }
            assert!(white_width(&harness) >= 70, "拖到 {far} 以外，标签被裁掉了：{}", white_width(&harness));
        }
        harness.mouse_up(MouseButton::Left);
    }

    /// 标签多到放不下：标签保持 80 宽、滚动着看，竖向滚轮翻成横向；最右边仍留着空白可以拖窗口
    #[test]
    fn many_tabs_scroll_and_a_drag_area_stays_on_the_right() {
        let (harness, workspace) = mounted();
        for _ in 0..14 {
            workspace.new_tab();
        }
        harness.frame();
        harness.advance(secs(1.0));
        let tab = harness.bounds("tab-1").expect("标签在");
        assert!((tab.width - 80.0).abs() < 0.5, "标签挤满了也不压窄：{}", tab.width);
        let plus = harness.bounds("new-tab").expect("新建按钮在");
        assert!(plus.x + plus.width + 64.0 <= WINDOW_SIZE.0 as f32, "新建按钮右边 {} 后面要给拖动区留 64", plus.x + plus.width);
        // 最后一个标签是当前的，已经滚到看得见
        let last = harness.window_bounds("tab-15").expect("最后的标签在");
        assert!(last.x >= 0.0 && last.x + last.width <= plus.x + 0.5, "当前标签滚进视口：{last:?}");
        // 竖向滚轮向上（往回）：标签往右走
        harness.wheel(300.0, 20.0, 0.0, -5.0);
        harness.advance(secs(1.0));
        let moved = harness.window_bounds("tab-15").expect("最后的标签在");
        assert!(moved.x > last.x + 20.0, "滚轮把标签翻回去了：{} → {}", last.x, moved.x);
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
        let before = (harness.bounds("nav-new").expect("在").height, harness.bounds("session-1").expect("在").height, harness.bounds("mode-terminal").expect("在").height);
        for _ in 0..30 {
            workspace.new_tab();
        }
        harness.frame();
        harness.advance(secs(1.0));
        let after = (harness.bounds("nav-new").expect("在").height, harness.bounds("session-1").expect("在").height, harness.bounds("mode-terminal").expect("在").height);
        assert_eq!(before, after, "行高不变");
        let sidebar = harness.bounds("sidebar").expect("在");
        let last = harness.window_bounds("session-31").expect("最后一个会话在");
        assert!(last.y + last.height <= sidebar.y + sidebar.height + 0.5, "当前会话滚进了侧栏：{last:?}");
        // 滚轮能滚到最前面的行
        harness.wheel(60.0, 300.0, 0.0, -80.0);
        harness.advance(secs(1.0));
        let first = harness.window_bounds("nav-new").expect("在");
        assert!(first.y >= sidebar.y && first.y + first.height <= sidebar.y + sidebar.height, "滚回顶上：{first:?}");
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
        assert!((workspace.sidebar_width.get_untracked() - (width + 60.0)).abs() < 1.0, "宽度跟着指针走：{}", workspace.sidebar_width.get_untracked());
        assert_eq!(harness.window_drag_count(), 0, "拖动条上不能拖窗口");
        harness.mouse_down(edge + 60.0, 300.0, MouseButton::Left);
        harness.mouse_move(2000.0, 300.0);
        harness.mouse_up(MouseButton::Left);
        assert!(workspace.sidebar_width.get_untracked() <= 360.0, "有上限");
        harness.mouse_down(workspace.sidebar_width.get_untracked() - 4.0, 300.0, MouseButton::Left);
        harness.mouse_move(0.0, 300.0);
        harness.mouse_up(MouseButton::Left);
        assert!(workspace.sidebar_width.get_untracked() >= 176.0, "有下限");
        command(&harness, "b");
        harness.advance(secs(1.0));
        assert!(harness.bounds("sidebar-resize").is_none(), "收起时没有拖动条");
    }

    /// 后台标签有了新输出：「需要我」亮起来，点它跳过去，标记清掉；前台标签的输出不算
    #[test]
    fn background_output_marks_the_tab_and_needs_me_jumps_to_it() {
        let (harness, workspace) = mounted();
        workspace.new_tab();
        let tabs = workspace.tabs.get_untracked();
        (tabs[1].session.hooks_for_test().activity)();
        assert!(workspace.unseen.get_untracked().is_empty(), "前台标签的输出不算");
        (tabs[0].session.hooks_for_test().activity)();
        assert_eq!(workspace.unseen.get_untracked().len(), 1);
        harness.frame();
        harness.click_named("needs-me");
        harness.frame();
        assert_eq!(workspace.active.get_untracked(), 1);
        assert!(workspace.unseen.get_untracked().is_empty());
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
        let origin = ((WINDOW_SIZE.0 as f32 - panel.width) / 2.0, (WINDOW_SIZE.1 as f32 - panel.height) / 2.0);
        let [x, y] = row.center();
        harness.click(origin.0 + x, origin.1 + y);
        harness.frame();
        assert_eq!(ids(&workspace), [1, 2]);
        assert!(!workspace.palette_open.get_untracked());
    }

    /// 监控页是占位：点侧栏的开关切过去，点标签切回终端
    #[test]
    fn the_monitor_page_is_a_placeholder_and_tabs_bring_the_terminal_back() {
        let (harness, workspace) = mounted();
        harness.click_named("mode-monitor");
        harness.frame();
        assert_eq!(workspace.mode.get_untracked(), Mode::Monitor);
        assert!(harness.bounds("grid").is_none(), "监控页里没有终端");
        harness.click_named("tab-1");
        harness.frame();
        assert_eq!(workspace.mode.get_untracked(), Mode::Terminal);
        assert!(harness.bounds("grid").is_some());
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
        assert!(card.x + card.width <= 720.0 + 0.5, "卡片右边 {} 超出窗口 720", card.x + card.width);
        assert!(card.y + card.height <= 460.0 + 0.5, "卡片下边 {} 超出窗口 460", card.y + card.height);
        let grid = harness.bounds("grid").expect("终端在");
        assert!(grid.x + grid.width <= card.x + card.width + 0.5 && grid.y + grid.height <= card.y + card.height + 0.5, "终端要在卡片里面");
        let (new_columns, new_rows) = workspace.size.get_untracked();
        assert!(new_columns < columns && new_rows < rows, "行列数跟着变少：{columns}×{rows} → {new_columns}×{new_rows}");
    }

    /// 收起侧栏：不能露出快捷键提示，宽度窄到只放得下图标
    #[test]
    fn collapsed_sidebar_shows_only_icons() {
        let (harness, workspace) = mounted();
        let shown = |harness: &Harness| format!("{:?}", harness.element_tree());
        assert!(shown(&harness).contains("⌘T") && shown(&harness).contains("⌘K"), "展开时有快捷键提示");
        command(&harness, "b");
        harness.advance(secs(1.0));
        let collapsed = shown(&harness);
        assert!(!collapsed.contains("⌘T") && !collapsed.contains("⌘K"), "收起后快捷键提示不能露出来");
        assert!(!collapsed.contains("新建会话") || harness.text("nav-new").as_deref().unwrap_or("").is_empty(), "收起后文字也不留");
        let sidebar = harness.bounds("sidebar").expect("侧栏在").width;
        assert!(sidebar <= 50.0, "收起后的宽度 {sidebar}，只放图标不用这么宽");
        // 后台有新输出时，收起后「需要我」也要看得出来（一个小点），不能因为文字没了就没有提示
        workspace.new_tab();
        (workspace.tabs.get_untracked()[0].session.hooks_for_test().activity)();
        harness.frame();
        assert_eq!(workspace.unseen.get_untracked().len(), 1);
    }

    /// 评审界面用：把几种状态存成 PNG。平时不跑；
    /// `UI_SHOTS=目录 cargo test screenshots_for_review -- --ignored` 出图（离屏渲染，没有系统材质）
    #[test]
    #[ignore = "出图用，不是检查"]
    fn screenshots_for_review() {
        let Ok(directory) = std::env::var("UI_SHOTS") else { return };
        let directory = std::path::PathBuf::from(directory);
        let save = |harness: &Harness, name: &str| {
            harness.advance(secs(1.0));
            harness.screenshot().save_png(directory.join(name)).expect("存得下 PNG");
        };
        let (harness, workspace) = mounted();
        harness.set_scale_factor(2.0);
        workspace.new_tab();
        workspace.tabs.get_untracked()[0].session.feed_for_test("\x1b[32mlicha\x1b[0m \x1b[34m~/终端\x1b[0m % cargo test\r\nrunning 77 tests\r\ntest result: \x1b[32mok\x1b[0m. 77 passed\r\n");
        (workspace.tabs.get_untracked()[0].session.hooks_for_test().activity)();
        save(&harness, "1-expanded.png");
        command(&harness, "b");
        save(&harness, "2-collapsed.png");
        command(&harness, "b");
        harness.resize(720.0, 460.0);
        save(&harness, "3-smallest.png");
        harness.resize(WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32);
        command(&harness, "k");
        save(&harness, "4-palette.png");
    }
}
