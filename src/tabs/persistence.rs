//! 工作区的保存和恢复：标签、文件夹、终端里的内容落盘，下次启动时再开出来。

use super::*;

/// 文件名
pub(super) const STATE_FILE: &str = "state.json";

/// 每个标签最多存多少行终端内容（文件名是 `output-第几个标签.ansi`）
pub(super) const OUTPUT_LINES: usize = 3000;

pub(super) fn output_file(index: usize) -> String {
    format!("output-{index}.ansi")
}

/// 落盘的工作区：文件夹、开着的标签（按标签条的顺序）、侧栏宽度
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Saved {
    pub(super) folders: Vec<SavedFolder>,
    pub(super) tabs: Vec<SavedTab>,
    /// 手动连过的服务器（`~/.ssh/config` 里的不用存，每次启动重新读）
    pub(super) hosts: Vec<String>,
    /// 手动连过的容器（运行中的不用存，每次启动重新列）
    pub(super) containers: Vec<String>,
    /// 当前标签是 `tabs` 里的第几个
    pub(super) active: Option<usize>,
    pub(super) sidebar_width: Option<f32>,
    pub(super) panel_width: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedFolder {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) path: Option<PathBuf>,
    pub(super) open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedTab {
    pub(super) folder: Option<u64>,
    /// 服务器的主机名；不是服务器的标签是 `None`
    pub(super) host: Option<String>,
    /// 容器的名字；不是容器的标签是 `None`
    pub(super) container: Option<String>,
    /// 用户给起的名字；没起过就是 `None`
    pub(super) title: Option<String>,
    /// 关掉时聚焦的那个终端的目录（没有 `layout` 的旧文件用它）
    pub(super) directory: Option<PathBuf>,
    /// 拆分的排布
    pub(super) layout: Option<SavedPane>,
    /// 聚焦的是排布里的第几个终端（从 0 数，从左到右、从上到下）
    pub(super) focus: Option<usize>,
    /// 本机标签指定的 shell 程序（新建菜单里选的）；用设置里的默认就是 `None`。旧文件里没有这一项
    pub(super) shell: Option<String>,
}

/// 落盘的排布：一个终端（带关掉时的目录），或者一次拆分
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum SavedPane {
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
    pub(super) fn pane_count(&self) -> usize {
        self.layout.as_ref().map_or(1, SavedPane::leaf_count)
    }
}

thread_local! {
    /// 退出应用时要最后存一次的工作区。这里拿着强引用：窗口一关，界面连同它拿着的工作区都释放了，
    /// 到应用的退出钩子（窗口都关完以后才调用）就存不了终端里最后几秒的输出了
    static EXIT: RefCell<Option<Rc<Workspace>>> = const { RefCell::new(None) };
}

/// 记住工作区，退出应用时最后存一次（`save_on_exit`）
pub(super) fn keep_for_exit(workspace: &Rc<Workspace>) {
    EXIT.with(|slot| *slot.borrow_mut() = Some(workspace.clone()));
}

/// 窗口要关了、应用要退出了：把工作区最后存一遍（当前目录、终端里的内容都是最后一刻的）。多次调用只有第一次起作用
pub fn save_on_exit() {
    if let Some(workspace) = EXIT.with(|slot| slot.borrow().clone()) {
        workspace.finish();
    }
}

impl Workspace {
    /// 工作区一变就存（稍等一下合并连续的改动）
    pub(super) fn watch_persistence(self: &Rc<Self>) {
        if !self.store.is_persistent() {
            return;
        }
        let this = self.this.clone();
        let (folders, tabs, bar, titles, active, width, panel_width, hosts) = (
            self.folders,
            self.tabs,
            self.bar,
            self.titles,
            self.active,
            self.sidebar_width,
            self.panel_width,
            self.hosts,
        );
        Effect::new(move || {
            let _ = (
                folders.get(),
                tabs.get(),
                bar.get(),
                titles.get(),
                active.get(),
                width.get(),
                panel_width.get(),
                hosts.get(),
            );
            if let Some(workspace) = this.upgrade() {
                workspace.save_soon();
            }
        });
    }

    /// 每隔 2 秒存一次：shell 的当前目录变了没有信号，只能定时去问
    pub(super) fn autosave(&self) {
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

    pub(super) fn save_soon(&self) {
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
    pub(super) fn snapshot(&self) -> Saved {
        let tabs = self.tabs.get_untracked();
        let customs = self.custom_titles.borrow();
        let bar = self.bar.get_untracked();
        let ordered: Vec<&Tab> = bar
            .iter()
            .filter_map(|id| tabs.iter().find(|tab| tab.id == *id))
            .collect();
        let active = self.active.get_untracked();
        Saved {
            hosts: self.manual_hosts.borrow().clone(),
            containers: self.manual_containers.borrow().clone(),
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
                        host: tab.target.host().map(str::to_string),
                        container: tab.target.container().map(str::to_string),
                        title: customs.get(&tab.id).cloned(),
                        directory: tab.session().working_directory(),
                        focus: layout.leaves().iter().position(|(id, _)| *id == focus),
                        layout: Some(SavedPane::of(&layout)),
                        shell: tab.shell.clone(),
                    }
                })
                .collect(),
            active: ordered.iter().position(|tab| tab.id == active),
            sidebar_width: Some(self.sidebar_width.get_untracked()),
            panel_width: Some(self.panel_width.get_untracked()),
        }
    }

    /// 要退出了：最后存一次，之后不再存
    pub fn finish(&self) {
        self.save_now();
        self.finished.set(true);
        // 后台的主连接要关掉（会话结束时没人替它们收尾），不然它们会在后台挂到兜底的超时
        for tab in self.tabs.get_untracked() {
            if let (Some(host), Some(control)) = (tab.target.host(), tab.control.as_deref()) {
                crate::ssh::shutdown(host, control);
            }
            if let Some(container) = tab.target.container() {
                crate::docker::shutdown(container);
            }
        }
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
    pub(super) fn save_outputs(&self) {
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
            let target = match tab.container.as_deref().and_then(crate::docker::normalize) {
                Some(container) => Target::Docker { container },
                None => tab
                    .host
                    .as_deref()
                    .and_then(crate::ssh::normalize)
                    .map_or(Target::Local, |host| Target::Ssh { host }),
            };
            if let Some(host) = target.host() {
                self.remember_host(host);
            }
            if let Some(container) = target.container() {
                self.remember_container(container);
            }
            let control = target.host().and_then(|_| crate::ssh::control_path(tab_id));
            // 指定的 shell 只对本机标签有意思
            let shell = tab.shell.clone().filter(|_| target == Target::Local);
            let pane = self.build_pane(
                &layout,
                &mut Restoring {
                    tab: tab_id,
                    folder,
                    target: &target,
                    shell: shell.clone(),
                    keep_output,
                    control: control.clone(),
                    flat: &mut flat,
                },
            );
            let focus = tab
                .focus
                .and_then(|index| pane.leaves().get(index).map(|(id, _)| *id))
                .unwrap_or_else(|| pane.first_id());
            self.push_tab(tab_id, folder, target, shell, control, pane, focus);
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
}
