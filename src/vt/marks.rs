//! 命令标记（OSC 133，shell 集成）：shell 在提示符开始、输入开始、命令开始执行、命令结束时各发一个标记，
//! 终端据此知道每条命令在屏幕和历史里的位置。
//!
//! 位置用「行的编号」记：往历史里每挤进一行，编号加一，所以一行字不管滚到哪儿，编号都不变（`已挤出的行数 + 屏幕行`）。

use std::collections::HashMap;
use std::time::Instant;

use super::Terminal;
use super::screen::Cell;

/// 一条命令
#[derive(Clone, Debug)]
pub struct Command {
    /// 提示符所在的行（A）
    pub prompt: u64,
    /// 输入开始的位置：行、列（B）
    pub input: Option<(u64, usize)>,
    /// 输出的第一行（C）
    pub output_start: Option<u64>,
    /// 输出的结尾（D），不含这一行
    pub output_end: Option<u64>,
    pub exit: Option<i32>,
    /// 敲进去的命令（执行时从屏幕上读下来的）
    pub text: String,
    started: Option<Instant>,
}

/// 一条命令结束了
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub exit: Option<i32>,
    pub seconds: u64,
    pub text: String,
}

/// 存下来的内容里，某一行要写的命令标记
#[derive(Default)]
pub(super) struct RowMarks {
    /// 这一行开头的标记（C、D、A）
    pub before: String,
    /// 输入开始（B）在这一行的第几列
    pub input: Option<usize>,
}

/// 最多记这么多条命令，再多挤掉最旧的
const MAX_COMMANDS: usize = 2000;
/// 命令文字最长这么多字符
const MAX_TEXT: usize = 1000;

impl Terminal {
    /// OSC 133 的处理。副屏（vim、less）里的不理：那里没有提示符
    pub(super) fn semantic_prompt(&mut self, text: &str) {
        if self.on_alternate {
            return;
        }
        let mut parts = text.split(';');
        let kind = parts.next().unwrap_or("");
        let here = self.cursor_line_id();
        let column = self.screen().cursor.column;
        match kind {
            "A" => {
                self.integrated = true;
                // 上一个提示符没跑过命令（直接按了回车、Ctrl-C）：不留
                if self.commands.last().is_some_and(|last| last.output_start.is_none()) {
                    self.commands.pop();
                }
                self.commands.push(Command {
                    prompt: here,
                    input: None,
                    output_start: None,
                    output_end: None,
                    exit: None,
                    text: String::new(),
                    started: None,
                });
                if self.commands.len() > MAX_COMMANDS {
                    self.commands.remove(0);
                }
            }
            "B" => {
                if let Some(last) = self.commands.last_mut()
                    && last.output_start.is_none()
                {
                    last.input = Some((here, column));
                }
            }
            "C" => {
                let Some(input) = self
                    .commands
                    .last()
                    .filter(|last| last.output_start.is_none())
                    .and_then(|last| last.input)
                else {
                    return;
                };
                // 输入的最后一行：光标在行首说明回车已经把光标送到下一行了
                let last_line = if column == 0 { here.saturating_sub(1) } else { here };
                let typed = self.text_between(input.0, last_line.max(input.0), input.1);
                if let Some(last) = self.commands.last_mut() {
                    last.output_start = Some(here);
                    last.text = typed;
                    last.started = Some(Instant::now());
                }
            }
            "D" => {
                let exit = parts.next().and_then(|code| code.parse::<i32>().ok());
                let end = if column == 0 { here } else { here + 1 };
                let Some(last) = self.commands.last_mut() else {
                    return;
                };
                if last.output_start.is_none() || last.output_end.is_some() {
                    return;
                }
                last.output_end = Some(end);
                last.exit = exit;
                let seconds = last.started.map_or(0, |started| started.elapsed().as_secs());
                let finished = Finished {
                    exit,
                    seconds,
                    text: last.text.clone(),
                };
                if self.finished.len() < 8 {
                    self.finished.push(finished);
                }
            }
            _ => {}
        }
    }

    /// 光标所在行的编号
    fn cursor_line_id(&self) -> u64 {
        self.primary.archived_total() + self.primary.cursor.row as u64
    }

    /// 编号对应的行（主屏）；已经被挤出历史的是 `None`
    pub fn line_by_id(&self, id: u64) -> Option<&[Cell]> {
        let screen = &self.primary;
        let history = screen.history_len();
        let dropped = screen.archived_total().checked_sub(history as u64)?;
        let absolute = usize::try_from(id.checked_sub(dropped)?).ok()?;
        if absolute < history {
            screen.history_line(history - 1 - absolute).map(Vec::as_slice)
        } else {
            screen.lines.get(absolute - history).map(Vec::as_slice)
        }
    }

    /// 编号对应的「绝对行」（0 是最老的历史行，和查找、回看用的一样）
    pub fn absolute_line(&self, id: u64) -> Option<usize> {
        let screen = &self.primary;
        let dropped = screen.archived_total().checked_sub(screen.history_len() as u64)?;
        usize::try_from(id.checked_sub(dropped)?).ok()
    }

    /// 屏幕和历史里，编号 `from..=to` 的行里的字，第一行从第 `first_column` 列起。
    /// 折行的（上一行写满了）直接接上，别的换行
    fn text_between(&self, from: u64, to: u64, first_column: usize) -> String {
        let columns = self.primary.columns;
        let mut text = String::new();
        let mut wrapped = false;
        for id in from..=to {
            let Some(line) = self.line_by_id(id) else {
                continue;
            };
            let skip = if id == from { first_column } else { 0 };
            let row: String = line
                .iter()
                .enumerate()
                .filter(|(column, cell)| *column >= skip && cell.width != 0)
                .map(|(_, cell)| cell.ch)
                .collect();
            if id != from && !wrapped {
                text.push('\n');
            }
            text.push_str(row.trim_end());
            wrapped = line.len() >= columns
                && line.last().is_some_and(|cell| cell.ch != ' ' && cell.width != 0);
            if text.chars().count() > MAX_TEXT {
                break;
            }
        }
        text.chars()
            .filter(|ch| !ch.is_control() || *ch == '\n')
            .take(MAX_TEXT)
            .collect::<String>()
            .trim()
            .to_string()
    }

    /// 第 `index` 条命令的输出（去掉前后的空行）；还没结束或者行已经被挤出历史就是 `None`
    pub fn command_output(&self, index: usize) -> Option<String> {
        let command = self.commands.get(index)?;
        let (start, end) = (command.output_start?, command.output_end?);
        if end <= start || self.line_by_id(start).is_none() {
            return None;
        }
        let text = self.text_between(start, end - 1, 0);
        Some(text.trim_matches('\n').to_string())
    }

    /// 每条命令在存下来的那段文字里的标记：绝对行 → 这一行开头要写的标记，和输入开始的列。
    /// 只收 `first..end` 行里的；同一行上的顺序是：输出开始（C）、输出结束（D）、下一个提示符（A）
    pub(super) fn mark_rows(&self, first: usize, end: usize) -> HashMap<usize, RowMarks> {
        let mut rows: HashMap<usize, RowMarks> = HashMap::new();
        let mut put = |id: u64, mark: String, input: Option<usize>| {
            let Some(line) = self.absolute_line(id).filter(|line| (first..end).contains(line)) else {
                return;
            };
            let row = rows.entry(line).or_default();
            row.before.push_str(&mark);
            row.input = row.input.or(input);
        };
        for command in &self.commands {
            put(command.prompt, "\x1b]133;A\x07".to_string(), None);
            if let Some((line, column)) = command.input {
                put(line, String::new(), Some(column));
            }
            if let Some(start) = command.output_start {
                put(start, "\x1b]133;C\x07".to_string(), None);
            }
            if let Some(stop) = command.output_end {
                let code = command.exit.map_or(String::new(), |code| format!(";{code}"));
                put(stop, format!("\x1b]133;D{code}\x07"), None);
            }
        }
        rows
    }

    /// 刚把存下的内容喂回来之后调：喂的过程里「跑完了」的命令不是新发生的事（别进命令历史、别弹通知），
    /// 而且现在的 shell 还没发过标记，不能当它在用命令标记
    pub fn finish_restore(&mut self) {
        self.finished.clear();
        self.integrated = false;
    }

    /// 所有命令，从老到新
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// 有没有收到过命令标记（shell 集成在不在工作）
    pub fn has_command_marks(&self) -> bool {
        self.integrated
    }

    /// 取走结束了的命令（给「命令跑完」通知用）
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟 shell：提示符、敲命令、输出、结束
    fn run(term: &mut Terminal, command: &str, output: &str, exit: i32) {
        term.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        term.feed(command.as_bytes());
        term.feed(b"\r\n\x1b]133;C\x07");
        term.feed(output.as_bytes());
        term.feed(format!("\x1b]133;D;{exit}\x07").as_bytes());
    }

    #[test]
    fn records_commands_with_text_output_and_exit() {
        let mut term = Terminal::new(40, 6);
        run(&mut term, "echo hi", "hi\r\n", 0);
        run(&mut term, "false", "", 1);
        run(&mut term, "ls", "a\r\nb\r\n", 0);
        term.feed(b"\x1b]133;A\x07$ ");
        assert!(term.has_command_marks());
        // 最后一个是正在等输入的提示符，还没跑过命令
        let commands = term.commands();
        assert_eq!(commands.len(), 4);
        assert!(commands[3].output_start.is_none());
        assert_eq!(
            commands.iter().take(3).map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["echo hi", "false", "ls"]
        );
        assert_eq!(commands.iter().take(3).map(|c| c.exit).collect::<Vec<_>>(), [Some(0), Some(1), Some(0)]);
        assert_eq!(term.command_output(0).as_deref(), Some("hi"));
        assert_eq!(term.command_output(2).as_deref(), Some("a\nb"));
        assert_eq!(term.command_output(1), None, "没输出");
        let finished = term.take_finished();
        assert_eq!(finished.len(), 3);
        assert_eq!(finished[1], Finished { exit: Some(1), seconds: 0, text: "false".into() });
        assert!(term.take_finished().is_empty());
    }

    /// 内容滚进历史之后，编号还认得那一行
    #[test]
    fn line_ids_survive_scrolling_into_history() {
        let mut term = Terminal::new(40, 4);
        run(&mut term, "echo first", "first-output\r\n", 0);
        for _ in 0..10 {
            run(&mut term, "filler", "x\r\n", 0);
        }
        assert!(term.screen().history_len() > 5, "已经滚进历史了");
        assert_eq!(term.command_output(0).as_deref(), Some("first-output"));
        let id = term.commands()[0].prompt;
        assert!(term.absolute_line(id).is_some_and(|line| line < term.screen().history_len()));
        assert_eq!(term.commands()[0].text, "echo first");
    }

    /// 空提示符（直接按回车）不留；副屏里的标记不理
    #[test]
    fn empty_prompts_and_alternate_screen_are_ignored() {
        let mut term = Terminal::new(40, 4);
        term.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07\r\n");
        term.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07\r\n");
        run(&mut term, "ls", "", 0);
        assert_eq!(term.commands().len(), 1);
        term.feed(b"\x1b[?1049h\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;0\x07\x1b[?1049l");
        assert_eq!(term.commands().len(), 1);
    }

    /// 命令很长折成两行：接起来；命令文字里的控制字符不留
    #[test]
    fn wrapped_commands_are_joined() {
        let mut term = Terminal::new(10, 5);
        term.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        term.feed(b"abcdefghijklmnopqrst");
        term.feed(b"\r\n\x1b]133;C\x07\x1b]133;D;0\x07");
        assert_eq!(term.commands()[0].text, "abcdefghijklmnopqrst");
    }

    /// 存下来再喂回去：命令的文字、退出码、输出、位置都还在，滚进历史的也一样；超链接跟着回来；
    /// 喂回来的不算「刚跑完」（不进命令历史、不弹通知），也不当成 shell 在用标记
    #[test]
    fn marks_and_links_survive_a_save_and_restore() {
        let mut term = Terminal::new(40, 5);
        run(&mut term, "echo first", "first-out\r\n", 0);
        run(&mut term, "false", "", 1);
        term.feed(b"\x1b]8;;https://example.com/a\x07link\x1b]8;;\x07 text\r\n");
        for index in 0..6 {
            run(&mut term, &format!("cmd{index}"), &format!("out{index}\r\n"), 0);
        }
        term.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07typing");
        assert!(term.screen().history_len() > 5, "内容已经滚进历史");
        let saved = term.transcript(1000);

        let mut restored = Terminal::new(40, 5);
        restored.feed(saved.as_bytes());
        restored.finish_restore();
        let summary = |term: &Terminal| {
            term.commands()
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    (
                        command.text.clone(),
                        command.exit,
                        term.command_output(index),
                        term.absolute_line(command.prompt),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(summary(&restored), summary(&term));
        assert_eq!(restored.commands()[0].text, "echo first");
        assert_eq!(restored.command_output(0).as_deref(), Some("first-out"));
        assert_eq!(restored.commands()[1].exit, Some(1));
        assert_eq!(restored.link_uris(), ["https://example.com/a"]);
        assert!(restored.take_finished().is_empty(), "喂回来的不算刚跑完");
        assert!(!restored.has_command_marks());
        // 再存一次一样
        assert_eq!(restored.transcript(1000), saved);
        // 恢复后新的提示符接上：最后那个还在等输入的提示符不留成一条命令
        restored.feed(b"\x1b[0m\r\n\x1b]133;A\x07$ ");
        let last = restored.commands().last().expect("有命令");
        assert!(last.output_start.is_none(), "新提示符");
        assert_eq!(restored.commands().iter().filter(|c| c.output_start.is_none()).count(), 1);
    }

    /// 只留最后几行时，更早的命令整条丢掉（不留半条）
    #[test]
    fn trimmed_history_drops_whole_commands() {
        let mut term = Terminal::new(40, 5);
        for index in 0..10 {
            run(&mut term, &format!("cmd{index}"), &format!("out{index}\r\n"), 0);
        }
        let saved = term.transcript(8);
        let mut restored = Terminal::new(40, 5);
        restored.feed(saved.as_bytes());
        let texts: Vec<_> = restored.commands().iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.is_empty() && texts.len() < 10, "{texts:?}");
        assert!(texts.iter().all(|text| text.starts_with("cmd")));
        assert_eq!(restored.command_output(0).as_deref().map(|o| o.starts_with("out")), Some(true));
    }
}
