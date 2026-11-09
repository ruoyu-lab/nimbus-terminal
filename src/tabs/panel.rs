//! 右侧面板：服务器的监控，和远端文件的浏览、上传、下载。

use super::*;

/// 失败的原因，写给人看
pub(super) fn failure_text(failure: monitor::Failure) -> String {
    match failure {
        monitor::Failure::NotConnected => "终端的连接断了".to_string(),
        monitor::Failure::Unsupported => "暂不支持".to_string(),
        monitor::Failure::Failed(reason) => reason,
    }
}

impl Workspace {
    /// 当前会话的目标有哪些面板：本机没有；设置里关了「远程功能」也没有。在副作用里调用会跟着变
    pub fn panels(&self) -> Vec<PanelKind> {
        if !self.form.remote.get() {
            return Vec::new();
        }
        let active = self.active.get();
        self.tabs
            .get()
            .into_iter()
            .find(|tab| tab.id == active)
            .map_or_else(Vec::new, |tab| tab.target.panels())
    }

    pub fn has_panels(&self) -> bool {
        !self.panels().is_empty()
    }

    /// 右侧面板现在显示着吗：当前会话的目标有面板、没被收起、不在设置页里
    pub fn panel_shown(&self) -> bool {
        !self.settings_open.get() && self.panel_open.get() && self.has_panels()
    }

    pub fn toggle_panel(&self) {
        self.panel_open.update(|open| *open = !*open);
    }

    /// 现在该监控谁：当前标签是远端（服务器、容器）就返回（标签、目标、SSH 的复用套接字）
    pub fn monitor_plan(&self) -> Option<(u64, Target, Option<PathBuf>)> {
        let active = self.active.get_untracked();
        let tab = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|tab| tab.id == active)?;
        tab.target.remote_name()?;
        Some((active, tab.target.clone(), tab.control.clone()))
    }

    /// 取一次监控数据，结果回来写进 `monitors`。上一次还没回来就不再发；
    /// 取数在后台线程做（`ssh` 要等网络），界面不会卡
    pub fn poll_monitor(self: &Rc<Self>) {
        if self.polling.get() || !self.form.remote.get_untracked() {
            return;
        }
        let Some((tab, target, control)) = self.monitor_plan() else {
            return;
        };
        self.polling.set(true);
        let this = Rc::downgrade(self);
        // 任务不能记在调用它的作用域上：这里是监控页的定时器调的，页面一卸掉（关了标签、收起面板、换页）
        // 作用域就释放，在途的任务跟着被取消，结果永远回不来，「取数中」的标记就一直是真，之后再也不取了。
        // 放在不挂界面的作用域里，页面来来去去它都会跑完
        let fetch = self.fetch.borrow().clone();
        aureole::detached(|| {
            aureole::spawn_blocking(move || fetch(&target, control.as_deref())).then(
                move |result| {
                    let Some(workspace) = this.upgrade() else {
                        return;
                    };
                    workspace.polling.set(false);
                    workspace.apply_reading(tab, result);
                },
            )
        });
    }

    /// 收下一次取数的结果（标签已经关了就丢掉）
    pub fn apply_reading(&self, tab: u64, result: Result<monitor::Reading, monitor::Failure>) {
        if !self.has_tab(tab) {
            return;
        }
        self.monitors
            .update(|monitors| monitors.entry(tab).or_default().apply(result));
    }

    pub(super) fn update_files<R>(
        &self,
        tab: u64,
        change: impl FnOnce(&mut FilesState) -> R,
    ) -> Option<R> {
        if !self.has_tab(tab) {
            return None;
        }
        let mut result = None;
        self.files
            .update(|files| result = Some(change(files.entry(tab).or_default())));
        result
    }

    /// 读一个目录（`None` 是主目录）。结果回来写进状态；任务不绑在界面上（理由同 `poll_monitor`）
    pub fn files_load(self: &Rc<Self>, dir: Option<String>) {
        if !self.form.remote.get_untracked() {
            return;
        }
        let Some((tab, target, control)) = self.monitor_plan() else {
            return;
        };
        // 去过的目录先显示缓存的（马上就有，不等网络），后台照常重新读，读回来再换
        let asked_home = dir.is_none();
        let cached = self
            .files
            .get_untracked()
            .get(&tab)
            .and_then(|state| state.cached(dir.as_deref()));
        let Some(request) = self.update_files(tab, |state| state.begin_load(cached)) else {
            return;
        };
        let (this, remote) = (Rc::downgrade(self), self.remote.borrow().clone());
        aureole::detached(|| {
            aureole::spawn_blocking(move || {
                remote.list(&target, control.as_deref(), dir.as_deref())
            })
            .then(move |result| {
                if let Some(workspace) = this.upgrade() {
                    workspace
                        .update_files(tab, |state| state.finish_load(request, asked_home, result));
                }
            })
        });
    }

    /// 文件页挂上、换了标签、连接恢复时调：还没读过、或者上次因为没连上而没读成，就读
    pub fn files_ensure_loaded(self: &Rc<Self>) {
        let Some((tab, ..)) = self.monitor_plan() else {
            return;
        };
        let (status, path) = self
            .files
            .get_untracked()
            .get(&tab)
            .map_or((files::Status::Idle, None), |state| {
                (state.status.clone(), state.path.clone())
            });
        if matches!(status, files::Status::Idle | files::Status::NotConnected) {
            self.files_load(path);
        }
    }

    pub(super) fn files_path(&self) -> Option<(u64, String)> {
        let (tab, ..) = self.monitor_plan()?;
        let path = self.files.get_untracked().get(&tab)?.path.clone()?;
        Some((tab, path))
    }

    /// 现在这个目录里一个条目的完整远端路径
    pub fn files_full_path(&self, name: &str) -> Option<String> {
        let (_, path) = self.files_path()?;
        files::is_safe_name(name).then(|| files::join(&path, name))
    }

    /// 在远端的某个目录里新开一个终端：`name` 是现在这个目录里的一个文件夹；`None` 是现在这个目录本身
    pub fn files_new_terminal(&self, name: Option<&str>) {
        let Some((_, target, _)) = self.monitor_plan() else {
            return;
        };
        let Some((_, here)) = self.files_path() else {
            return;
        };
        let directory = match name {
            Some(name) if files::is_safe_name(name) => files::join(&here, name),
            Some(_) => return,
            None => here,
        };
        match target {
            Target::Ssh { host } => {
                self.new_tab_ssh_in(&host, Some(directory));
            }
            Target::Docker { container } => {
                self.new_tab_docker_in(&container, Some(directory));
            }
            Target::Local => {}
        }
    }

    /// 进一个子目录
    pub fn files_open(self: &Rc<Self>, name: &str) {
        if let Some((_, path)) = self.files_path()
            && files::is_safe_name(name)
        {
            self.files_load(Some(files::join(&path, name)));
        }
    }

    /// 回上一级
    pub fn files_up(self: &Rc<Self>) {
        if let Some((_, path)) = self.files_path() {
            self.files_load(Some(files::parent(&path)));
        }
    }

    /// 路径栏里输入了一个路径：`~` 是主目录，`/` 开头是绝对路径，别的当作相对现在这个目录的路径。
    /// 空的什么也不做。能不能进去由远端的 `cd` 说了算（进不去会在页面上写原因）
    pub fn files_go(self: &Rc<Self>, input: &str) {
        let input = input.trim();
        if input.is_empty() {
            return;
        }
        let target = if input == "~" {
            None
        } else if input.starts_with('/') {
            Some(input.to_string())
        } else if let Some((_, current)) = self.files_path() {
            Some(files::join(&current, input))
        } else {
            return;
        };
        self.files_load(target);
    }

    /// 重新读现在这个目录
    pub fn files_refresh(self: &Rc<Self>) {
        let path = self.files_path().map(|(_, path)| path);
        self.files_load(path);
    }

    pub fn files_toggle_hidden(&self) {
        if let Some((tab, ..)) = self.monitor_plan() {
            self.update_files(tab, |state| state.show_hidden = !state.show_hidden);
        }
    }

    pub fn files_clear_transfers(&self) {
        if let Some((tab, ..)) = self.monitor_plan() {
            self.update_files(tab, FilesState::clear_finished);
        }
    }

    pub(super) fn next_transfer_id(&self) -> u64 {
        let id = self.next_transfer.get();
        self.next_transfer.set(id + 1);
        id
    }

    /// 下载现在这个目录里的一个条目到「下载」目录（本地已有同名的加序号，不覆盖）。文件夹整个下载
    pub fn files_download(self: &Rc<Self>, name: &str) {
        let Some((tab, target, control)) = self.monitor_plan() else {
            return;
        };
        let Some((_, path)) = self.files_path() else {
            return;
        };
        let kind = self.files.get_untracked().get(&tab).and_then(|state| {
            state
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.kind)
        });
        let id = self.next_transfer_id();
        let Some(kind) = kind.filter(|kind| *kind != files::Kind::Other) else {
            self.update_files(tab, |state| {
                state.fail_transfer(id, name, false, "这种文件不能下载")
            });
            return;
        };
        if !files::is_safe_name(name) {
            self.update_files(tab, |state| {
                state.fail_transfer(id, name, false, "这个文件名不能传（带控制字符）")
            });
            return;
        }
        self.update_files(tab, |state| state.start_transfer(id, name, false));
        let (this, remote, into) = (
            Rc::downgrade(self),
            self.remote.borrow().clone(),
            self.download_dir.borrow().clone(),
        );
        let remote_path = files::join(&path, name);
        aureole::detached(|| {
            aureole::spawn_blocking(move || {
                remote.download(
                    &target,
                    control.as_deref(),
                    &remote_path,
                    kind == files::Kind::Dir,
                    &into,
                )
            })
            .then(move |result| {
                let Some(workspace) = this.upgrade() else {
                    return;
                };
                let state = match result {
                    Ok(saved) => TransferState::Done(format!("已存到 {}", saved.display())),
                    Err(failure) => TransferState::Failed(failure_text(failure)),
                };
                workspace.update_files(tab, |files| files.finish_transfer(id, state));
            })
        });
    }

    /// 上传本地的文件或文件夹到现在这个远端目录。远端已有同名的不覆盖：那一项直接记一条失败，别的照传。
    /// 传完（有成功的）重新读一次目录
    pub fn files_upload(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let Some((tab, target, control)) = self.monitor_plan() else {
            return;
        };
        let Some((_, directory)) = self.files_path() else {
            return;
        };
        for local in paths {
            let name = local
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let id = self.next_transfer_id();
            let exists = self
                .files
                .get_untracked()
                .get(&tab)
                .is_some_and(|state| state.has_entry(&name));
            if exists {
                self.update_files(tab, |state| {
                    state.fail_transfer(id, &name, true, "远端已有同名文件，没有覆盖")
                });
                continue;
            }
            if !files::is_safe_name(&name) {
                self.update_files(tab, |state| {
                    state.fail_transfer(id, &name, true, "这个文件名不能传（带控制字符）")
                });
                continue;
            }
            self.update_files(tab, |state| state.start_transfer(id, &name, true));
            let (this, remote) = (Rc::downgrade(self), self.remote.borrow().clone());
            let (target, control, directory) = (target.clone(), control.clone(), directory.clone());
            aureole::detached(|| {
                aureole::spawn_blocking(move || {
                    remote.upload(&target, control.as_deref(), &local, &directory)
                })
                .then(move |result| {
                    let Some(workspace) = this.upgrade() else {
                        return;
                    };
                    let ok = result.is_ok();
                    let state = match result {
                        Ok(()) => TransferState::Done("已上传".to_string()),
                        Err(failure) => TransferState::Failed(failure_text(failure)),
                    };
                    workspace.update_files(tab, |files| files.finish_transfer(id, state));
                    if ok {
                        workspace.files_refresh();
                    }
                })
            });
        }
    }

    /// 弹出系统的选择文件对话框，选好的上传
    pub fn files_choose_upload(&self) {
        let this = self.this.clone();
        aureole::detached(|| {
            aureole::spawn_local(async move {
                let mut dialog = aureole::FileDialog::new().title("上传文件");
                if let Some(home) = crate::store::user_home() {
                    dialog = dialog.directory(home);
                }
                let paths = dialog.pick_files().await;
                if paths.is_empty() {
                    return;
                }
                if let Some(workspace) = this.upgrade() {
                    workspace.files_upload(paths);
                }
            })
        });
    }
}
