//! 只在测试里用的会话方法：读内部状态、喂输出。

use super::*;

impl Session {
    pub(crate) fn flashes_for_test(&self) -> u64 {
        self.flashes.get_untracked()
    }

    pub(crate) fn hooks_for_test(&self) -> Hooks {
        self.hooks.clone()
    }

    pub(crate) fn exited_for_test(&self, code: i32) {
        self.exited(code);
    }

    /// 屏幕上现在的文字，测试用来看 shell 有没有回话
    pub(crate) fn screen_text_for_test(&self) -> String {
        let terminal = self.terminal.borrow();
        let screen = terminal.screen();
        screen
            .lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|cell| if cell.width == 0 { '\0' } else { cell.ch })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 网格上被高亮（底色是查找的颜色）的格子：（行，列，是不是当前那一处）
    pub(crate) fn highlighted_for_test(&self) -> Vec<(usize, usize, bool)> {
        let colors = view::colors(crate::theme::is_dark());
        let [columns, rows] = self.grid.size();
        let mut found = Vec::new();
        for row in 0..rows {
            for column in 0..columns {
                let Some(background) = self
                    .grid
                    .cell(row, column)
                    .and_then(|cell| cell.style.background)
                else {
                    continue;
                };
                if background == colors.found_current {
                    found.push((row, column, true));
                } else if background == colors.found {
                    found.push((row, column, false));
                }
            }
        }
        found
    }

    pub(crate) fn grid_size_for_test(&self) -> [usize; 2] {
        self.grid.size()
    }

    pub(crate) fn offset_for_test(&self) -> usize {
        self.offset.get()
    }

    /// 当作 shell 输出了这段文字
    pub(crate) fn feed_for_test(&self, text: &str) {
        self.terminal.borrow_mut().feed(text.as_bytes());
        self.output_version.set(self.output_version.get() + 1);
        self.flush();
    }
}
