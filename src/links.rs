//! 终端文字里的链接：⌘ 加点击打开。认两种：网址（`http://`、`https://`）和文件路径（`/…`、`~/…`、`./…`、`../…`，
//! 后面可以跟 `:行号` 或 `:行号:列号`，编译器和 grep 的输出都是这么写的）。
//!
//! 一次只看一行里的字，折成两行的长网址认不全（和查找一样，终端不记哪些行是同一行折出来的）。

use crate::vt::Cell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    /// 路径原样（还没展开 `~`、没接当前目录），以及后面跟的行号
    Path {
        path: String,
        line: Option<u32>,
    },
}

/// 链接两边的界：碰到这些就断开
fn is_break(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(
            ch,
            '"' | '\''
                | '`'
                | '<'
                | '>'
                | '|'
                | '（'
                | '）'
                | '「'
                | '」'
                | '“'
                | '”'
                | '，'
                | '。'
        )
}

/// 一行里所有的链接，连同各自占的列范围（含头不含尾）。`uris` 是终端的超链接表（OSC 8）：
/// 程序明说是链接的那一段先算，里面的字不再当网址、路径另外去猜
pub fn scan(line: &[Cell], uris: &[String]) -> Vec<(Link, usize, usize)> {
    let mut found = explicit_links(line, uris);
    // 宽字的右半不带字：记下每个字从第几列开始、占几列
    let chars: Vec<(char, usize, usize)> = line
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.width != 0)
        .map(|(index, cell)| (cell.ch, index, usize::from(cell.width.max(1))))
        .collect();
    let mut index = 0;
    while index < chars.len() {
        if is_break(chars[index].0) {
            index += 1;
            continue;
        }
        let mut last = index;
        while last + 1 < chars.len() && !is_break(chars[last + 1].0) {
            last += 1;
        }
        if let Some(hit) = token_link(&chars, index, last)
            && !found
                .iter()
                .any(|(_, start, end)| hit.1 < *end && *start < hit.2)
        {
            found.push(hit);
        }
        index = last + 1;
    }
    found.sort_by_key(|(_, start, _)| *start);
    found
}

/// OSC 8 超链接：连续的、挂着同一个链接编号的格子是一段
fn explicit_links(line: &[Cell], uris: &[String]) -> Vec<(Link, usize, usize)> {
    let mut found = Vec::new();
    let mut column = 0;
    while column < line.len() {
        let id = line[column].attrs.link;
        if id == 0 {
            column += 1;
            continue;
        }
        let start = column;
        while column < line.len() && line[column].attrs.link == id {
            column += 1;
        }
        if let Some(uri) = uris.get(id as usize - 1) {
            found.push((Link::Url(uri.clone()), start, column));
        }
    }
    found
}

/// 第 `column` 列上的链接，连同它占的列范围
pub fn link_at(line: &[Cell], column: usize, uris: &[String]) -> Option<(Link, usize, usize)> {
    scan(line, uris)
        .into_iter()
        .find(|(_, start, end)| (*start..*end).contains(&column))
}

/// 一段不含界的字（`chars[first..=last]`）里的链接
fn token_link(
    chars: &[(char, usize, usize)],
    mut first: usize,
    last: usize,
) -> Option<(Link, usize, usize)> {
    // 头上的括号、尾上的标点不算链接的一部分；括号成对的（维基百科的网址）留着
    let mut text: Vec<char> = chars[first..=last].iter().map(|&(ch, _, _)| ch).collect();
    while text.first().is_some_and(|ch| matches!(ch, '(' | '[' | '{')) {
        text.remove(0);
        first += 1;
    }
    while let Some(&end) = text.last() {
        let open = match end {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            '.' | ',' | ';' | ':' | '!' | '?' => '\0',
            _ => break,
        };
        // 右括号：前面有配对的左括号就留着
        if open != '\0'
            && text.iter().filter(|&&ch| ch == open).count()
                >= text.iter().filter(|&&ch| ch == end).count()
        {
            break;
        }
        text.pop();
    }
    if text.is_empty() {
        return None;
    }
    let last = first + text.len() - 1;
    let value: String = text.iter().collect();
    let link = classify(&value)?;
    Some((link, chars[first].1, chars[last].1 + chars[last].2))
}

/// 点一下就用默认应用打开也不要紧的文件：文档、图片、文本、代码。别的（可执行的、安装包）不直接开
pub fn is_plain_document(path: &std::path::Path) -> bool {
    const PLAIN: [&str; 40] = [
        "txt", "md", "log", "json", "toml", "yaml", "yml", "csv", "pdf", "png", "jpg", "jpeg",
        "gif", "webp", "svg", "html", "htm", "css", "rs", "py", "js", "ts", "tsx", "jsx", "go",
        "c", "h", "cpp", "java", "kt", "swift", "rb", "php", "xml", "ini", "conf", "cfg", "lock",
        "diff", "patch",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| PLAIN.contains(&extension.to_ascii_lowercase().as_str()))
}

fn classify(text: &str) -> Option<Link> {
    let lower = text.to_ascii_lowercase();
    for scheme in ["http://", "https://"] {
        if lower.starts_with(scheme) {
            // 域名那一段不能是空的
            return (text.len() > scheme.len()).then(|| Link::Url(text.to_string()));
        }
    }
    if !(text.starts_with('/')
        || text.starts_with("~/")
        || text.starts_with("./")
        || text.starts_with("../"))
    {
        return None;
    }
    // `路径:行号[:列号]`：从后面剥掉纯数字的段
    let mut path = text;
    let mut numbers = Vec::new();
    while let Some((head, tail)) = path.rsplit_once(':') {
        if !tail.is_empty() && tail.chars().all(|ch| ch.is_ascii_digit()) && numbers.len() < 2 {
            numbers.push(tail.parse::<u32>().ok()?);
            path = head;
        } else {
            break;
        }
    }
    if path.len() < 2 || path == "//" {
        return None;
    }
    numbers.reverse();
    Some(Link::Path {
        path: path.to_string(),
        line: numbers.first().copied(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Terminal;

    fn line_of(text: &str) -> Vec<Cell> {
        let mut terminal = Terminal::new(120, 2);
        terminal.feed(text.as_bytes());
        terminal.screen().lines[0].clone()
    }

    fn at(text: &str, needle: &str, offset: usize) -> Option<(Link, usize, usize)> {
        let line = line_of(text);
        let column = text.find(needle).expect("有这段字") + offset;
        link_at(&line, column, &[])
    }

    #[test]
    fn finds_an_url_and_its_columns() {
        let (link, start, end) = at("see https://example.com/a?b=1 now", "https", 3).expect("链接");
        assert_eq!(link, Link::Url("https://example.com/a?b=1".into()));
        assert_eq!((start, end), (4, 29));
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_link() {
        let (link, ..) = at("open https://example.com/docs.", "https", 0).expect("链接");
        assert_eq!(link, Link::Url("https://example.com/docs".into()));
        let (link, ..) = at("(see https://example.com/x)", "https", 0).expect("链接");
        assert_eq!(link, Link::Url("https://example.com/x".into()));
        let (link, ..) =
            at("https://en.wikipedia.org/wiki/Rust_(language)", "https", 0).expect("链接");
        assert_eq!(
            link,
            Link::Url("https://en.wikipedia.org/wiki/Rust_(language)".into())
        );
    }

    #[test]
    fn quoted_urls_stop_at_the_quote() {
        let (link, ..) = at("href=\"https://example.com/p\" class", "https", 0).expect("链接");
        assert_eq!(link, Link::Url("https://example.com/p".into()));
    }

    #[test]
    fn not_on_the_link_finds_nothing() {
        assert!(at("see https://example.com now", "see", 0).is_none());
        assert!(at("see https://example.com now", "now", 0).is_none());
        // 光有协议没有域名不算
        assert!(at("https://", "https", 0).is_none());
        // 别的协议不认（`javascript:`、`file:` 之类的不能点一下就开）
        assert!(at("javascript:alert(1)", "java", 0).is_none());
        assert!(at("file:///etc/passwd", "file", 0).is_none());
    }

    #[test]
    fn finds_paths_with_line_and_column() {
        let (link, ..) = at("error in ./src/main.rs:42:7: bad", "./src", 3).expect("路径");
        assert_eq!(
            link,
            Link::Path {
                path: "./src/main.rs".into(),
                line: Some(42)
            }
        );
        let (link, ..) = at("~/notes/todo.md", "~/", 0).expect("路径");
        assert_eq!(
            link,
            Link::Path {
                path: "~/notes/todo.md".into(),
                line: None
            }
        );
        let (link, ..) = at("cat /var/log/system.log", "/var", 0).expect("路径");
        assert_eq!(
            link,
            Link::Path {
                path: "/var/log/system.log".into(),
                line: None
            }
        );
        // 普通的词、相对路径（没有 ./）、单个斜杠不算
        assert!(at("a/b/c", "a/b", 0).is_none());
        assert!(at("/ alone", "/", 0).is_none());
    }

    /// 宽字在前面：列按格子算
    #[test]
    fn columns_count_wide_characters() {
        let line = line_of("你好 https://a.cn/x");
        let (link, start, end) = link_at(&line, 6, &[]).expect("链接");
        assert_eq!(link, Link::Url("https://a.cn/x".into()));
        assert_eq!((start, end), (5, 19));
        // 点在宽字上不是链接
        assert!(link_at(&line, 1, &[]).is_none());
    }
}
