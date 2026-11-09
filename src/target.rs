//! 会话连到哪里，以及这个目标自己带哪些面板。
//!
//! 界面不认识具体的目标：它只问「这个会话的目标有没有面板」。本机一个面板也没有，所以只用本机的人
//! 永远看不到右侧面板；连到服务器的会话才有监控和文件，换回本机标签面板就自己消失。
//! 以后加别的目标（WSL、容器），也只是在这里多一种，再回答它有哪些面板。

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local,
    /// 通过系统的 `ssh` 连到服务器。`host` 是 `~/.ssh/config` 里的名字或者 `user@host`
    Ssh { host: String },
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
        }
    }

    /// 服务器的主机名；本机是 `None`
    pub fn host(&self) -> Option<&str> {
        match self {
            Self::Local => None,
            Self::Ssh { host } => Some(host),
        }
    }

    /// 这个目标有哪些面板，按显示的先后。本机没有
    pub fn panels(&self) -> Vec<PanelKind> {
        match self {
            Self::Local => Vec::new(),
            Self::Ssh { .. } => vec![PanelKind::Monitor, PanelKind::Files],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_machine_the_session_is_on() {
        assert_eq!(Target::Local.label(), "本机");
        assert_eq!(Target::Ssh { host: "prod-hk".into() }.label(), "prod-hk");
    }

    /// 本机没有面板：只用本机的人看不到任何远程的东西
    #[test]
    fn only_remote_targets_have_panels() {
        assert!(Target::Local.panels().is_empty());
        let remote = Target::Ssh { host: "prod-hk".into() };
        assert_eq!(remote.panels(), [PanelKind::Monitor, PanelKind::Files]);
    }
}
