//! 会话连到哪里，以及这个目标自己带哪些面板。
//!
//! 界面不认识具体的目标：它只问「这个会话的目标有没有面板」。本机一个面板也没有，所以只用本机的人
//! 永远看不到右侧面板；连到服务器、容器的会话才有监控和文件，换回本机标签面板就自己消失。
//! 以后加别的目标（WSL），也只是在这里多一种，再回答它有哪些面板。

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local,
    /// 通过系统的 `ssh` 连到服务器。`host` 是 `~/.ssh/config` 里的名字或者 `user@host`
    Ssh {
        host: String,
    },
    /// 通过系统的 `docker exec` 进一个运行中的容器。`container` 是容器名或者 ID
    Docker {
        container: String,
    },
}

/// 右侧面板里的一页
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelKind {
    /// CPU、内存、磁盘
    Monitor,
    /// 远端的文件（SFTP）
    Files,
}

impl PanelKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Monitor => "监控",
            Self::Files => "文件",
        }
    }
}

impl Target {
    /// 状态条和提示里写的名字
    pub fn label(&self) -> String {
        match self {
            Self::Local => "本机".to_string(),
            Self::Ssh { host } => host.clone(),
            Self::Docker { container } => container.clone(),
        }
    }

    /// 服务器的主机名；本机、容器是 `None`
    pub fn host(&self) -> Option<&str> {
        match self {
            Self::Ssh { host } => Some(host),
            Self::Local | Self::Docker { .. } => None,
        }
    }

    /// 容器的名字；本机、服务器是 `None`
    pub fn container(&self) -> Option<&str> {
        match self {
            Self::Docker { container } => Some(container),
            Self::Local | Self::Ssh { .. } => None,
        }
    }

    /// 远端目标的名字（服务器的主机名、容器的名字）：标签默认叫它，断线提示里也写它。本机是 `None`
    pub fn remote_name(&self) -> Option<&str> {
        match self {
            Self::Ssh { host } => Some(host),
            Self::Docker { container } => Some(container),
            Self::Local => None,
        }
    }

    /// 这个目标有哪些面板，按显示的先后。本机没有
    pub fn panels(&self) -> Vec<PanelKind> {
        match self {
            Self::Local => Vec::new(),
            Self::Ssh { .. } | Self::Docker { .. } => vec![PanelKind::Monitor, PanelKind::Files],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_machine_the_session_is_on() {
        assert_eq!(Target::Local.label(), "本机");
        assert_eq!(
            Target::Ssh {
                host: "prod-hk".into()
            }
            .label(),
            "prod-hk"
        );
        assert_eq!(
            Target::Docker {
                container: "web-1".into()
            }
            .label(),
            "web-1"
        );
    }

    /// 本机没有面板：只用本机的人看不到任何远程的东西
    #[test]
    fn only_remote_targets_have_panels() {
        assert!(Target::Local.panels().is_empty());
        let remote = Target::Ssh {
            host: "prod-hk".into(),
        };
        assert_eq!(remote.panels(), [PanelKind::Monitor, PanelKind::Files]);
        let container = Target::Docker {
            container: "web-1".into(),
        };
        assert_eq!(container.panels(), [PanelKind::Monitor, PanelKind::Files]);
    }

    #[test]
    fn remote_names_distinguish_ssh_hosts_from_containers() {
        let ssh = Target::Ssh {
            host: "prod-hk".into(),
        };
        let docker = Target::Docker {
            container: "web-1".into(),
        };
        assert_eq!(ssh.host(), Some("prod-hk"));
        assert_eq!(ssh.container(), None);
        assert_eq!(docker.host(), None);
        assert_eq!(docker.container(), Some("web-1"));
        assert_eq!(ssh.remote_name(), Some("prod-hk"));
        assert_eq!(docker.remote_name(), Some("web-1"));
        assert_eq!(Target::Local.remote_name(), None);
    }
}
