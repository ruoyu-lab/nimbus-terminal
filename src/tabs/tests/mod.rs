//! 工作区的测试：按功能分文件，这里放大家共用的辅助函数和假的会话、假的远端。

mod docker;
mod files_page;
mod folder;
mod input;
mod keyboard;
mod notification;
mod restart;
mod screenshots;
mod server;
mod settings;
mod shells;
mod side_panel;
mod split_panes;
mod tab_strip;

use std::sync::Arc;

use aureole::MouseButton;
use aureole::testing::Harness;

use super::*;
use crate::pane::{Axis, Pane};
use crate::pty::{Pty, Recorder};

/// 假的起会话：每次都记下是怎么起的，给一个只记录写入的假 pty（可以假装它在某个目录、前台有程序在跑）
#[derive(Clone, Default)]
pub(super) struct Fake {
    pub(super) launches: Rc<RefCell<Vec<Launch>>>,
    pub(super) ptys: Rc<RefCell<Vec<Arc<Recorder>>>>,
}

impl Fake {
    pub(super) fn spawn(&self) -> SpawnFn {
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

    pub(super) fn launch(&self, index: usize) -> Launch {
        self.launches.borrow()[index].clone()
    }

    pub(super) fn pty(&self, index: usize) -> Arc<Recorder> {
        self.ptys.borrow()[index].clone()
    }
}

pub(super) fn title_bar() -> aureole::TitleBarInfo {
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
pub(super) fn blank(store: Store) -> (Harness, Rc<Workspace>, Fake) {
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
pub(super) fn blank_with_traffic_lights(store: Store) -> (Harness, Rc<Workspace>, Fake) {
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
pub(super) fn mounted() -> (Harness, Rc<Workspace>) {
    mounted_with(title_bar())
}

/// 用给定的标题栏度量挂（macOS 的红绿灯，或者没有系统按钮的 Windows、Linux）
pub(super) fn mounted_with(info: aureole::TitleBarInfo) -> (Harness, Rc<Workspace>) {
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
pub(super) fn directory(label: &str) -> PathBuf {
    let directory = crate::store::tests::scratch(label);
    std::fs::create_dir_all(&directory).expect("建得了目录");
    directory
}

/// 侧栏的顺序（开的先后）
pub(super) fn ids(workspace: &Workspace) -> Vec<u64> {
    workspace
        .tabs
        .get_untracked()
        .iter()
        .map(|tab| tab.id)
        .collect()
}

/// 顶部标签条的顺序
pub(super) fn bar_ids(workspace: &Workspace) -> Vec<u64> {
    workspace.bar.get_untracked()
}

/// 侧栏里的会话行，从上到下的 id
pub(super) fn sidebar_order(harness: &Harness, workspace: &Workspace) -> Vec<u64> {
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
pub(super) fn bar_order(harness: &Harness, workspace: &Workspace) -> Vec<u64> {
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

pub(super) fn command(harness: &Harness, key: &str) {
    harness.key_down_with(Key::Character(key.into()), Modifiers::SUPER);
    harness.frame();
}

/// 右键点一下命名元素的中心
pub(super) fn right_click_named(harness: &Harness, name: &str) {
    let [x, y] = harness.window_bounds(name).expect("元素在").center();
    harness.right_click(x, y);
    harness.frame();
}

/// 本机标签没起名、程序也没报标题时叫什么：Windows 上是默认 shell 的名字（Windows PowerShell……），别的平台是「终端」
pub(super) fn local_title() -> String {
    crate::pty::shell_name(&crate::pty::default_shell())
        .unwrap_or(DEFAULT_TITLE)
        .to_string()
}

/// 点浮层里的菜单项（浮层里的元素用窗口坐标点）
pub(super) fn click_menu(harness: &Harness, item: &str) {
    let [x, y] = harness
        .window_bounds(item)
        .unwrap_or_else(|| panic!("菜单里没有 {item}"))
        .center();
    harness.click(x, y);
    harness.advance(secs(1.0));
}

pub(super) fn click(harness: &Harness, name: &str) {
    harness.click_named(name);
    harness.advance(secs(1.0));
}

pub(super) fn folder_names(workspace: &Workspace) -> Vec<String> {
    workspace
        .folders
        .get_untracked()
        .iter()
        .map(|folder| folder.name.clone())
        .collect()
}

/// 把一个标签的目标换成服务器（连接还没做，面板只看目标的声明）
pub(super) fn make_remote(workspace: &Workspace, id: u64) {
    workspace.tabs.update(|tabs| {
        if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id) {
            tab.target = crate::target::Target::Ssh {
                host: "prod-hk".into(),
            };
        }
    });
}

// ---- SSH 目标 ----

use crate::target::Target;

pub(super) fn ssh_host(workspace: &Workspace, id: u64) -> Option<String> {
    workspace
        .tabs
        .get_untracked()
        .iter()
        .find(|tab| tab.id == id)
        .and_then(|tab| tab.target.host().map(str::to_string))
}

// ---- Docker 目标 ----

pub(super) fn docker_container(workspace: &Workspace, id: u64) -> Option<String> {
    workspace
        .tabs
        .get_untracked()
        .iter()
        .find(|tab| tab.id == id)
        .and_then(|tab| tab.target.container().map(str::to_string))
}

/// 挂好的文件页：界面、工作区、假的远端、每次起会话的 `Launch`
pub(super) type MountedFiles = (
    Harness,
    Rc<Workspace>,
    Arc<FakeRemote>,
    Rc<RefCell<Vec<Launch>>>,
);

/// 同 `mounted_files_with_launches`，但标签连的是容器
pub(super) fn mounted_docker_files() -> MountedFiles {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    *workspace.container_lister.borrow_mut() = Arc::new(|| Ok(Vec::new()));
    let remote = Arc::new(FakeRemote::default());
    *workspace.remote.borrow_mut() = remote.clone();
    workspace.polling.set(true);
    *workspace.download_dir.borrow_mut() = PathBuf::from("/tmp/nimbus-downloads");
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    workspace.new_tab_docker("web-1");
    workspace.panel_tab.set(crate::target::PanelKind::Files);
    harness.frame();
    (harness, workspace, remote, fake.launches.clone())
}

// ---- 监控 ----

use crate::monitor::{CpuSample, CpuTimes, Failure, Reading, Status};

pub(super) fn reading(busy: u64, total: u64) -> Reading {
    Reading {
        cpu: CpuSample::Times(CpuTimes { busy, total }),
        mem_total: 16_384_000,
        mem_available: 8_192_000,
        disk: Some((102_400_000, 40_960_000)),
        load: Some([0.52, 0.61, 0.7]),
        uptime: Some(273_600),
    }
}

// ---- 文件 ----

use crate::files::{Entry, Kind, Listing, TransferState};
use crate::remote::Remote;
use std::sync::Mutex;

/// 假的远端：几个固定的目录，记下每次调用。可以让它假装没连上、不支持
#[derive(Default)]
pub(super) struct FakeRemote {
    pub(super) log: Mutex<Vec<String>>,
    /// 每次调用连的是哪个目标（服务器还是容器、叫什么）
    pub(super) targets: Mutex<Vec<Target>>,
    pub(super) failure: Mutex<Option<Failure>>,
    /// 设了的话，列目录要等它变真才返回（模拟慢的网络）
    pub(super) gate: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
    /// 设了的话，主目录里是这些（模拟服务器上的东西变了）
    pub(super) home_override: Mutex<Option<Vec<Entry>>>,
}

impl FakeRemote {
    pub(super) fn entry(name: &str, kind: Kind, size: u64) -> Entry {
        Entry {
            name: name.to_string(),
            kind,
            size,
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.log.lock().expect("锁").clone()
    }

    pub(super) fn fail_with(&self, failure: Option<Failure>) {
        *self.failure.lock().expect("锁") = failure;
    }
}

impl Remote for FakeRemote {
    fn list(
        &self,
        target: &Target,
        _: Option<&Path>,
        dir: Option<&str>,
    ) -> Result<Listing, Failure> {
        self.targets.lock().expect("锁").push(target.clone());
        self.log
            .lock()
            .expect("锁")
            .push(format!("list {}", dir.unwrap_or("~")));
        let gate = self.gate.lock().expect("锁").clone();
        if let Some(gate) = gate {
            while !gate.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        if let Some(failure) = self.failure.lock().expect("锁").clone() {
            return Err(failure);
        }
        let path = dir.unwrap_or("/home/deploy").to_string();
        let entries = match path.as_str() {
            "/home/deploy" if self.home_override.lock().expect("锁").is_some() => self
                .home_override
                .lock()
                .expect("锁")
                .clone()
                .unwrap_or_default(),
            "/home/deploy" => vec![
                Self::entry("src", Kind::Dir, 4096),
                Self::entry(".env", Kind::File, 20),
                Self::entry("readme.txt", Kind::File, 2048),
                Self::entry("pipe", Kind::Other, 0),
            ],
            "/home/deploy/src" => vec![Self::entry("main.rs", Kind::File, 900)],
            "/home" => vec![Self::entry("deploy", Kind::Dir, 4096)],
            _ => vec![],
        };
        Ok(Listing { path, entries })
    }

    fn download(
        &self,
        target: &Target,
        _: Option<&Path>,
        remote: &str,
        directory: bool,
        into: &Path,
    ) -> Result<PathBuf, Failure> {
        self.targets.lock().expect("锁").push(target.clone());
        self.log
            .lock()
            .expect("锁")
            .push(format!("download {remote} dir={directory}"));
        if remote.ends_with("boom") {
            return Err(Failure::Failed("没权限".into()));
        }
        Ok(into.join(remote.rsplit('/').next().unwrap_or("x")))
    }

    fn upload(
        &self,
        target: &Target,
        _: Option<&Path>,
        local: &Path,
        remote_dir: &str,
    ) -> Result<(), Failure> {
        self.targets.lock().expect("锁").push(target.clone());
        self.log
            .lock()
            .expect("锁")
            .push(format!("upload {} -> {remote_dir}", local.display()));
        Ok(())
    }
}

/// 挂一个服务器标签、文件页，并把远端换成假的。返回的 `harness` 已经开着文件页
pub(super) fn mounted_files() -> (Harness, Rc<Workspace>, Arc<FakeRemote>) {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    let remote = Arc::new(FakeRemote::default());
    *workspace.remote.borrow_mut() = remote.clone();
    workspace.polling.set(true); // 监控页不在这里测
    *workspace.download_dir.borrow_mut() = PathBuf::from("/tmp/nimbus-downloads");
    workspace.new_tab_ssh("prod-hk");
    workspace.panel_tab.set(crate::target::PanelKind::Files);
    harness.frame();
    (harness, workspace, remote)
}

/// 后台的活要真的跑完：边推时间边等，直到条件成立（最多 3 秒）
pub(super) fn settle(harness: &Harness, done: impl Fn() -> bool) {
    for _ in 0..150 {
        harness.advance(ms(30));
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn files_of(workspace: &Workspace) -> crate::files::FilesState {
    workspace
        .files
        .get_untracked()
        .get(&1)
        .cloned()
        .unwrap_or_default()
}

// ---- 文件页：右键新建终端、拖进终端 ----

/// 同 `mounted_files`，并且记下每次起会话的 `Launch`（看新终端进了哪个远端目录）
pub(super) fn mounted_files_with_launches() -> MountedFiles {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    let remote = Arc::new(FakeRemote::default());
    *workspace.remote.borrow_mut() = remote.clone();
    workspace.polling.set(true);
    *workspace.download_dir.borrow_mut() = PathBuf::from("/tmp/nimbus-downloads");
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    workspace.new_tab_ssh("prod-hk");
    workspace.panel_tab.set(crate::target::PanelKind::Files);
    harness.frame();
    FAKE_PTYS.with(|slot| *slot.borrow_mut() = Some(fake.ptys.clone()));
    (harness, workspace, remote, fake.launches.clone())
}

/// 起过的会话的假 pty，按起的先后
type FakePtys = Rc<RefCell<Vec<Arc<Recorder>>>>;

thread_local! {
    static FAKE_PTYS: RefCell<Option<FakePtys>> = const { RefCell::new(None) };
}

/// 第一个会话的假 pty（看写进了什么）
pub(super) fn fake_pty(_workspace: &Workspace) -> Arc<Recorder> {
    FAKE_PTYS.with(|slot| slot.borrow().as_ref().expect("先挂好").borrow()[0].clone())
}
