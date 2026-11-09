//! 往历史里翻：滚动、Shift+PageUp 这类键、回到最新的输出。

use super::*;

impl Session {
    /// 往历史里翻：`lines` 为正是往更早的方向，为负是往最新的方向。到头了就停
    pub(super) fn scroll_back(&self, lines: isize) {
        let history = self.terminal.borrow().scrollback_len() as isize;
        let offset = self.offset.get();
        let target = (offset as isize).saturating_add(lines).clamp(0, history) as usize;
        if target != offset {
            self.offset.set(target);
            // 选区记的是网格上的位置，内容换了它就指错地方了
            self.grid.set_selection(None);
            self.flush();
        }
    }

    /// 回到最新的输出
    pub(super) fn snap_to_bottom(&self) {
        self.jump_mark.set(None);
        self.scroll_back(isize::MIN / 2);
    }

    /// Shift + PageUp / PageDown / Home / End：翻历史（和 xterm、iTerm、GNOME 终端一样）
    pub(super) fn scroll_key(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if !modifiers.has_shift()
            || modifiers.has_control()
            || modifiers.has_alt()
            || modifiers.has_super()
        {
            return false;
        }
        let page = self.terminal.borrow().size().1.saturating_sub(1).max(1) as isize;
        match &event.key {
            Key::Named(NamedKey::PageUp) => self.scroll_back(page),
            Key::Named(NamedKey::PageDown) => self.scroll_back(-page),
            Key::Named(NamedKey::Home) => self.scroll_back(isize::MAX / 2),
            Key::Named(NamedKey::End) => self.snap_to_bottom(),
            _ => return false,
        }
        true
    }
}
