//! 选区和剪贴板：选中的字、复制、全选。

use super::*;

impl Session {
    /// 选中的字（没有选中是 `None`）
    pub fn selected_text(&self) -> Option<String> {
        self.grid.selected_text().filter(|text| !text.is_empty())
    }

    /// 把选中的字放进剪贴板；没有选中什么也不做
    pub fn copy_selection(&self) {
        if let Some(text) = self.selected_text() {
            aureole::clipboard::write_text(text);
        }
    }

    /// 选中屏幕上的全部内容
    pub fn select_all(&self) {
        let [columns, rows] = self.grid.size();
        self.grid.set_selection(Some((
            CharGridPosition::new(0, 0),
            CharGridPosition::new(rows.saturating_sub(1), columns),
        )));
    }
}
