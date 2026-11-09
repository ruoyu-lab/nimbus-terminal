//! 命令历史：跑完的命令（要有命令标记）记下来，存在配置目录里，命令面板里搜。

use super::*;

pub(super) const HISTORY_FILE: &str = "history.json";

/// 命令历史最多留这么多条
pub(super) const HISTORY_LIMIT: usize = 2000;

impl Workspace {
    /// 记下跑过的一条命令（和上一条一样的不重复记；已有的挪到最新）
    pub(super) fn record_command(&self, command: &str) {
        let command = command.trim();
        // 多行的（粘贴的几行、here-doc）不进历史：写回提示符时换行没法原样还原，改成空格会变成另一条命令
        if command.is_empty() || command.contains(['\n', '\r']) {
            return;
        }
        {
            let mut history = self.history.borrow_mut();
            history.retain(|known| known != command);
            history.push(command.to_string());
            let excess = history.len().saturating_sub(HISTORY_LIMIT);
            history.drain(..excess);
        }
        self.store.save(HISTORY_FILE, &*self.history.borrow());
    }

    /// 命令历史里包含 `query` 的（不分大小写），最近的在前，最多 `limit` 条
    pub fn history_matches(&self, query: &str, limit: usize) -> Vec<String> {
        let needle = query.to_lowercase();
        self.history
            .borrow()
            .iter()
            .rev()
            .filter(|command| !command.contains(['\n', '\r']))
            .filter(|command| command.to_lowercase().contains(&needle))
            .take(limit)
            .cloned()
            .collect()
    }
}
