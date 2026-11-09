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
use std::sync::Arc;

use aureole::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::{Hooks, Launch, PADDING, Session, Spawned};
use crate::files::{self, FilesState, TransferState};
use crate::monitor::{self, Monitor};
use crate::pane::{Axis, Pane};
use crate::settings::{
    Form, LastTab, Settings, ShellKind, SidebarStart, Startup, ThemeChoice, TitleSource,
};
use crate::store::Store;
use crate::target::{PanelKind, Target};
use crate::{chrome, view};

mod alerts;
mod folders;
mod history;
mod panel;
mod panes;
mod persistence;
mod remote;
mod shortcuts;

use alerts::PendingPaste;
use history::HISTORY_FILE;
use panes::{PaneSpec, Restoring};
pub use persistence::save_on_exit;
use persistence::{STATE_FILE, Saved, SavedPane, SavedTab, keep_for_exit, output_file};

/// 打开窗口时的内容区大小
pub const WINDOW_SIZE: (u32, u32) = (1040, 680);
/// macOS 红绿灯的位置，要让它在标题栏里垂直居中
pub const CONTROLS_POSITION: (f32, f32) = (16.0, 13.0);
/// 终端的默认名字
pub const DEFAULT_TITLE: &str = "终端";

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

/// 取一次监控数据：目标、复用套接字（SSH 才有） → 读数。在后台线程里调
type Fetch =
    Arc<dyn Fn(&Target, Option<&Path>) -> Result<monitor::Reading, monitor::Failure> + Send + Sync>;

/// 列出运行中的容器。在后台线程里调；测试里换成假的
type ContainerLister = Arc<dyn Fn() -> Result<Vec<String>, monitor::Failure> + Send + Sync>;

/// 一个标签
#[derive(Clone)]
pub struct Tab {
    pub id: u64,
    /// 这个标签里终端的排布：一个，或者拆分出来的好几个
    pub layout: Signal<Pane>,
    /// 聚焦的是哪个终端（排布里叶子的编号）：键盘打字、状态条、存目录都看它
    pub focus: Signal<u64>,
    pub target: Target,
    /// SSH 标签的连接（`ssh::control_path`）：macOS、Linux 上是复用套接字，第一个终端的 `ssh` 是主连接，监控借它；
    /// Windows 上是面板另开的那条连接登记用的名字。别的标签没有
    pub control: Option<PathBuf>,
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

/// 拖着的文件夹：拖的过程中顺序不变，别的文件夹只靠平移让位，松手才真的换顺序（和标签一样）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FolderDrag {
    pub id: u64,
    pub from: usize,
    pub to: usize,
}

/// 从文件页拖一个条目时带的数据：它在服务器上的绝对路径。放进终端就是把路径（按 shell 的规矩引好）写在光标处
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePath(pub String);

/// 拖文件夹时带的数据。单独一个类型：窗口里接标签的放置目标收的是 `u64`，不能把文件夹当成标签放下
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderId(pub u64);

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

/// 一个窗口里的全部状态
pub struct Workspace {
    /// 全部会话，按开的先后排；侧栏照这个列，不会被挪动
    pub tabs: Signal<Vec<Tab>>,
    /// 「目标」下面的文件夹，按创建的先后
    pub folders: Signal<Vec<Folder>>,
    /// 侧栏「远程」里列的服务器：`~/.ssh/config` 里的，加上手动连过的
    pub hosts: Signal<Vec<String>>,
    /// 手动连过的那部分（要存下来）
    manual_hosts: RefCell<Vec<String>>,
    /// 侧栏「容器」里列的：运行中的，加上手动连过的
    pub containers: Signal<Vec<String>>,
    /// 手动连过的那部分（要存下来）
    manual_containers: RefCell<Vec<String>>,
    /// 「连接到服务器」的输入框开着没有
    pub connect_prompt: Signal<bool>,
    /// 「连接到容器」的输入框开着没有
    pub docker_prompt: Signal<bool>,
    /// 每个服务器标签的监控数据（只有看着监控页的标签才会更新）
    pub monitors: Signal<HashMap<u64, Monitor>>,
    /// 每个服务器标签的文件页状态
    pub files: Signal<HashMap<u64, FilesState>>,
    /// 下载存到哪里（默认是「下载」目录）
    pub download_dir: RefCell<PathBuf>,
    /// 远端的列目录、下载、上传：平时按目标走 SSH 或 Docker，测试里换成假的
    remote: RefCell<Arc<dyn crate::remote::Remote>>,
    next_transfer: Cell<u64>,
    /// 有一次取数还没回来：不叠着发
    polling: Cell<bool>,
    /// 怎么取数（后台线程里调）：平时按目标走 SSH 或 Docker，测试里换成可控的
    fetch: RefCell<Fetch>,
    /// 怎么列容器（后台线程里调）：平时是 `docker ps`，测试里换成假的
    container_lister: RefCell<ContainerLister>,
    /// 文件夹命名框：新建，或者在改哪一个
    pub folder_prompt: Signal<Option<FolderPrompt>>,
    /// 正在拖着的文件夹；没拖时是 `None`
    pub folder_drag: Signal<Option<FolderDrag>>,
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
    /// 右侧面板：用户有没有把它收起来、多宽、停在哪一页。有没有面板可显示由当前会话的目标决定
    pub panel_open: Signal<bool>,
    pub panel_width: Signal<f32>,
    pub panel_tab: Signal<PanelKind>,
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
    /// 要不要把这段多行文字粘贴进去（程序没开括号粘贴时先问）：哪个标签、哪个终端、什么文字
    pub pending_paste: Signal<Option<PendingPaste>>,
    /// 跑过的命令（有命令标记才有），从老到新，去重，存在配置目录里
    history: RefCell<Vec<String>>,
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
    /// shell 集成脚本放在哪（写不进配置目录时是 `None`）
    integration: Option<PathBuf>,
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
        let panel_width = saved
            .panel_width
            .unwrap_or(chrome::PANEL_DEFAULT)
            .clamp(chrome::PANEL_MIN, chrome::PANEL_MAX);
        let workspace = Rc::new_cyclic(|this| Self {
            tabs: signal(Vec::new()),
            folders: signal(folders),
            hosts: signal(saved.hosts.clone()),
            manual_hosts: RefCell::new(saved.hosts.clone()),
            containers: signal(saved.containers.clone()),
            manual_containers: RefCell::new(saved.containers.clone()),
            connect_prompt: signal(false),
            docker_prompt: signal(false),
            monitors: signal(HashMap::new()),
            files: signal(HashMap::new()),
            download_dir: RefCell::new(
                crate::store::user_home()
                    .map_or_else(|| PathBuf::from("."), |home| home.join("Downloads")),
            ),
            remote: RefCell::new(Arc::new(crate::remote::OverRemote)),
            next_transfer: Cell::new(1),
            polling: Cell::new(false),
            fetch: RefCell::new(Arc::new(crate::remote::poll)),
            container_lister: RefCell::new(Arc::new(crate::docker::list_containers)),
            folder_prompt: signal(None),
            folder_drag: signal(None),
            bar: signal(Vec::new()),
            tab_drag: signal(None),
            titles: signal(HashMap::new()),
            active: signal(0),
            unseen: signal(HashSet::new()),
            size: signal(size),
            collapsed: signal(settings.sidebar == SidebarStart::Closed),
            sidebar_width: signal(sidebar_width),
            panel_open: signal(true),
            panel_width: signal(panel_width),
            panel_tab: signal(PanelKind::Monitor),
            palette_open: signal(false),
            settings_open: signal(false),
            settings_page: signal(SettingsPage::General),
            form: Form::new(&settings),
            renaming: signal(None),
            closing: signal(None),
            pending_paste: signal(None),
            history: RefCell::new(store.load(HISTORY_FILE)),
            closing_pane: Cell::new(None),
            custom_titles: RefCell::new(HashMap::new()),
            reported_titles: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            next_pane: Cell::new(1),
            next_folder: Cell::new(next_folder),
            spawn,
            window,
            integration: crate::shell::install(&store),
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
        // 「远程功能」开着就列一遍容器（启动时这里也会跑一次）；关了侧栏的「容器」整块不存在。
        // 等界面跑起来再取：取容器名单的函数这时才能被换掉（测试里换成假的）
        let this = self.this.clone();
        Effect::new(move || {
            if form.remote.get()
                && let Some(workspace) = this.upgrade()
            {
                aureole::detached(|| {
                    aureole::spawn_local(async move {
                        workspace.refresh_containers();
                    })
                });
            }
        });
        form.persist(self.store.clone());
    }

    // ---- 落盘 ----

    // ---- 新建、文件夹 ----

    /// 新开一个本机标签并切过去。当前会话在某个文件夹里时，新会话也归到那里
    pub fn new_tab(&self) {
        let folder = self
            .tabs
            .get_untracked()
            .iter()
            .find(|tab| tab.id == self.active.get_untracked())
            .and_then(|tab| tab.folder);
        let id = self.open(folder, Target::Local, None, None, None);
        self.activate(id);
    }

    /// 新开一个不归任何文件夹的终端（直接列在「目标」下），用设置里的起始目录
    pub fn new_tab_loose(&self) {
        let id = self.open(None, Target::Local, None, None, None);
        self.activate(id);
    }

    /// 在这个文件夹里新开一个本机终端，并把它展开。文件夹有目录就从那个目录起
    pub fn new_tab_in(&self, folder: u64) {
        self.reveal_folder(folder);
        let id = self.open(Some(folder), Target::Local, None, None, None);
        self.activate(id);
    }

    // ---- 起会话 ----

    fn new_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    /// 开一个标签（里面一个终端），放进标签条和侧栏，但不切过去。返回它的 id
    fn open(
        &self,
        folder: Option<u64>,
        target: Target,
        directory: Option<PathBuf>,
        remote_directory: Option<String>,
        history: Option<String>,
    ) -> u64 {
        let id = self.new_id();
        let control = target.host().and_then(|_| crate::ssh::control_path(id));
        let (pane, session) = self.spawn_pane(PaneSpec {
            tab: id,
            folder,
            target: &target,
            control: control.clone(),
            directory,
            remote_directory,
            history,
            waiting: false,
        });
        self.push_tab(id, folder, target, control, Pane::leaf(pane, session), pane);
        id
    }

    // ---- 拆分 ----

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
                // 远端的会话默认叫目标的名字（主机名、容器名），一眼分得清是哪个
                self.tabs
                    .get_untracked()
                    .iter()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| tab.target.remote_name().map(str::to_string))
                    .unwrap_or_else(|| DEFAULT_TITLE.to_string())
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
        if let (Some(host), Some(control)) = (closed.target.host(), closed.control.as_deref()) {
            crate::ssh::shutdown_in_background(host, control);
        }
        // 容器的通道是按容器共用的：还有别的标签用这个容器就留着
        if let Some(container) = closed.target.container()
            && !tabs
                .iter()
                .any(|tab| tab.target.container() == Some(container))
        {
            crate::docker::shutdown(container);
        }
        self.unseen.update(|unseen| {
            unseen.remove(&id);
        });
        self.monitors.update(|monitors| {
            monitors.remove(&id);
        });
        self.files.update(|files| {
            files.remove(&id);
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

    // ---- 文件 ----

    pub fn toggle_sidebar(&self) {
        self.collapsed.update(|collapsed| *collapsed = !*collapsed);
    }
}

/// 窗口的根视图
pub fn root() -> Element {
    crate::native::keep_window_still_unless_asked();
    crate::native::set_application_icon();
    let store = Store::system();
    let window = current_window();
    let workspace = Workspace::new(Rc::new(crate::app::spawn), window, store.clone());
    keep_for_exit(&workspace);
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
    // 侧栏「远程」里列的服务器：~/.ssh/config 里的
    workspace.add_config_hosts(
        crate::ssh::config_path().map_or_else(Vec::new, |path| crate::ssh::load_hosts(&path)),
    );
    workspace.restore();
    workspace.autosave();
    chrome::build(workspace)
}

#[cfg(test)]
mod tests;
