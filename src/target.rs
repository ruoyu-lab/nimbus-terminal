//! 会话连到哪里。现在只有本机。

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local,
}

impl Target {
    /// 状态条和提示里写的名字
    pub fn label(&self) -> String {
        match self {
            Self::Local => "本机".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_machine_the_session_is_on() {
        assert_eq!(Target::Local.label(), "本机");
    }
}
