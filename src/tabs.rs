//! 工作区的状态：有哪些标签（每个一个会话）、哪个是当前的、侧栏收起了没有、设置页开着没有、
//! 命令面板开着没有、哪些后台标签有新输出。界面（`chrome`）只读这些信号、调这里的方法。
//!
//! 只有当前标签挂着视图；别的标签的会话照样在后台读输出、更新自己的屏幕，切回来时视图重新挂上，
//! 网格里的内容是最新的。
//!
//! 侧栏的「目标」下面是文件夹：可以从访达拖一个目录进来，之后在这个文件夹里创建终端就从那个目录起。
//! 文件夹、开着的标签、侧栏宽度会落盘（见 [`Saved`]），设置在 [`crate::settings`]。

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use aureole::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::{Hooks, Launch, PADDING, Session, Spawned};
use crate::pane::{Axis, Pane};
use crate::settings::{
    Form, LastTab, Settings, ShellKind, SidebarStart, Startup, ThemeChoice, TitleSource,
};
use crate::store::Store;
use crate::target::Target;
use crate::{chrome, view};

/// 打开窗口时的内容区大小
pub const WINDOW_SIZE: (u32, u32) = (1040, 680);
/// macOS 红绿灯的位置，要让它在标题栏里垂直居中
pub const CONTROLS_POSITION: (f32, f32) = (16.0, 13.0);
/// 终端的默认名字
pub const DEFAULT_TITLE: &str = "终端";
/// 文件名
const STATE_FILE: &str = "state.json";
/// 每个标签最多存多少行终端内容（文件名是 `output-第几个标签.ansi`）
const OUTPUT_LINES: usize = 3000;

fn output_file(index: usize) -> String {
    format!("output-{index}.ansi")
}

/// 设置页左边的分类
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Appearance,
    Terminal,
    Shortcuts,
    About,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 5] = [
        Self::General,
        Self::Appearance,
        Self::Terminal,
        Self::Shortcuts,
        Self::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Appearance => "外观",
            Self::Terminal => "终端",
            Self::Shortcuts => "快捷键",
            Self::About => "关于",
        }
    }

    /// 界面元素的名字（测试和无障碍用）
    pub fn name(self) -> &'static str {
        match self {
            Self::General => "settings-general",
            Self::Appearance => "settings-appearance",
            Self::Terminal => "settings-terminal",
            Self::Shortcuts => "settings-shortcuts",
            Self::About => "settings-about",
        }
    }
}

/// 起一个会话：怎么起（连到哪、多少列行、哪个目录、哪个 shell）
pub type SpawnFn = Rc<dyn Fn(&Launch) -> Spawned>;

/// 一个标签
#[derive(Clone)]
pub struct Tab {
    pub id: u64,
    /// 这个标签里终端的排布：一个，或者拆分出来的好几个
    pub layout: Signal<Pane>,
    /// 聚焦的是哪个终端（排布里叶子的编号）：键盘打字、状态条、存目录都看它
    pub focus: Signal<u64>,
    pub target: Target,
    /// 归在哪个文件夹下面；没有就是直接列在「目标」下
    pub folder: Option<u64>,
}

impl Tab {
    /// 聚焦的那个终端的会话
    pub fn session(&self) -> Rc<Session> {
        let layout = self.layout.get_untracked();
        layout
            .session(self.focus.get_untracked())
            .or_else(|| layout.session(layout.first_id()))
            .expect("排布里至少有一个终端")
    }

    /// 这个标签里的所有会话
    pub fn sessions(&self) -> Vec<Rc<Session>> {
        self.layout
            .get_untracked()
            .leaves()
            .into_iter()
            .map(|(_, session)| session)
            .collect()
    }
}

/// 拖动标签时的状态：被拖的是谁、从标签条的第几格起、现在落在第几格。
/// 「现在落在第几格」按被拖的标签自己的位置算（它的边越过相邻标签的一半就换格），不看指针在哪；
/// 拖的过程中标签条的顺序不变，别的标签只靠平移让位，松手才真的换顺序
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDrag {
    pub id: u64,
    pub from: usize,
    pub to: usize,
}

/// 侧栏「目标」里的一个文件夹
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: u64,
    pub name: String,
    /// 对应的磁盘目录（从访达拖进来、或者「添加目录」选的）。在里面创建的终端从这里起；
    /// 自己新建的空文件夹没有，用设置里的起始目录
    pub path: Option<PathBuf>,
    pub open: bool,
}

/// 文件夹命名框在新建还是改名
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderPrompt {
    Create,
    Rename(u64),
}

/// 落盘的工作区：文件夹、开着的标签（按标签条的顺序）、侧栏宽度
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    folders: Vec<SavedFolder>,
    tabs: Vec<SavedTab>,
    /// 当前标签是 `tabs` 里的第几个
    active: Option<usize>,
    sidebar_width: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedFolder {
    id: u64,
    name: String,
    path: Option<PathBuf>,
    open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedTab {
    folder: Option<u64>,
    /// 用户给起的名字；没起过就是 `None`
    title: Option<String>,
    /// 关掉时聚焦的那个终端的目录（没有 `layout` 的旧文件用它）
    directory: Option<PathBuf>,
    /// 拆分的排布
    layout: Option<SavedPane>,
    /// 聚焦的是排布里的第几个终端（从 0 数，从左到右、从上到下）
    focus: Option<usize>,
}

/// 落盘的排布：一个终端（带关掉时的目录），或者一次拆分
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedPane {
    Pane {
        directory: Option<PathBuf>,
    },
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<SavedPane>,
        second: Box<SavedPane>,
    },
}

impl SavedPane {
    fn leaf_count(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// 现在的排布写成落盘的样子
    fn of(pane: &Pane) -> Self {
        match pane {
            Pane::Leaf { session, .. } => Self::Pane {
                directory: session.working_directory(),
            },
            Pane::Split {
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                axis: *axis,
                ratio: ratio.get_untracked(),
                first: Box::new(Self::of(first)),
                second: Box::new(Self::of(second)),
            },
        }
    }
}

impl SavedTab {
    fn pane_count(&self) -> usize {
        self.layout.as_ref().map_or(1, SavedPane::leaf_count)
    }
}

/// 一个窗口里的全部状态
pub struct Workspace {
    /// 全部会话，按开的先后排；侧栏照这个列，不会被挪动
    pub tabs: Signal<Vec<Tab>>,
    /// 「目标」下面的文件夹，按创建的先后
    pub folders: Signal<Vec<Folder>>,
    /// 文件夹命名框：新建，或者在改哪一个
    pub folder_prompt: Signal<Option<FolderPrompt>>,
    /// 顶部标签条里的顺序（会话的 id）：拖动只改它，和侧栏的顺序无关
    pub bar: Signal<Vec<u64>>,
    /// 正在拖着的标签；没拖时是 `None`
    pub tab_drag: Signal<Option<TabDrag>>,
    /// 每个标签现在显示的名字
    pub titles: Signal<HashMap<u64, String>>,
    /// 当前会话；一个会话都没有时是 0。设置页开着时它还是进设置之前的那个
    pub active: Signal<u64>,
    /// 不在前台、读到了新输出还没看过的标签
    pub unseen: Signal<HashSet<u64>>,
    /// 最近一次的行列数：新标签按它起，起来就是对的大小；状态条也显示它
    pub size: Signal<(usize, usize)>,
    pub collapsed: Signal<bool>,
    /// 侧栏展开时的宽度，拖交界处改它
    pub sidebar_width: Signal<f32>,
    pub palette_open: Signal<bool>,
    /// 设置页开着：卡片里是设置，顶上的标签条收起，侧栏换成设置的分类
    pub settings_open: Signal<bool>,
    /// 设置页里选中的分类
    pub settings_page: Signal<SettingsPage>,
    /// 设置页上的选择：会话、窗口、界面都读它
    pub form: Form,
    /// 重命名框开着时，是在改哪个标签
    pub renaming: Signal<Option<u64>>,
    /// 确认关闭框开着时，是在关哪个标签（它前台有程序在跑）
    pub closing: Signal<Option<u64>>,
    /// 确认关闭框是只关这个标签里的某个终端（拆分出来的）时，是哪个；`None` 是关整个标签
    closing_pane: Cell<Option<u64>>,
    /// 用户起的名字：有就盖过程序设的标题
    custom_titles: RefCell<HashMap<u64, String>>,
    /// 程序设的标题（可能是空的）
    reported_titles: RefCell<HashMap<u64, String>>,
    next_id: Cell<u64>,
    next_pane: Cell<u64>,
    next_folder: Cell<u64>,
    spawn: SpawnFn,
    pub window: Option<WindowHandle>,
    store: Store,
    /// 排着一次延后的落盘
    save_pending: Cell<bool>,
    /// 已经写到盘上的终端内容：第几个标签的文件写的是哪个会话的哪一版，没变就不重写
    saved_outputs: RefCell<HashMap<usize, (u64, u64)>>,
    /// 盘上有多少个终端内容文件（多出来的要删）
    output_files: Cell<usize>,
    /// 退出时最后存过一次了：之后不再存（窗口关了以后 shell 收到 SIGHUP 退出，
    /// 屏幕上会多出「进程已退出」，不能把它存进去）
    finished: Cell<bool>,
    this: Weak<Workspace>,
}

impl Workspace {
    /// `store` 是落盘的位置（`Store::none()` 不落盘）。设置和上次的文件夹、侧栏宽度从里面读；
    /// 上次开着的标签要调 [`Workspace::restore`] 才会开出来
    pub fn new(spawn: SpawnFn, window: Option<WindowHandle>, store: Store) -> Rc<Self> {
        // 工作区的信号和副作用不挂在窗口的作用域上：窗口一关作用域就释放，
        // 而退出时还要最后存一次（读这些信号）
        aureole::detached(|| Self::build(spawn, window, store))
    }

    fn build(spawn: SpawnFn, window: Option<WindowHandle>, store: Store) -> Rc<Self> {
        let settings = Settings::load(&store);
        let saved: Saved = store.load(STATE_FILE);
        let (width, height) =
            window.map_or((WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32), |window| {
                let [width, height] = window.size();
                (width, height)
            });
        let (width, height) = chrome::terminal_area((width, height));
        let size = view::fit(width - 2.0 * PADDING, height - 2.0 * PADDING, (0.0, 0.0));
        let folders: Vec<Folder> = saved
            .folders
            .iter()
            .map(|folder| Folder {
                id: folder.id,
                name: folder.name.clone(),
                path: folder.path.clone(),
                open: folder.open,
            })
            .collect();
        let next_folder = folders.iter().map(|folder| folder.id).max().unwrap_or(0) + 1;
        let sidebar_width = saved
            .sidebar_width
            .unwrap_or(chrome::SIDEBAR_DEFAULT)
            .clamp(chrome::SIDEBAR_MIN, chrome::SIDEBAR_MAX);
        let workspace = Rc::new_cyclic(|this| Self {
            tabs: signal(Vec::new()),
            folders: signal(folders),
            folder_prompt: signal(None),
            bar: signal(Vec::new()),
            tab_drag: signal(None),
            titles: signal(HashMap::new()),
            active: signal(0),
            unseen: signal(HashSet::new()),
            size: signal(size),
            collapsed: signal(settings.sidebar == SidebarStart::Closed),
            sidebar_width: signal(sidebar_width),
            palette_open: signal(false),
            settings_open: signal(false),
            settings_page: signal(SettingsPage::General),
            form: Form::new(&settings),
            renaming: signal(None),
            closing: signal(None),
            closing_pane: Cell::new(None),
            custom_titles: RefCell::new(HashMap::new()),
            reported_titles: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            next_pane: Cell::new(1),
            next_folder: Cell::new(next_folder),
            spawn,
            window,
            store,
            save_pending: Cell::new(false),
            saved_outputs: RefCell::new(HashMap::new()),
            output_files: Cell::new(saved.tabs.iter().map(SavedTab::pane_count).sum()),
            finished: Cell::new(false),
            this: this.clone(),
        });
        workspace.watch_theme();
        workspace.watch_settings();
        workspace.watch_persistence();
        workspace
    }

    /// 界面现在是深色还是浅色：设置里的「外观」，选了「跟随系统」就看系统的外观。
    /// 要在建界面之前就定下来（所以在 `new` 里马上跑一次）
    fn watch_theme(self: &Rc<Self>) {
        let appearance = self.form.appearance;
        Effect::new(move || {
            // 窗口外框的深浅（Windows 的 Mica）由框架按这个外观画，要和界面自己的深浅一致
            let (mode, dark) = match appearance.get() {
                ThemeChoice::Light => (AppearanceMode::Light, false),
                ThemeChoice::Dark => (AppearanceMode::Dark, true),
                ThemeChoice::System => (
                    AppearanceMode::System,
                    aureole::system_appearance() == aureole::Appearance::Dark,
                ),
            };
            aureole::set_appearance_mode(mode);
            crate::theme::set_dark(dark);
        });
    }

    /// 设置里和已经开着的标签有关的几项：标题来源、历史行数。改了就照新的来
    fn watch_settings(self: &Rc<Self>) {
        let (this, form) = (self.this.clone(), self.form);
        Effect::new(move || {
            let _ = form.title.get();
            if let Some(workspace) = this.upgrade() {
                for id in workspace.bar.get_untracked() {
                    workspace.refresh_title(id);
                }
            }
        });
        let this = self.this.clone();
        Effect::new(move || {
            let _ = form.scrollback.get();
            if let Some(workspace) = this.upgrade() {
                for tab in workspace.tabs.get_untracked() {
                    for session in tab.sessions() {
                        session.apply_scrollback();
                    }
                }
            }
        });
        form.persist(self.store.clone());
    }

    // ---- 落盘 ----

    /// 工作区一变就存（稍等一下合并连续的改动）
    fn watch_persistence(self: &Rc<Self>) {
        if !self.store.is_persistent() {
            return;
        }
        let this = self.this.clone();
        let (folders, tabs, bar, titles, active, width) = (
            self.folders,
            self.tabs,
            self.bar,
            self.titles,
            self.active,
            self.sidebar_width,
        );
        Effect::new(move || {
            let _ = (
                folders.get(),
                tabs.get(),
                bar.get(),
                titles.get(),
                active.get(),
                width.get(),
            );
            if let Some(workspace) = this.upgrade() {
                workspace.save_soon();
            }
        });
    }

    /// 每隔 2 秒存一次：shell 的当前目录变了没有信号，只能定时去问
    fn autosave(&self) {
        if !self.store.is_persistent() {
            return;
        }
        let this = self.this.clone();
        // 任务不挂在任何界面上：窗口里的东西卸载不能把它一起取消
        aureole::detached(|| {
            aureole::spawn_local(async move {
                loop {
                    aureole::sleep(secs(2.0)).await;
                    let Some(workspace) = this.upgrade() else {
                        break;
                    };
                    workspace.save_now();
                }
            })
        });
    }

    fn save_soon(&self) {
        if self.save_pending.replace(true) {
            return;
        }
        let this = self.this.clone();
        aureole::detached(|| {
            aureole::spawn_local(async move {
                aureole::sleep(ms(300)).await;
                if let Some(workspace) = this.upgrade() {
                    workspace.save_pending.set(false);
                    workspace.save_now();
                }
            })
        });
    }

    /// 现在的工作区，写成要落盘的样子（标签按标签条的顺序）
    fn snapshot(&self) -> Saved {
        let tabs = self.tabs.get_untracked();
        let customs = self.custom_titles.borrow();
        let bar = self.bar.get_untracked();
        let ordered: Vec<&Tab> = bar
            .iter()
            .filter_map(|id| tabs.iter().find(|tab| tab.id == *id))
            .collect();
        let active = self.active.get_untracked();
        Saved {
            folders: self
                .folders
                .get_untracked()
                .iter()
                .map(|folder| SavedFolder {
                    id: folder.id,
                    name: folder.name.clone(),
                    path: folder.path.clone(),
                    open: folder.open,
                })
                .collect(),
            tabs: ordered
                .iter()
                .map(|tab| {
                    let layout = tab.layout.get_untracked();
                    let focus = tab.focus.get_untracked();
                    SavedTab {
                        folder: tab.folder,
                        title: customs.get(&tab.id).cloned(),
                        directory: tab.session().working_directory(),
                        focus: layout.leaves().iter().position(|(id, _)| *id == focus),
                        layout: Some(SavedPane::of(&layout)),
                    }
                })
                .collect(),
            active: ordered.iter().position(|tab| tab.id == active),
            sidebar_width: Some(self.sidebar_width.get_untracked()),
        }
    }

    /// 要退出了：最后存一次，之后不再存
    pub fn finish(&self) {
        self.save_now();
        self.finished.set(true);
    }

    /// 配置存在哪个目录；不落盘时是 `None`
    pub fn config_directory(&self) -> Option<PathBuf> {
        self.store.directory().map(Path::to_path_buf)
    }

    /// 马上存。没变就不写
    pub fn save_now(&self) {
        if !self.store.is_persistent() || self.finished.get() {
            return;
        }
        self.store.save(STATE_FILE, &self.snapshot());
        self.save_outputs();
    }

    /// 存每个终端里的内容（按标签条的顺序，标签里从左到右、从上到下，和 `state.json` 里的排布一一对应；
    /// 文件名里的序号是这个终端在全部终端里排第几）。设置里关了就把存过的删掉；自上次存以后没有新输出的不重写
    fn save_outputs(&self) {
        let enabled = self.form.save_output.get_untracked();
        let tabs = self.tabs.get_untracked();
        let bar = self.bar.get_untracked();
        let panes: Vec<(u64, Rc<Session>)> = if enabled {
            bar.iter()
                .filter_map(|id| tabs.iter().find(|tab| tab.id == *id))
                .flat_map(|tab| tab.layout.get_untracked().leaves())
                .collect()
        } else {
            Vec::new()
        };
        let mut saved = self.saved_outputs.borrow_mut();
        for (index, (pane, session)) in panes.iter().enumerate() {
            let stamp = (*pane, session.output_version());
            if saved.get(&index) == Some(&stamp) {
                continue;
            }
            let text = session.transcript(OUTPUT_LINES);
            if text.is_empty() {
                self.store.remove(&output_file(index));
            } else {
                self.store.save_text(&output_file(index), &text);
            }
            saved.insert(index, stamp);
        }
        for index in panes.len()..self.output_files.get().max(saved.len()) {
            self.store.remove(&output_file(index));
            saved.remove(&index);
        }
        self.output_files.set(panes.len());
    }

    /// 把上次开着的标签（连同拆分、每个终端的目录和里面的内容）再开出来，选中上次选中的那个。
    /// 设置里「启动时」选了空白就什么也不做。返回开了几个标签
    pub fn restore(&self) -> usize {
        if self.form.startup.get_untracked() != Startup::Restore {
            return 0;
        }
        let saved: Saved = self.store.load(STATE_FILE);
        let folders = self.folders.get_untracked();
        let keep_output = self.form.save_output.get_untracked();
        // 终端内容的文件按全部终端的先后编号，和存的时候的顺序一样
        let mut flat = 0usize;
        let mut opened = Vec::new();
        for tab in &saved.tabs {
            // 文件夹被删过的，标签放回「目标」下面
            let folder = tab
                .folder
                .filter(|id| folders.iter().any(|folder| folder.id == *id));
            let tab_id = self.new_id();
            let layout = tab.layout.clone().unwrap_or(SavedPane::Pane {
                directory: tab.directory.clone(),
            });
            let pane = self.build_pane(tab_id, folder, &layout, keep_output, &mut flat);
            let focus = tab
                .focus
                .and_then(|index| pane.leaves().get(index).map(|(id, _)| *id))
                .unwrap_or_else(|| pane.first_id());
            self.push_tab(tab_id, folder, pane, focus);
            if let Some(title) = &tab.title {
                self.rename_tab(tab_id, title);
            }
            opened.push(tab_id);
        }
        // 上次选中的那个；没记下来就是第一个
        if let Some(&id) = saved
            .active
            .and_then(|index| opened.get(index))
            .or(opened.first())
        {
            self.activate(id);
        }
        opened.len()
    }

    /// 按落盘的排布把终端一个个开出来
    fn build_pane(
        &self,
        tab: u64,
        folder: Option<u64>,
        saved: &SavedPane,
        keep_output: bool,
        flat: &mut usize,
    ) -> Pane {
        match saved {
            SavedPane::Pane { directory } => {
                let history = keep_output
                    .then(|| self.store.load_text(&output_file(*flat)))
                    .flatten();
                *flat += 1;
                let (id, session) = self.spawn_pane(tab, folder, directory.clone(), history);
                Pane::leaf(id, session)
            }
            SavedPane::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.build_pane(tab, folder, first, keep_output, flat);
                let second = self.build_pane(tab, folder, second, keep_output, flat);
                Pane::split_node(
                    *axis,
                    ratio.clamp(crate::pane::RATIO_RANGE.0, crate::pane::RATIO_RANGE.1),
                    first,
                    second,
                )
            }
        }
    }

    // ---- 新建、文件夹 ----

    /// 新开一个本机标签并切过去。当前会话在某个文件夹里时，新会话也归到那里
    pub fn new_tab(&self) {
        let folder = self
            .tabs
            .get_untracked()
            .iter()
            .find(|tab| tab.id == self.active.get_untracked())
            .and_then(|tab| tab.folder);
        let id = self.open(folder, None, None);
        self.activate(id);
    }

    /// 新开一个不归任何文件夹的终端（直接列在「目标」下），用设置里的起始目录
    pub fn new_tab_loose(&self) {
        let id = self.open(None, None, None);
        self.activate(id);
    }

    /// 在这个文件夹里新开一个本机终端，并把它展开。文件夹有目录就从那个目录起
    pub fn new_tab_in(&self, folder: u64) {
        self.reveal_folder(folder);
        let id = self.open(Some(folder), None, None);
        self.activate(id);
    }

    /// 新建一个文件夹，名字空着就用「文件夹」「文件夹 2」这样排下去。这样的文件夹没有对应的目录
    pub fn create_folder(&self, name: &str) -> u64 {
        let name = name.trim();
        let name = if name.is_empty() {
            self.fresh_folder_name()
        } else {
            name.to_string()
        };
        self.push_folder(name, None)
    }

    fn push_folder(&self, name: String, path: Option<PathBuf>) -> u64 {
        let id = self.next_folder.get();
        self.next_folder.set(id + 1);
        self.folders.update(|folders| {
            folders.push(Folder {
                id,
                name,
                path,
                open: true,
            });
        });
        id
    }

    /// 把一个磁盘目录加进「目标」（从访达拖进来、或者选的）：文件夹用目录的名字，
    /// 之后在里面创建的终端从这个目录起。这个目录已经有文件夹了就只是展开它。
    /// 不是目录（文件、不存在）返回 `None`
    pub fn add_directory(&self, path: &Path) -> Option<u64> {
        if !path.is_dir() {
            return None;
        }
        let same = |other: &Path| {
            other == path
                || matches!(
                    (other.canonicalize(), path.canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        };
        let existing = self
            .folders
            .get_untracked()
            .iter()
            .find(|folder| folder.path.as_deref().is_some_and(same))
            .map(|folder| folder.id);
        if let Some(id) = existing {
            self.reveal_folder(id);
            return Some(id);
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Some(self.push_folder(name, Some(path.to_path_buf())))
    }

    /// 弹出系统的选择目录对话框，选好的目录加进「目标」并在里面开一个终端
    pub fn choose_directory(&self) {
        let this = self.this.clone();
        // 对话框的任务不能记在按钮或菜单上：选完马上要换界面，它们的作用域会被卸掉
        aureole::detached(|| {
            aureole::spawn_local(async move {
                let mut dialog = aureole::FileDialog::new().title("选择目录");
                if let Some(home) = crate::store::user_home() {
                    dialog = dialog.directory(home);
                }
                let Some(path) = dialog.pick_directory().await else {
                    return;
                };
                if let Some(workspace) = this.upgrade()
                    && let Some(folder) = workspace.add_directory(&path)
                {
                    workspace.new_tab_in(folder);
                }
            })
        });
    }

    /// 还没被占用的默认文件夹名
    pub fn fresh_folder_name(&self) -> String {
        let names: Vec<String> = self
            .folders
            .get_untracked()
            .iter()
            .map(|folder| folder.name.clone())
            .collect();
        let mut candidate = "文件夹".to_string();
        let mut number = 2u32;
        while names.iter().any(|name| name == &candidate) {
            candidate = format!("文件夹 {number}");
            number += 1;
        }
        candidate
    }

    pub fn rename_folder(&self, id: u64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.name = name.to_string();
            }
        });
    }

    /// 删掉文件夹，里面的会话一起关掉
    pub fn delete_folder(&self, id: u64) {
        let sessions: Vec<u64> = self
            .tabs
            .get_untracked()
            .iter()
            .filter(|tab| tab.folder == Some(id))
            .map(|tab| tab.id)
            .collect();
        for session in sessions {
            self.close_tab(session);
        }
        self.folders
            .update(|folders| folders.retain(|folder| folder.id != id));
        if self.folder_prompt.get_untracked() == Some(FolderPrompt::Rename(id)) {
            self.folder_prompt.set(None);
        }
    }

    pub fn toggle_folder(&self, id: u64) {
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.open = !folder.open;
            }
        });
    }

    fn reveal_folder(&self, id: u64) {
        let closed = self
            .folders
            .get_untracked()
            .iter()
            .any(|folder| folder.id == id && !folder.open);
        if !closed {
            return;
        }
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.open = true;
            }
        });
    }

    // ---- 起会话 ----

    /// 新会话从哪个目录起：`explicit`（恢复上次时用关掉时的目录）、所在文件夹的目录、
    /// 设置里指定的起始目录，依次找第一个真是目录的；都没有就是 `None`（主目录）
    fn start_directory(&self, folder: Option<u64>, explicit: Option<PathBuf>) -> Option<PathBuf> {
        let from_folder = folder.and_then(|id| {
            self.folders
                .get_untracked()
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.path.clone())
        });
        let from_settings = (self.form.directory.get_untracked()
            == crate::settings::DirectoryKind::Custom)
            .then(|| expand_home(self.form.directory_path.get_untracked().trim()))
            .flatten();
        [explicit, from_folder, from_settings]
            .into_iter()
            .flatten()
            .find(|path| path.is_dir())
    }

    fn new_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    /// 开一个标签（里面一个终端），放进标签条和侧栏，但不切过去。返回它的 id
    fn open(
        &self,
        folder: Option<u64>,
        directory: Option<PathBuf>,
        history: Option<String>,
    ) -> u64 {
        let id = self.new_id();
        let (pane, session) = self.spawn_pane(id, folder, directory, history);
        self.push_tab(id, folder, Pane::leaf(pane, session), pane);
        id
    }

    /// 起一个终端（会话），它属于标签 `tab`。返回它在标签里的编号和会话
    fn spawn_pane(
        &self,
        tab: u64,
        folder: Option<u64>,
        directory: Option<PathBuf>,
        history: Option<String>,
    ) -> (u64, Rc<Session>) {
        let (columns, rows) = self.size.get_untracked();
        let shell = (self.form.shell.get_untracked() == ShellKind::Program)
            .then(|| self.form.shell_program.get_untracked().trim().to_string())
            .filter(|program| !program.is_empty());
        let launch = Launch {
            target: Target::Local,
            columns,
            rows,
            directory: self.start_directory(folder, directory),
            shell,
            dark_background: crate::theme::dark().get_untracked(),
        };
        let spawned = (self.spawn)(&launch);
        let pane = self.next_pane.get();
        self.next_pane.set(pane + 1);
        let (for_title, for_size, for_exit, for_activity) = (
            self.this.clone(),
            self.this.clone(),
            self.this.clone(),
            self.this.clone(),
        );
        let hooks = Hooks {
            title: Rc::new(move |title| {
                if let Some(workspace) = for_title.upgrade() {
                    workspace.set_reported_title(tab, title.to_string());
                }
            }),
            resized: Rc::new(move |columns, rows| {
                if let Some(workspace) = for_size.upgrade() {
                    workspace.size.set((columns, rows));
                }
            }),
            exited: Rc::new(move |code| {
                // 正常退出（`exit`、Ctrl-D）这个终端就关（拆分出来的只关它，最后一个关标签）；出错退出留着，屏幕上有原因
                if code == 0
                    && let Some(workspace) = for_exit.upgrade()
                {
                    workspace.close_pane(tab, pane);
                }
            }),
            activity: Rc::new(move || {
                if let Some(workspace) = for_activity.upgrade()
                    && workspace.active.get_untracked() != tab
                {
                    workspace.unseen.update(|unseen| {
                        unseen.insert(tab);
                    });
                }
            }),
        };
        let session = Session::start(spawned, columns, rows, hooks, self.form, history.as_deref());
        (pane, session)
    }

    /// 把排布放进新的标签：进标签条和侧栏的末尾
    fn push_tab(&self, id: u64, folder: Option<u64>, layout: Pane, focus: u64) {
        self.refresh_title(id);
        // 信号放在不挂界面的作用域里：标签是在点击、快捷键、命令面板的处理器里开的，
        // 记在它们的作用域上，一会儿就释放了
        let (layout, focus) = aureole::detached(|| (signal(layout), signal(focus)));
        self.tabs.update(|tabs| {
            tabs.push(Tab {
                id,
                layout,
                focus,
                target: Target::Local,
                folder,
            })
        });
        self.bar.update(|bar| bar.push(id));
    }

    // ---- 拆分 ----

    /// 把当前标签里聚焦的终端拆开：新终端在右边（左右分）或下边（上下分），从聚焦的那个终端现在的目录起，
    /// 并且聚焦到它。设置页开着、没有标签时什么也不做
    pub fn split(&self, axis: Axis) {
        if self.settings_open.get_untracked() {
            return;
        }
        let Some(tab) = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|tab| tab.id == self.active.get_untracked())
        else {
            return;
        };
        let directory = tab.session().working_directory();
        let (pane, session) = self.spawn_pane(tab.id, tab.folder, directory, None);
        let layout = tab.layout.get_untracked();
        tab.layout
            .set(layout.split(tab.focus.get_untracked(), axis, Pane::leaf(pane, session)));
        tab.focus.set(pane);
    }

    /// 聚焦到标签里的这个终端（点了它）
    pub fn focus_pane(&self, tab: u64, pane: u64) {
        if let Some(tab) = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|candidate| candidate.id == tab)
            && tab.focus.get_untracked() != pane
        {
            tab.focus.set(pane);
        }
    }

    /// 关标签里的一个终端，不问。是最后一个就关整个标签
    pub fn close_pane(&self, tab_id: u64, pane: u64) {
        let Some(tab) = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let layout = tab.layout.get_untracked();
        let Some((rest, next_focus)) = layout.remove(pane) else {
            self.close_tab(tab_id);
            return;
        };
        if let Some(session) = layout.session(pane) {
            session.close();
        }
        if tab.focus.get_untracked() == pane {
            tab.focus.set(next_focus);
        }
        tab.layout.set(rest);
    }

    /// 要关标签里的一个终端：开了「关闭前确认」而且它前台有程序在跑时先问
    pub fn request_close_pane(&self, tab_id: u64, pane: u64) {
        let busy = self.form.confirm_close.get_untracked()
            && self
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| tab.layout.get_untracked().session(pane))
                .is_some_and(|session| session.is_busy());
        if busy {
            self.closing_pane.set(Some(pane));
            self.closing.set(Some(tab_id));
        } else {
            self.close_pane(tab_id, pane);
        }
    }

    /// 确认框里点了「关闭」：关它问的那个标签（或者标签里的那个终端）
    pub fn confirm_close(&self) {
        let Some(tab) = self.closing.get_untracked() else {
            return;
        };
        let pane = self.closing_pane.take();
        self.closing.set(None);
        match pane {
            Some(pane) => self.close_pane(tab, pane),
            None => self.close_tab(tab),
        }
    }

    // ---- 标题 ----

    /// 程序（shell、vim）设了标题：记下来；按设置决定显示不显示、用户给这个标签起过名字时不显示它
    fn set_reported_title(&self, id: u64, title: String) {
        self.reported_titles.borrow_mut().insert(id, title);
        self.refresh_title(id);
    }

    /// 重新算这个标签显示什么：用户起的名字，其次（设置允许时）程序报告的标题，最后是「终端」
    fn refresh_title(&self, id: u64) {
        let shown = if let Some(custom) = self.custom_titles.borrow().get(&id) {
            custom.clone()
        } else {
            let reported = self
                .reported_titles
                .borrow()
                .get(&id)
                .cloned()
                .unwrap_or_default();
            if self.form.title.get_untracked() == TitleSource::Reported && !reported.is_empty() {
                reported
            } else {
                DEFAULT_TITLE.to_string()
            }
        };
        self.titles.update(|titles| {
            titles.insert(id, shown);
        });
    }

    /// 给标签起名，之后程序设的标题不再改它。名字去掉首尾空白；空的就清掉自己起的名字，退回默认
    pub fn rename_tab(&self, id: u64, name: &str) {
        if !self.has_tab(id) {
            return;
        }
        let name = name.trim();
        if name.is_empty() {
            self.custom_titles.borrow_mut().remove(&id);
        } else {
            self.custom_titles.borrow_mut().insert(id, name.to_string());
        }
        self.refresh_title(id);
    }

    // ---- 切换 ----

    /// 切到这个标签：回到终端页面；有新输出的标记清掉
    fn activate(&self, id: u64) {
        self.settings_open.set(false);
        self.show(id);
    }

    fn show(&self, id: u64) {
        self.active.set(id);
        self.unseen.update(|unseen| {
            unseen.remove(&id);
        });
        if let Some(folder) = self
            .tabs
            .get_untracked()
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.folder)
        {
            self.reveal_folder(folder);
        }
    }

    /// 打开设置页：顶上的标签条收起，侧栏换成设置的分类。当前会话不变，退出设置就回到它
    pub fn open_settings(&self) {
        self.settings_open.set(true);
    }

    /// 退出设置页，回到进来之前的会话（或者「没有标签」）
    pub fn close_settings(&self) {
        self.settings_open.set(false);
    }

    pub fn toggle_settings(&self) {
        self.settings_open.update(|open| *open = !*open);
    }

    // ---- 关闭 ----

    /// 用户要关一个标签（点关闭、⌘W、右键删除）：设置里开了「关闭前确认」而且它前台有程序在跑时先问一下，
    /// 否则直接关
    pub fn request_close(&self, id: u64) {
        let busy = self.form.confirm_close.get_untracked()
            && self
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.id == id)
                .is_some_and(|tab| tab.sessions().iter().any(|session| session.is_busy()));
        if busy {
            self.closing_pane.set(None);
            self.closing.set(Some(id));
        } else {
            self.close_tab(id);
        }
    }

    /// 关一个标签，不问。最后一个关掉时按设置：留着空窗口，或者退出应用
    pub fn close_tab(&self, id: u64) {
        let mut tabs = self.tabs.get_untracked();
        let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let closed = tabs.remove(index);
        for session in closed.sessions() {
            session.close();
        }
        self.unseen.update(|unseen| {
            unseen.remove(&id);
        });
        let mut bar = self.bar.get_untracked();
        let slot = bar.iter().position(|&shown| shown == id);
        bar.retain(|&shown| shown != id);
        if self.active.get_untracked() == id {
            // 关的是当前这个：换到标签条上原来右边的那个，没有就换到左边的。一个都不剩就空着
            if bar.is_empty() {
                self.active.set(0);
            } else {
                let slot = slot.unwrap_or(0).min(bar.len() - 1);
                self.show(bar[slot]);
            }
        }
        self.titles.update(|titles| {
            titles.remove(&id);
        });
        self.custom_titles.borrow_mut().remove(&id);
        self.reported_titles.borrow_mut().remove(&id);
        for prompt in [self.renaming, self.closing] {
            if prompt.get_untracked() == Some(id) {
                prompt.set(None);
            }
        }
        let empty = tabs.is_empty();
        self.bar.set(bar);
        self.tabs.set(tabs);
        if empty && self.form.last_tab.get_untracked() == LastTab::Quit {
            self.finish();
            aureole::quit();
        }
    }

    /// ⌘W：设置页开着时是退出设置，否则关当前标签
    pub fn close_active(&self) {
        if self.settings_open.get_untracked() {
            self.close_settings();
        } else {
            let active = self.active.get_untracked();
            let focus = self
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.id == active)
                .filter(|tab| tab.layout.get_untracked().leaves().len() > 1)
                .map(|tab| tab.focus.get_untracked());
            match focus {
                // 拆分过的标签：⌘W 只关聚焦的那个终端，最后一个才关标签
                Some(pane) => self.request_close_pane(active, pane),
                None => self.request_close(active),
            }
        }
    }

    fn has_tab(&self, id: u64) -> bool {
        self.tabs.get_untracked().iter().any(|tab| tab.id == id)
    }

    /// 点标签条、侧栏上的会话：切过去（设置页开着就退出设置）
    pub fn select(&self, id: u64) {
        if self.has_tab(id) {
            self.activate(id);
        }
    }

    /// 切到标签条上第 `index` 个（0 起算）标签
    fn select_index(&self, index: usize) {
        if let Some(&id) = self.bar.get_untracked().get(index) {
            self.select(id);
        }
    }

    /// 标签在顶部标签条上的位置
    pub fn bar_index(&self, id: u64) -> Option<usize> {
        self.bar
            .get_untracked()
            .iter()
            .position(|&shown| shown == id)
    }

    /// 拖着的标签放下了：按拖动时算好的格子换顺序，并选中它。
    /// 先选中、再清拖动状态：标签的选中样式在拖动中看的是「谁被拖」，清掉以后看的是「谁是当前」，
    /// 反过来做的话，原来的当前标签会在中间闪回选中再淡出；换顺序时别的标签的格子各挪了一格，
    /// 让位的平移要同时归零，不然画面会跳
    pub fn drop_tab(&self, id: u64) {
        let Some(dragged) = self
            .tab_drag
            .get_untracked()
            .filter(|dragged| dragged.id == id)
        else {
            return;
        };
        self.select(id);
        aureole::without_animation(|| {
            self.tab_drag.set(None);
            self.move_tab(id, dragged.to);
        });
    }

    /// 把标签挪到标签条上第 `to` 个位置（拖动换位置）；只动顶部的顺序，侧栏不变。已经在那里就什么也不做
    pub fn move_tab(&self, id: u64, to: usize) {
        let Some(from) = self.bar_index(id) else {
            return;
        };
        if from == to {
            return;
        }
        self.bar.update(|bar| {
            let moved = bar.remove(from);
            bar.insert(to.min(bar.len()), moved);
        });
    }

    /// 按标签条的顺序往右（`1`）或往左（`-1`）切一个，到头了绕回去
    fn cycle(&self, step: isize) {
        let bar = self.bar.get_untracked();
        if bar.is_empty() {
            return;
        }
        let next = match self.bar_index(self.active.get_untracked()) {
            Some(index) => (index as isize + step).rem_euclid(bar.len() as isize) as usize,
            None => 0,
        };
        self.select(bar[next]);
    }

    pub fn toggle_sidebar(&self) {
        self.collapsed.update(|collapsed| *collapsed = !*collapsed);
    }

    /// 快捷键。macOS 用 ⌘，别的平台用 Ctrl+Shift（Ctrl 单独要留给 shell）。收下了返回 `true`
    pub fn shortcut(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if modifiers.has_alt() {
            return false;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab：各平台都一样
        if modifiers.has_control()
            && !modifiers.has_super()
            && matches!(event.key, Key::Named(NamedKey::Tab))
        {
            self.cycle(if modifiers.has_shift() { -1 } else { 1 });
            return true;
        }
        let command = if modifiers.has_super() {
            !modifiers.has_control()
        } else {
            modifiers.has_control() && modifiers.has_shift()
        };
        if !command {
            return false;
        }
        let Key::Character(text) = &event.base else {
            return false;
        };
        match text.as_str() {
            "t" | "T" => self.new_tab(),
            "w" | "W" => self.close_active(),
            "b" | "B" => self.toggle_sidebar(),
            // ⌘D 左右分，⌘⇧D 上下分（iTerm 的习惯）
            "d" | "D" => self.split(if modifiers.has_shift() {
                Axis::Rows
            } else {
                Axis::Columns
            }),
            "k" | "K" => self.palette_open.update(|open| *open = !*open),
            "," => self.toggle_settings(),
            // ⌘⇧[ / ⌘⇧]
            "[" | "{" => self.cycle(-1),
            "]" | "}" => self.cycle(1),
            // ⌘1..⌘8 是第几个，⌘9 是最后一个（浏览器的习惯）
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => {
                self.select_index(digit.parse::<usize>().unwrap_or(1) - 1)
            }
            "9" => self.select_index(self.bar.get_untracked().len().saturating_sub(1)),
            _ => return false,
        }
        true
    }
}

/// `~`、`~/xx`、`~\xx` 展开成主目录；空串是 `None`。`~名字` 是另一个用户的目录，这里不展开
fn expand_home(text: &str) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    let Some(rest) = text.strip_prefix('~') else {
        return Some(PathBuf::from(text));
    };
    let Some(home) = crate::store::user_home() else {
        return Some(PathBuf::from(text));
    };
    if rest.is_empty() {
        return Some(home);
    }
    if rest.starts_with(['/', '\\']) {
        Some(home.join(rest.trim_start_matches(['/', '\\'])))
    } else {
        Some(PathBuf::from(text))
    }
}

thread_local! {
    /// 退出应用时要最后存一次的工作区。这里拿着强引用：窗口一关，界面连同它拿着的工作区都释放了，
    /// 到应用的退出钩子（窗口都关完以后才调用）就存不了终端里最后几秒的输出了
    static EXIT: RefCell<Option<Rc<Workspace>>> = const { RefCell::new(None) };
}

/// 窗口要关了、应用要退出了：把工作区最后存一遍（当前目录、终端里的内容都是最后一刻的）。多次调用只有第一次起作用
pub fn save_on_exit() {
    if let Some(workspace) = EXIT.with(|slot| slot.borrow().clone()) {
        workspace.finish();
    }
}

/// 窗口的根视图
pub fn root() -> Element {
    crate::native::keep_window_still_unless_asked();
    crate::native::set_application_icon();
    let store = Store::system();
    let window = current_window();
    let workspace = Workspace::new(Rc::new(crate::app::spawn), window, store.clone());
    EXIT.with(|slot| *slot.borrow_mut() = Some(workspace.clone()));
    if let Some(window) = window {
        // 上次最大化着，这次也最大化（大小位置在开窗口时就定好了，见 `main`）
        let plan = crate::display::Memory::load(&store).plan(
            &crate::display::displays(),
            (WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32),
        );
        if plan.maximized {
            window.set_maximized(true);
        }
        crate::display::track(window, store);
    }
    workspace.restore();
    workspace.autosave();
    chrome::build(workspace)
}

#[cfg(test)]
mod tests;
