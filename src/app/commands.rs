//! 命令标记（OSC 133）带来的动作：在命令之间跳、复制上一条命令的输出。标记本身的记录在 `vt::marks`。

use super::*;

impl Session {
    /// 在命令之间跳：`older` 是上一条，否则是下一条。跳到的那条命令（提示符加它的输出）会被选中高亮，
    /// 看得见就不滚动，在历史里就翻回去让它出现在最上面；选中之后 ⌘C 直接复制它。
    /// 一直往下跳，过了最新的一条就回到最新的输出。没有命令标记、副屏里、没有可去的地方返回 `false`
    pub fn jump_prompt(&self, older: bool) -> bool {
        // 跑过的命令：（提示符的行编号，输出结束的行编号）
        let ran: Vec<(u64, u64)> = {
            let terminal = self.terminal.borrow();
            if terminal.on_alternate {
                return false;
            }
            terminal
                .commands()
                .iter()
                .filter(|command| {
                    command.output_start.is_some()
                        && terminal.absolute_line(command.prompt).is_some()
                })
                .map(|command| {
                    (
                        command.prompt,
                        command.output_end.unwrap_or(command.prompt + 1),
                    )
                })
                .collect()
        };
        let current = self.jump_mark.get();
        let target = match (older, current) {
            (true, None) => ran.last().copied(),
            (true, Some(mark)) => ran.iter().rev().find(|(prompt, _)| *prompt < mark).copied(),
            (false, None) => return false,
            (false, Some(mark)) => ran.iter().find(|(prompt, _)| *prompt > mark).copied(),
        };
        let Some((prompt, end)) = target else {
            if older {
                return false;
            }
            // 过了最新的一条：回到最新的输出，取消选中
            self.snap_to_bottom();
            self.grid.set_selection(None);
            return true;
        };
        self.jump_mark.set(Some(prompt));
        let (history, rows, line) = {
            let terminal = self.terminal.borrow();
            (
                terminal.scrollback_len(),
                terminal.size().1,
                terminal.absolute_line(prompt).unwrap_or(0),
            )
        };
        // 不在看得见的范围里才滚：在历史里就翻回去，让它出现在最上面
        let top = history.saturating_sub(self.offset.get());
        if line < top || line >= top + rows {
            self.offset.set(history.saturating_sub(line.min(history)));
        }
        self.flush();
        // 选中这条命令：从提示符那一行到输出的最后一行，超出看得见的部分截掉
        let top = history.saturating_sub(self.offset.get());
        let columns = self.grid.size()[0];
        let last = self
            .terminal
            .borrow()
            .absolute_line(end.saturating_sub(1).max(prompt))
            .unwrap_or(line);
        let first_row = line.saturating_sub(top).min(rows - 1);
        let last_row = last.saturating_sub(top).min(rows - 1);
        self.grid.set_selection(Some((
            CharGridPosition::new(first_row, 0),
            CharGridPosition::new(last_row, columns),
        )));
        true
    }

    /// 最近一条跑完、有输出的命令的输出
    pub fn last_command_output(&self) -> Option<String> {
        let terminal = self.terminal.borrow();
        (0..terminal.commands().len()).rev().find_map(|index| {
            terminal
                .command_output(index)
                .filter(|text| !text.is_empty())
        })
    }

    /// 把上一条命令的输出放进剪贴板
    pub fn copy_last_output(&self) -> bool {
        match self.last_command_output() {
            Some(text) => {
                aureole::clipboard::write_text(text);
                true
            }
            None => false,
        }
    }
}
