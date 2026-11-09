//! 远程目标：连到服务器（SSH）和容器（Docker），记住连过的主机和容器。

use super::*;

impl Workspace {
    /// 新开一个连到服务器的标签并切过去。主机名不能用（空的、以 `-` 开头、带空白……）时什么也不做，返回 `false`。
    /// 手动连过的主机记下来，下次启动还在侧栏的「远程」里
    pub fn new_tab_ssh(&self, host: &str) -> bool {
        self.new_tab_ssh_in(host, None)
    }

    /// 同上，并且一开始进远端的 `directory`（绝对路径）。目录不能用（相对路径、带控制字符）就进主目录
    pub fn new_tab_ssh_in(&self, host: &str, directory: Option<String>) -> bool {
        let Some(host) = crate::ssh::normalize(host) else {
            return false;
        };
        self.remember_host(&host);
        let id = self.open(None, Target::Ssh { host }, None, directory, None);
        self.activate(id);
        true
    }

    pub(super) fn remember_host(&self, host: &str) {
        if self.hosts.get_untracked().iter().any(|known| known == host) {
            return;
        }
        self.manual_hosts.borrow_mut().push(host.to_string());
        self.hosts.update(|hosts| hosts.push(host.to_string()));
    }

    /// 把 `~/.ssh/config` 里读到的主机并进侧栏的「远程」（和手动连过的合在一起，去重）
    pub fn add_config_hosts(&self, found: Vec<String>) {
        self.hosts.update(|hosts| {
            for host in found {
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
        });
    }

    /// 新开一个连到容器的标签并切过去。名字不能用（空的、以 `-` 开头、带空白……）时什么也不做，返回 `false`。
    /// 手动连过的容器记下来，下次启动还在侧栏的「容器」里
    pub fn new_tab_docker(&self, container: &str) -> bool {
        self.new_tab_docker_in(container, None)
    }

    /// 同上，并且一开始进容器里的 `directory`（绝对路径）。目录不能用（相对路径、带控制字符）就进主目录
    pub fn new_tab_docker_in(&self, container: &str, directory: Option<String>) -> bool {
        let Some(container) = crate::docker::normalize(container) else {
            return false;
        };
        self.remember_container(&container);
        let id = self.open(None, Target::Docker { container }, None, directory, None);
        self.activate(id);
        true
    }

    pub(super) fn remember_container(&self, container: &str) {
        if self
            .containers
            .get_untracked()
            .iter()
            .any(|known| known == container)
        {
            return;
        }
        self.manual_containers
            .borrow_mut()
            .push(container.to_string());
        self.containers
            .update(|containers| containers.push(container.to_string()));
    }

    /// 重新列一遍运行中的容器（`docker ps`），和手动连过的、开着的标签用的合在一起：
    /// 停了的容器只要还有标签开着就还列着，没有标签的就不列了。列不出来（docker 不在）就保持原样。
    /// 在后台线程跑，不占界面
    pub fn refresh_containers(self: &Rc<Self>) {
        if !self.form.remote.get_untracked() {
            return;
        }
        let this = Rc::downgrade(self);
        let lister = self.container_lister.borrow().clone();
        aureole::detached(|| {
            aureole::spawn_blocking(move || lister()).then(move |result| {
                let Some(workspace) = this.upgrade() else {
                    return;
                };
                let Ok(found) = result else {
                    return;
                };
                let mut names = workspace.manual_containers.borrow().clone();
                let open = workspace
                    .tabs
                    .get_untracked()
                    .iter()
                    .filter_map(|tab| tab.target.container().map(str::to_string))
                    .collect::<Vec<_>>();
                for name in found.into_iter().chain(open) {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                workspace.containers.set(names);
            })
        });
    }
}
