//! 标签里的终端：起一个终端、拆分、聚焦、关掉其中一个。排布的树本身在 `pane`。

use super::*;

/// 起一个终端要说的：属于哪个标签、什么目标、从哪儿开始
pub(super) struct PaneSpec<'a> {
    pub(super) tab: u64,
    pub(super) folder: Option<u64>,
    pub(super) target: &'a Target,
    /// 复用连接的套接字（服务器的第一个终端才有）
    pub(super) control: Option<PathBuf>,
    pub(super) directory: Option<PathBuf>,
    pub(super) remote_directory: Option<String>,
    /// 上次存下的终端内容，先放上去
    pub(super) history: Option<String>,
    /// 服务器的会话不连，停在「已断开，按回车连接」（重启后恢复的）
    pub(super) waiting: bool,
}

/// 按落盘的排布恢复一个标签时，沿着排布的树往下传的东西
pub(super) struct Restoring<'a> {
    pub(super) tab: u64,
    pub(super) folder: Option<u64>,
    pub(super) target: &'a Target,
    pub(super) keep_output: bool,
    /// 复用套接字只给第一个终端
    pub(super) control: Option<PathBuf>,
    /// 终端内容的文件按全部终端的先后编号，到了第几个
    pub(super) flat: &'a mut usize,
}

impl Workspace {
    /// 按落盘的排布把终端一个个开出来
    pub(super) fn build_pane(&self, saved: &SavedPane, restoring: &mut Restoring) -> Pane {
        match saved {
            SavedPane::Pane { directory } => {
                let history = restoring
                    .keep_output
                    .then(|| self.store.load_text(&output_file(*restoring.flat)))
                    .flatten();
                *restoring.flat += 1;
                // 复用套接字只给第一个终端：它当主连接，拆出来的各连各的
                let (id, session) = self.spawn_pane(PaneSpec {
                    tab: restoring.tab,
                    folder: restoring.folder,
                    target: restoring.target,
                    control: restoring.control.take(),
                    directory: directory.clone(),
                    remote_directory: None,
                    history,
                    waiting: true,
                });
                Pane::leaf(id, session)
            }
            SavedPane::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.build_pane(first, restoring);
                let second = self.build_pane(second, restoring);
                Pane::split_node(
                    *axis,
                    ratio.clamp(crate::pane::RATIO_RANGE.0, crate::pane::RATIO_RANGE.1),
                    first,
                    second,
                )
            }
        }
    }

    /// 起一个终端（会话），它属于标签 `tab`。返回它在标签里的编号和会话
    ///
    /// `waiting` 为真时服务器的会话不连，停在「已断开，按回车连接」（重启后恢复的）
    pub(super) fn spawn_pane(&self, spec: PaneSpec) -> (u64, Rc<Session>) {
        let PaneSpec {
            tab,
            folder,
            target,
            control,
            directory,
            remote_directory,
            history,
            waiting,
        } = spec;
        let (columns, rows) = self.size.get_untracked();
        let shell = (self.form.shell.get_untracked() == ShellKind::Program)
            .then(|| self.form.shell_program.get_untracked().trim().to_string())
            .filter(|program| !program.is_empty());
        let owns_connection = target
            .host()
            .zip(control.clone())
            .map(|(host, control)| (host.to_string(), control));
        let launch = Launch {
            target: target.clone(),
            columns,
            rows,
            // 起始目录只对本机有意思
            directory: match target {
                Target::Local => self.start_directory(folder, directory),
                Target::Ssh { .. } | Target::Docker { .. } => None,
            },
            shell,
            dark_background: crate::theme::dark().get_untracked(),
            control,
            remote_directory,
            integration: self
                .form
                .shell_integration
                .get_untracked()
                .then(|| self.integration.clone())
                .flatten(),
        };
        let remote = target.remote_name().is_some();
        let restoring = remote && waiting;
        let spawned = if restoring {
            // 恢复的远端会话先不连
            Spawned {
                pty: None,
                queue: None,
                error: None,
            }
        } else {
            (self.spawn)(&launch)
        };
        let pane = self.next_pane.get();
        self.next_pane.set(pane + 1);
        let (for_alert, for_paste, for_record) =
            (self.this.clone(), self.this.clone(), self.this.clone());
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
                // 主连接在后台常驻，客户端结束了它还在：这个终端是主连接的主人（带着套接字）时，
                // 它结束就把连接和通道一起收掉（重连时会开一条新的）
                if let Some((host, control)) = &owns_connection {
                    crate::ssh::shutdown_in_background(host, control);
                }
                // 正常退出（`exit`、Ctrl-D）这个终端就关（拆分出来的只关它，最后一个关标签）；出错退出留着，屏幕上有原因
                if code == 0
                    && let Some(workspace) = for_exit.upgrade()
                {
                    workspace.close_pane(tab, pane);
                }
            }),
            confirm_paste: Some(Rc::new(move |text| {
                if let Some(workspace) = for_paste.upgrade() {
                    workspace
                        .pending_paste
                        .set(Some(PendingPaste { tab, pane, text }));
                }
            })),
            record: Some(Rc::new(move |command| {
                if let Some(workspace) = for_record.upgrade() {
                    workspace.record_command(&command);
                }
            })),
            alert: Rc::new(move |alert| {
                if let Some(workspace) = for_alert.upgrade() {
                    workspace.alert(tab, pane, alert);
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
        session.set_local(!remote);
        if remote {
            // 断线后按回车重连：用当时的大小再起一次 `ssh` / `docker exec`
            let (spawn, this) = (self.spawn.clone(), self.this.clone());
            session.set_reconnect(Rc::new(move || {
                let mut launch = launch.clone();
                if let Some(workspace) = this.upgrade() {
                    (launch.columns, launch.rows) = workspace.size.get_untracked();
                }
                spawn(&launch)
            }));
            if restoring {
                let name = target.remote_name().unwrap_or_default();
                session.wait_for_reconnect(&format!("[已断开 {name}。按回车连接]"));
            }
        }
        (pane, session)
    }

    /// 把排布放进新的标签：进标签条和侧栏的末尾
    pub(super) fn push_tab(
        &self,
        id: u64,
        folder: Option<u64>,
        target: Target,
        control: Option<PathBuf>,
        layout: Pane,
        focus: u64,
    ) {
        // 信号放在不挂界面的作用域里：标签是在点击、快捷键、命令面板的处理器里开的，
        // 记在它们的作用域上，一会儿就释放了
        let (layout, focus) = aureole::detached(|| (signal(layout), signal(focus)));
        self.tabs.update(|tabs| {
            tabs.push(Tab {
                id,
                layout,
                focus,
                target,
                control,
                folder,
            })
        });
        self.bar.update(|bar| bar.push(id));
        // 标题要在标签进了列表之后才算：服务器的会话默认叫主机名，得先找得到它的目标
        self.refresh_title(id);
    }

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
        // 拆出来的终端各连各的，不抢主连接
        let (pane, session) = self.spawn_pane(PaneSpec {
            tab: tab.id,
            folder: tab.folder,
            target: &tab.target,
            control: None,
            directory,
            remote_directory: None,
            history: None,
            waiting: false,
        });
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

    /// ⌘W：设置页开着时是退出设置，否则关当前标签
    /// 现在聚焦的那个终端（设置页开着时没有）
    pub fn focused_session(&self) -> Option<Rc<crate::app::Session>> {
        if self.settings_open.get_untracked() {
            return None;
        }
        let active = self.active.get_untracked();
        let tab = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|tab| tab.id == active)?;
        tab.layout
            .get_untracked()
            .session(tab.focus.get_untracked())
    }
}
