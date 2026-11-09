//! 在屏幕和滚动历史里查找文字。
//!
//! 行的编号是「绝对行」：0 是最老的那条历史，历史有 `n` 行时屏幕第 `r` 行是 `n + r`。
//! 一次只在一行里找，跨过自动换行的那一处找不到（和 xterm、iTerm 的做法一样，终端不记哪些行是同一行折出来的）。

use super::screen::Screen;

/// 一处命中：绝对行里的 `start..end` 列（含头不含尾，宽字占两列）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// 命中太多时留到这里为止：一个字母搜上十万行，高亮和计数都没有意义，还拖慢重画
pub const LIMIT: usize = 5000;

/// 查询里有大写字母就区分大小写，全小写就不区分（vim 的 smartcase）
pub fn is_case_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

fn fold(ch: char, case_sensitive: bool) -> char {
    if case_sensitive {
        return ch;
    }
    let mut lower = ch.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(only), None) => only,
        _ => ch,
    }
}

/// 在历史加屏幕里找 `query`，按位置从老到新排。空查询没有命中
pub fn find(screen: &Screen, query: &str, case_sensitive: bool) -> Vec<Match> {
    let needle: Vec<char> = query.chars().map(|ch| fold(ch, case_sensitive)).collect();
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let history = screen.history_len();
    for line in 0..history + screen.rows {
        let cells = if line < history {
            screen.history_line(history - 1 - line)
        } else {
            screen.lines.get(line - history)
        };
        if let Some(cells) = cells {
            search_line(cells, line, &needle, case_sensitive, &mut found);
        }
        if found.len() >= LIMIT {
            found.truncate(LIMIT);
            break;
        }
    }
    found
}

fn search_line(
    cells: &[super::screen::Cell],
    line: usize,
    needle: &[char],
    case_sensitive: bool,
    found: &mut Vec<Match>,
) {
    // 宽字的右半（width == 0）不带字；记下每个字从哪一列开始、占几列
    let mut text = Vec::with_capacity(cells.len());
    let mut columns = Vec::with_capacity(cells.len());
    for (column, cell) in cells.iter().enumerate() {
        if cell.width == 0 {
            continue;
        }
        text.push(fold(cell.ch, case_sensitive));
        columns.push((column, usize::from(cell.width.max(1))));
    }
    if text.len() < needle.len() {
        return;
    }
    let mut at = 0;
    while at + needle.len() <= text.len() {
        if text[at..at + needle.len()] == *needle {
            let last = at + needle.len() - 1;
            found.push(Match {
                line,
                start: columns[at].0,
                end: columns[last].0 + columns[last].1,
            });
            // 命中之间不重叠：在 "aaaa" 里找 "aa" 是两处
            at += needle.len();
        } else {
            at += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Terminal;

    fn terminal(text: &str, columns: usize, rows: usize) -> Terminal {
        let mut terminal = Terminal::new(columns, rows);
        terminal.feed(text.as_bytes());
        terminal
    }

    #[test]
    fn finds_in_screen_and_history_oldest_first() {
        // 3 行的屏幕写 5 行：hello 0、1 在历史里（绝对行 0、1），2、3、4 在屏幕上
        let terminal = terminal("hello 0\r\nhello 1\r\nhello 2\r\nother\r\nhello 4", 20, 3);
        let screen = terminal.screen();
        let history = screen.history_len();
        assert_eq!(history, 2);
        let found = find(screen, "hello", false);
        let lines: Vec<_> = found.iter().map(|hit| hit.line).collect();
        assert_eq!(lines, [0, 1, 2, 4]);
        assert!(found.iter().all(|hit| hit.start == 0 && hit.end == 5));
    }

    #[test]
    fn empty_query_finds_nothing() {
        let terminal = terminal("abc", 10, 3);
        assert!(find(terminal.screen(), "", false).is_empty());
    }

    #[test]
    fn case_follows_smart_case() {
        let terminal = terminal("Error error ERROR", 30, 2);
        assert!(!is_case_sensitive("error"));
        assert!(is_case_sensitive("Error"));
        assert_eq!(find(terminal.screen(), "error", false).len(), 3);
        let strict = find(terminal.screen(), "Error", true);
        assert_eq!(strict.len(), 1);
        assert_eq!((strict[0].start, strict[0].end), (0, 5));
    }

    #[test]
    fn matches_do_not_overlap() {
        let terminal = terminal("aaaa", 10, 2);
        let found = find(terminal.screen(), "aa", false);
        assert_eq!(found.len(), 2);
        assert_eq!((found[1].start, found[1].end), (2, 4));
    }

    /// 宽字占两列：命中的列范围按格子算，不按字数算
    #[test]
    fn columns_count_wide_characters() {
        let terminal = terminal("你好 world 世界", 30, 2);
        let screen = terminal.screen();
        let world = find(screen, "world", false);
        assert_eq!((world[0].start, world[0].end), (5, 10));
        let wide = find(screen, "世界", false);
        assert_eq!((wide[0].start, wide[0].end), (11, 15));
    }

    #[test]
    fn match_does_not_span_rows() {
        // 10 列写 15 个字：自动换行折成两行，跨行的那一处找不到（已知的限制）
        let terminal = terminal("aaaaaaaaaaXYZbbbbb", 10, 3);
        assert!(find(terminal.screen(), "aXYZ", false).is_empty());
        assert_eq!(find(terminal.screen(), "XYZ", false).len(), 1);
    }

    #[test]
    fn stops_at_the_limit() {
        let mut terminal = Terminal::new(40, 5);
        for _ in 0..(LIMIT / 4 + 10) {
            terminal.feed(b"x x x x x\r\n");
        }
        assert_eq!(find(terminal.screen(), "x", false).len(), LIMIT);
    }
}
