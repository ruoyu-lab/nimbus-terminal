//! 仿真层：字节流 → 屏幕状态。
//!
//! [`Terminal`] 拿到 [`parser`] 切好的事件，按 xterm 的语义落到 [`Screen`] 上：笔（SGR）、
//! 模式（DECSET/DECRST）、主副屏、滚动区域、要回给 shell 的应答。它只认识「行、列、字、样式」，
//! 不认识窗口也不认识字符网格。
//!
//! 这一层是纯状态机，可以单独测：给它一段 shell 的输出，断言屏幕上应该出现什么。

pub mod image;
pub mod keys;
pub mod marks;
pub mod mouse;
mod parser;
pub mod screen;
pub mod search;
#[cfg(test)]
mod tests;

use unicode_width::UnicodeWidthChar;

pub use mouse::MouseMode;
pub use parser::{Event, Parser};
pub use screen::{Attributes, Cell, ColorSpec, Damage, Point, Screen};

/// 光标形状（DECSCUSR 能改）。网格那边的 `CaretShape` 由 `view` 换算
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

/// 终端开着的行为
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    /// DECAWM：写到行尾自动换行
    pub auto_wrap: bool,
    /// DECTCEM：光标可见（`\x1b[?25l` 关掉，全屏程序常这么干）
    pub cursor_visible: bool,
    /// DECCKM：方向键发 `\x1bOA` 而不是 `\x1b[A`
    pub application_cursor: bool,
    /// DECNKM：数字键盘应用模式
    pub application_keypad: bool,
    /// 括号粘贴（模式 2004）：粘贴时前后加 `\x1b[200~` `\x1b[201~`
    pub bracketed_paste: bool,
    /// IRM：插入模式，新写的字把后面的往后推
    pub insert_mode: bool,
    /// LNM：LF 同时带回车
    pub line_feed_newline: bool,
    /// 光标闪不闪
    pub cursor_blink: bool,
    /// 鼠标上报到什么程度（`?1000` / `?1002` / `?1003`）
    pub mouse: MouseMode,
    /// `?1006`：鼠标上报用 SGR 写法
    pub mouse_sgr: bool,
    /// `?1004`：窗口得到、失去焦点时告诉程序（`ESC [ I` / `ESC [ O`）
    pub focus_reporting: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            auto_wrap: true,
            cursor_visible: true,
            application_cursor: false,
            application_keypad: false,
            bracketed_paste: false,
            insert_mode: false,
            line_feed_newline: false,
            cursor_blink: true,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            focus_reporting: false,
        }
    }
}

/// 字符集：只有 ASCII 和 DEC 特殊图形（`ESC ( 0`，tmux、mc、老的 ncurses 程序拿它画表格线）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Charset {
    #[default]
    Ascii,
    LineDrawing,
}

impl Charset {
    /// 这个字符集下 `ch` 实际画什么
    fn map(self, ch: char) -> char {
        if self == Charset::Ascii {
            return ch;
        }
        match ch {
            '`' => '◆',
            'a' => '▒',
            'f' => '°',
            'g' => '±',
            'j' => '┘',
            'k' => '┐',
            'l' => '┌',
            'm' => '└',
            'n' => '┼',
            'o' => '⎺',
            'p' => '⎻',
            'q' => '─',
            'r' => '⎼',
            's' => '⎽',
            't' => '├',
            'u' => '┤',
            'v' => '┴',
            'w' => '┬',
            'x' => '│',
            'y' => '≤',
            'z' => '≥',
            '{' => 'π',
            '|' => '≠',
            '}' => '£',
            '~' => '·',
            other => other,
        }
    }
}

pub struct Terminal {
    pub primary: Screen,
    pub alternate: Screen,
    pub on_alternate: bool,
    /// 写字用的笔
    pub pen: Attributes,
    pub modes: Modes,
    /// 程序用 DECSCUSR 指定的形状；`None` 是没指定（用设置里的默认形状）
    pub cursor_shape: Option<CursorShape>,
    /// OSC 0 / 2 设的标题，空的用窗口自己的标题
    pub title: String,
    /// 响过几次铃（`view` 拿它决定闪一下还是出声）
    pub bell: u64,
    /// 程序要求弹的系统通知（OSC 9、OSC 777），还没取走的：（标题，正文）。取走前最多留 [`MAX_ALERTS`] 条
    alerts: Vec<(String, String)>,
    /// 现在写的字挂着哪个超链接（OSC 8）：`link_uris` 里的编号（从 1 起），0 是没有
    link: u32,
    /// 出现过的超链接地址，编号是下标加一。只收 `http`、`https`、`mailto`，条数封顶
    link_uris: Vec<String>,
    /// 命令标记（OSC 133）：每条命令的位置
    commands: Vec<marks::Command>,
    /// 收到过命令标记，说明 shell 集成在工作
    integrated: bool,
    /// 结束了、还没被取走的命令
    finished: Vec<marks::Finished>,
    /// 屏幕上的图片
    images: image::Images,
    /// G0、G1 现在指定的字符集，以及 SO / SI 选中了哪一个
    charsets: [Charset; 2],
    active_charset: usize,
    /// 最近写进格子的那个字（REP 重复它）
    last_char: Option<char>,
    parser: Parser,
    events: Vec<Event>,
    /// 要回给 shell 的字节（DSR、DA 的应答）
    output: Vec<u8>,
    /// 主屏光标（切副屏时存一下，切回来还原）
    saved_on_primary: Option<(Point, Attributes)>,
    /// 历史上限：整台终端复位、换屏时要带到新的屏上
    scrollback_limit: usize,
}

impl Terminal {
    pub fn new(columns: usize, rows: usize) -> Self {
        let mut primary = Screen::new(columns, rows);
        primary.records_history = true;
        Self {
            alternate: Screen::new(columns, rows),
            primary,
            on_alternate: false,
            pen: Attributes::default(),
            modes: Modes::default(),
            cursor_shape: None,
            title: String::new(),
            bell: 0,
            alerts: Vec::new(),
            link: 0,
            link_uris: Vec::new(),
            commands: Vec::new(),
            integrated: false,
            finished: Vec::new(),
            images: image::Images::new(),
            charsets: [Charset::Ascii; 2],
            active_charset: 0,
            last_char: None,
            parser: Parser::new(),
            events: Vec::new(),
            output: Vec::new(),
            saved_on_primary: None,
            scrollback_limit: screen::HISTORY_LIMIT,
        }
    }

    /// 现在显示的那块屏
    pub fn screen(&self) -> &Screen {
        if self.on_alternate {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn screen_mut(&mut self) -> &mut Screen {
        if self.on_alternate {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    /// 能往回翻的历史有多少行（副屏没有历史）
    pub fn scrollback_len(&self) -> usize {
        if self.on_alternate {
            0
        } else {
            self.primary.history_len()
        }
    }

    /// 改历史的上限（设置里的「历史行数」）
    pub fn set_scrollback_limit(&mut self, limit: usize) {
        self.scrollback_limit = limit;
        self.primary.set_history_limit(limit);
        self.alternate.set_history_limit(limit);
    }

    /// 把主屏（历史加现在的屏幕）写成一段能原样喂回终端的文字：带颜色、粗体这些 SGR，超链接（OSC 8），
    /// 命令标记（OSC 133，恢复后还能在命令之间跳、复制输出），行与行之间 `\r\n`。
    /// 末尾空白的行去掉，最多留最后 `max_lines` 行。副屏（vim、less）是临时画布，不存；图片不存。
    /// 用来关掉应用时存下终端里的内容，下次恢复标签时先喂进去
    pub fn transcript(&self, max_lines: usize) -> String {
        let screen = &self.primary;
        let mut rows: Vec<&[Cell]> = (0..screen.history_len())
            .rev()
            .filter_map(|from_end| screen.history_line(from_end))
            .map(|line| line.as_slice())
            .collect();
        rows.extend(screen.lines.iter().map(|line| line.as_slice()));
        let is_blank = |line: &[Cell]| {
            line.iter()
                .all(|cell| cell.ch == ' ' && cell.attrs.background.is_none())
        };
        while rows.last().is_some_and(|line| is_blank(line)) {
            rows.pop();
        }
        // 留下的是第 `first` 行起的（行的下标就是「绝对行」：0 是最老的历史行）
        let first = rows.len().saturating_sub(max_lines);
        let marks = self.mark_rows(first, rows.len());
        let mut text = String::new();
        for (index, line) in rows.iter().enumerate().skip(first) {
            if index > first {
                text.push_str("\r\n");
            }
            let row_marks = marks.get(&index);
            if let Some(row_marks) = row_marks {
                text.push_str(&row_marks.before);
            }
            let input = row_marks.and_then(|row_marks| row_marks.input);
            // 行尾没有样式的空格不写；输入开始的位置（提示符后面那个空格）要写到，不然恢复后位置差一格
            let end = line
                .iter()
                .rposition(|cell| *cell != Cell::PLAIN)
                .map_or(0, |last| last + 1)
                .max(input.unwrap_or(0));
            let mut pen = Attributes::default();
            let mut link = 0;
            for (column, cell) in line.iter().enumerate().take(end) {
                if input == Some(column) {
                    text.push_str("\x1b]133;B\x07");
                }
                // 宽字的右半不带字
                if cell.width == 0 {
                    continue;
                }
                // 链接不是 SGR 的一部分，单独比
                let look = Attributes {
                    link: 0,
                    ..cell.attrs
                };
                if look != pen {
                    pen = look;
                    push_sgr(&mut text, pen);
                }
                if cell.attrs.link != link {
                    link = cell.attrs.link;
                    self.push_link(&mut text, link);
                }
                text.push(if cell.ch.is_control() { ' ' } else { cell.ch });
            }
            if input.is_some_and(|column| column >= end) {
                text.push_str("\x1b]133;B\x07");
            }
            if link != 0 {
                self.push_link(&mut text, 0);
            }
            if pen != Attributes::default() {
                text.push_str("\x1b[0m");
            }
        }
        text
    }

    /// 写一个 OSC 8：`link` 是链接表的编号，0 是结束
    fn push_link(&self, text: &mut String, link: u32) {
        let uri = link
            .checked_sub(1)
            .and_then(|index| self.link_uris.get(index as usize))
            .map_or("", String::as_str);
        text.push_str("\x1b]8;;");
        text.push_str(uri);
        text.push_str("\x1b\\");
    }

    /// 主屏一共往历史里放过多少行（只增不减）
    pub fn scrollback_total(&self) -> u64 {
        self.primary.archived_total()
    }

    /// 按键编码要看的几个模式
    pub fn keymap(&self) -> keys::Keymap {
        keys::Keymap {
            application_cursor: self.modes.application_cursor,
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.primary.columns, self.primary.rows)
    }

    /// 改大小：两块屏一起改，然后要求整屏重画
    pub fn resize(&mut self, columns: usize, rows: usize) {
        self.primary.resize(columns, rows);
        self.alternate.resize(columns, rows);
        self.screen_mut().repaint_everything();
    }

    /// 取走显示屏幕的改动，交给 `view` 同步到网格
    pub fn damage(&mut self) -> Damage {
        self.screen_mut().take_damage()
    }

    /// 取走要写给 shell 的字节
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// 喂进来一块从 pty 读到的字节
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut events = std::mem::take(&mut self.events);
        events.clear();
        self.parser.feed(bytes, &mut events);
        for event in events.drain(..) {
            self.apply(event);
        }
        self.events = events;
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Print(ch) => self.print(ch),
            Event::Control(byte) => self.control(byte),
            Event::Escape {
                intermediates,
                final_byte,
            } => self.escape(intermediates, final_byte),
            Event::Csi {
                private,
                intermediates,
                params,
                subparams,
                final_byte,
            } => self.csi(private, intermediates, &params, subparams, final_byte),
            Event::Osc { code, text } => self.osc(code, &text),
            Event::Apc(content) => self.apc(&content),
            Event::Dcs(content) => self.dcs(&content),
        }
    }

    fn print(&mut self, ch: char) {
        let ch = self.charsets[self.active_charset].map(ch);
        // 组合字符（变音符号、零宽）要接到前面那一格上才不出错，那需要一格存一整个字素簇，
        // 现在先把它们丢掉：宁可少个音标，也不要错位
        let Some(width) = ch.width().filter(|width| *width > 0) else {
            return;
        };
        if width > 2 {
            return;
        }
        let pen = Attributes {
            link: self.link,
            ..self.pen
        };
        let wrap = self.modes.auto_wrap;
        if self.modes.insert_mode {
            self.screen_mut().insert_blanks(width, pen);
        }
        self.screen_mut().put(ch, width as u8, pen, wrap);
        self.last_char = Some(ch);
    }

    fn control(&mut self, byte: u8) {
        match byte {
            0x07 => self.bell += 1,
            0x08 => self.screen_mut().backspace(1),
            0x09 => self.screen_mut().tab(),
            // LF、VT、FF：往下走一行（FF 有的终端当清屏，这里跟 LF 一样）
            0x0a..=0x0c => {
                let newline = self.modes.line_feed_newline;
                if newline {
                    self.screen_mut().carriage_return();
                }
                self.screen_mut().line_feed(1);
            }
            0x0d => self.screen_mut().carriage_return(),
            // SO / SI：在 G1、G0 之间切
            0x0e => self.active_charset = 1,
            0x0f => self.active_charset = 0,
            _ => {}
        }
    }

    fn escape(&mut self, intermediates: Vec<u8>, final_byte: u8) {
        if !intermediates.is_empty() {
            // `ESC ( 0` / `ESC ( B`（G0）、`ESC ) 0`（G1）指定字符集；`ESC # 8`（对齐测试）等认不出来就整条丢
            let charset = if final_byte == b'0' {
                Charset::LineDrawing
            } else {
                Charset::Ascii
            };
            match intermediates.as_slice() {
                [b'('] => self.charsets[0] = charset,
                [b')'] => self.charsets[1] = charset,
                _ => {}
            }
            return;
        }
        match final_byte {
            b'H' => self.screen_mut().set_tab_here(),
            b'D' => self.screen_mut().line_feed(1),
            b'E' => {
                self.screen_mut().carriage_return();
                self.screen_mut().line_feed(1);
            }
            b'M' => self.screen_mut().reverse_line_feed(1),
            b'7' => {
                let pen = self.pen;
                self.screen_mut().save_cursor(pen);
            }
            b'8' => {
                let restored = self.screen_mut().restore_cursor();
                if let Some(pen) = restored {
                    self.pen = pen;
                }
            }
            b'=' => self.modes.application_keypad = true,
            b'>' => self.modes.application_keypad = false,
            b'c' => self.reset(),
            _ => {}
        }
    }

    /// `\x1b[...` 的大分发
    fn csi(
        &mut self,
        private: Option<u8>,
        intermediates: Vec<u8>,
        params: &[i32],
        subparams: u32,
        final_byte: u8,
    ) {
        // 参数缺省值按 ECMA-48：位移、位置类是 1，擦除和模式是 0
        let value = |index: usize, default: i32| param(params, index, default);
        let count = |index: usize| value(index, 1).max(1) as usize;
        match (private, final_byte) {
            (None, b'A') => self.screen_mut().cursor_up(count(0)),
            (None, b'B') => self.screen_mut().cursor_down(count(0)),
            (None, b'C') => self.screen_mut().cursor_right(count(0)),
            (None, b'D') => self.screen_mut().cursor_left(count(0)),
            (None, b'E') => {
                self.screen_mut().carriage_return();
                self.screen_mut().cursor_down(count(0));
            }
            (None, b'F') => {
                self.screen_mut().carriage_return();
                self.screen_mut().cursor_up(count(0));
            }
            (None, b'G') | (None, b'`') => {
                let row = self.screen().cursor.row;
                self.screen_mut().move_to(row, count(0).saturating_sub(1));
            }
            (None, b'd') => {
                let column = self.screen().cursor.column;
                self.screen_mut()
                    .move_to(count(0).saturating_sub(1), column);
            }
            (None, b'H') | (None, b'f') => {
                let (columns, rows) = self.screen_size();
                let row = if params.is_empty() {
                    0
                } else {
                    value(0, 1).clamp(1, rows.max(1) as i32) as usize - 1
                };
                let column = if params.len() < 2 {
                    0
                } else {
                    value(1, 1).clamp(1, columns.max(1) as i32) as usize - 1
                };
                self.screen_mut().move_to(row, column);
            }
            (None, b'J') => {
                // 清整屏（2）和连历史一起清（3）：图片也清掉
                if matches!(value(0, 0), 2 | 3) {
                    self.clear_images();
                }
                let pen = self.pen;
                self.screen_mut()
                    .erase_display(value(0, 0).max(0) as u8, pen);
            }
            (None, b'K') => {
                let pen = self.pen;
                self.screen_mut().erase_line(value(0, 0).max(0) as u8, pen);
            }
            (None, b'L') => {
                let pen = self.pen;
                self.screen_mut().insert_lines(count(0), pen);
            }
            (None, b'M') => {
                let pen = self.pen;
                self.screen_mut().delete_lines(count(0), pen);
            }
            (None, b'@') => {
                let pen = self.pen;
                self.screen_mut().insert_blanks(count(0), pen);
            }
            (None, b'P') => {
                let pen = self.pen;
                self.screen_mut().delete_chars(count(0), pen);
            }
            (None, b'X') => {
                let pen = self.pen;
                self.screen_mut().erase_chars(count(0), pen);
            }
            // CHT：往前跳 n 个制表位
            (None, b'I') => {
                for _ in 0..count(0).min(self.screen().columns) {
                    self.screen_mut().tab();
                }
            }
            // REP：重复上一个字 n 遍（ncurses 靠它压缩重复的字符，xterm-256color 的 terminfo 里有 `rep`）
            (None, b'b') => {
                if let Some(ch) = self.last_char {
                    let limit = self.screen().columns * self.screen().rows;
                    for _ in 0..count(0).min(limit) {
                        self.print(ch);
                    }
                }
            }
            // TBC：清制表位
            (None, b'g') => match value(0, 0) {
                0 => self.screen_mut().clear_tab_here(),
                3 => self.screen_mut().clear_all_tabs(),
                _ => {}
            },
            // DECSTR：软复位（笔、模式、滚动区域、字符集回默认；屏幕内容留着）
            (Some(b'!'), b'p') => self.soft_reset(),
            (None, b'S') => self.screen_mut().scroll_up(count(0)),
            (None, b'T') => self.screen_mut().scroll_down(count(0)),
            (None, b'Z') => self.screen_mut().back_tab(count(0)),
            (None, b'm') => self.sgr(params, subparams),
            (None, b'r') => {
                let (_, rows) = self.screen_size();
                let top = count(0);
                let bottom = if params.len() < 2 {
                    rows
                } else {
                    value(1, rows as i32).clamp(1, rows.max(1) as i32) as usize
                };
                self.screen_mut().set_region(top.saturating_sub(1), bottom);
            }
            (None, b'h') | (None, b'l') => self.set_modes(None, params, final_byte == b'h'),
            (Some(b'?'), b'h') | (Some(b'?'), b'l') => {
                let on = final_byte == b'h';
                self.set_modes(Some(b'?'), params, on);
                // 换屏、光标显隐都要重画：网格只有一个，屏幕上显示的是哪一块变了
                if params.iter().any(|p| matches!(p, 25 | 47 | 1047 | 1049)) {
                    self.screen_mut().repaint_everything();
                }
            }
            (None, b's') => {
                let pen = self.pen;
                self.screen_mut().save_cursor(pen);
            }
            (None, b'u') => {
                if let Some(pen) = self.screen_mut().restore_cursor() {
                    self.pen = pen;
                }
            }
            (None, b'n') => self.device_status_report(value(0, 0)),
            (Some(b'?'), b'n') => self.output.extend_from_slice(b"\x1b[?0n"),
            // 终端类型：62 = VT420，22 = 支持 ANSI 颜色。问这个的程序（`tic`、一些老脚本）
            // 拿到不认识的答复会退回最保守的行为，所以宁可报得朴素
            (None, b'c') => self.output.extend_from_slice(b"\x1b[?62;22c"),
            (Some(b'>'), b'c') => self.output.extend_from_slice(b"\x1b[>1;10;0c"),
            // DECSCUSR：光标的形状
            (None, b'q') if intermediates.first() == Some(&b' ') => {
                self.set_cursor_shape(value(0, 0))
            }
            _ => {}
        }
    }

    fn screen_size(&self) -> (usize, usize) {
        let screen = self.screen();
        (screen.columns, screen.rows)
    }

    fn device_status_report(&mut self, which: i32) {
        match which {
            5 => self.output.extend_from_slice(b"\x1b[0n"),
            6 => {
                let cursor = self.screen().cursor;
                let text = format!("\x1b[{};{}R", cursor.row + 1, cursor.column + 1);
                self.output.extend_from_slice(text.as_bytes());
            }
            _ => {}
        }
    }

    fn set_cursor_shape(&mut self, param: i32) {
        let (shape, blink) = match param {
            // 0 是「回到默认」：形状和闪烁都交还给设置
            0 => (None, true),
            1 => (Some(CursorShape::Block), true),
            2 => (Some(CursorShape::Block), false),
            3 => (Some(CursorShape::Underline), true),
            4 => (Some(CursorShape::Underline), false),
            5 => (Some(CursorShape::Bar), true),
            6 => (Some(CursorShape::Bar), false),
            _ => return,
        };
        self.cursor_shape = shape;
        self.modes.cursor_blink = blink;
    }

    fn set_modes(&mut self, private: Option<u8>, params: &[i32], on: bool) {
        for &param in params {
            match (private, param) {
                (None, 4) => self.modes.insert_mode = on,
                (None, 12) => self.modes.cursor_blink = on,
                (None, 20) => self.modes.line_feed_newline = on,
                (Some(b'?'), 1) => self.modes.application_cursor = on,
                (Some(b'?'), 7) => self.modes.auto_wrap = on,
                (Some(b'?'), 12) => self.modes.cursor_blink = on,
                (Some(b'?'), 25) => self.modes.cursor_visible = on,
                (Some(b'?'), 2004) => self.modes.bracketed_paste = on,
                (Some(b'?'), 47) | (Some(b'?'), 1047) => self.set_alternate(on, false),
                (Some(b'?'), 1049) => self.set_alternate(on, true),
                (Some(b'?'), 1000) => self.set_mouse_mode(MouseMode::Press, on),
                (Some(b'?'), 1002) => self.set_mouse_mode(MouseMode::Drag, on),
                (Some(b'?'), 1003) => self.set_mouse_mode(MouseMode::Motion, on),
                (Some(b'?'), 1006) => self.modes.mouse_sgr = on,
                (Some(b'?'), 1004) => self.modes.focus_reporting = on,
                // 不认的模式（1005、1015 的别的鼠标写法、2026 同步输出……）原样丢掉比存着不用更安全
                _ => {}
            }
        }
    }

    /// 开、关一档鼠标上报。关的时候只关自己那一档：程序先开 1003 再关 1000 不该把 1003 也关掉
    fn set_mouse_mode(&mut self, mode: MouseMode, on: bool) {
        if on {
            self.modes.mouse = mode;
        } else if self.modes.mouse == mode {
            self.modes.mouse = MouseMode::Off;
        }
    }

    /// 主副屏切换（`smcup` / `rmcup`）
    fn set_alternate(&mut self, on: bool, clear_and_save: bool) {
        if on == self.on_alternate {
            return;
        }
        let (columns, rows) = (self.primary.columns, self.primary.rows);
        if on {
            if clear_and_save {
                let pen = self.pen;
                self.saved_on_primary = Some((self.primary.cursor, pen));
            }
            // 每次进副屏都是干净的一块：全屏程序自己会重画
            self.alternate = Screen::new(columns, rows);
            self.on_alternate = true;
        } else {
            self.on_alternate = false;
            if clear_and_save && let Some((cursor, pen)) = self.saved_on_primary.take() {
                self.primary.move_to(cursor.row, cursor.column);
                self.pen = pen;
            }
        }
        self.screen_mut().repaint_everything();
    }

    /// DECSTR：软复位。程序退出时没收拾干净的状态（隐藏的光标、卷起的滚动区域）靠它救回来
    fn soft_reset(&mut self) {
        self.pen = Attributes::default();
        self.link = 0;
        self.modes = Modes::default();
        self.cursor_shape = None;
        self.charsets = [Charset::Ascii; 2];
        self.active_charset = 0;
        self.screen_mut().reset_region();
    }

    /// RIS：整台终端复位
    fn reset(&mut self) {
        let (columns, rows) = (self.primary.columns, self.primary.rows);
        let mut primary = Screen::new(columns, rows);
        primary.records_history = true;
        primary.set_history_limit(self.scrollback_limit);
        self.primary = primary;
        self.alternate = Screen::new(columns, rows);
        self.on_alternate = false;
        self.pen = Attributes::default();
        self.link = 0;
        self.modes = Modes::default();
        self.cursor_shape = None;
        self.charsets = [Charset::Ascii; 2];
        self.active_charset = 0;
        self.last_char = None;
        self.parser = Parser::new();
        self.screen_mut().repaint_everything();
    }

    /// SGR：把参数串翻译成笔的变化。
    ///
    /// 不带参数的 `CSI m` 等于 `CSI 0 m`（复位）；冒号后面的子参数跟着前一个参数走：
    /// `4:3`（弯曲下划线）不能被当成「4 和 3」两个独立的码，不然会顺手把斜体打开
    fn sgr(&mut self, params: &[i32], subparams: u32) {
        if params.is_empty() {
            self.pen = Attributes::default();
            return;
        }
        let is_sub = |index: usize| index < 32 && subparams >> index & 1 == 1;
        let mut index = 0;
        while index < params.len() {
            let code = params[index].max(0) as u16;
            let mut end = index + 1;
            while end < params.len() && is_sub(end) {
                end += 1;
            }
            let subs = &params[index + 1..end];
            if !subs.is_empty() {
                // 冒号写法：只认下划线样式（`4:0` 关，其余开）和 38 / 48 的颜色，别的整组跳过
                match code {
                    4 => self.pen.underline = subs[0] != 0,
                    38 | 48 => {
                        if let Some(color) = colon_color(subs) {
                            if code == 38 {
                                self.pen.foreground = Some(color);
                            } else {
                                self.pen.background = Some(color);
                            }
                        }
                    }
                    _ => {}
                }
                index = end;
                continue;
            }
            index += 1;
            match code {
                0 => self.pen = Attributes::default(),
                1 => self.pen.bold = true,
                2 => self.pen.dim = true,
                3 => self.pen.italic = true,
                4 => self.pen.underline = true,
                5 | 6 => self.pen.blink = true,
                7 => self.pen.inverse = true,
                8 => self.pen.hidden = true,
                9 => self.pen.strikethrough = true,
                21 | 22 => {
                    self.pen.bold = false;
                    self.pen.dim = false;
                }
                23 => self.pen.italic = false,
                24 => self.pen.underline = false,
                25 => self.pen.blink = false,
                27 => self.pen.inverse = false,
                28 => self.pen.hidden = false,
                29 => self.pen.strikethrough = false,
                30..=37 => self.pen.foreground = Some(ColorSpec::Indexed((code - 30) as u8)),
                39 => self.pen.foreground = None,
                40..=47 => self.pen.background = Some(ColorSpec::Indexed((code - 40) as u8)),
                49 => self.pen.background = None,
                90..=97 => self.pen.foreground = Some(ColorSpec::Indexed((code - 90 + 8) as u8)),
                100..=107 => self.pen.background = Some(ColorSpec::Indexed((code - 100 + 8) as u8)),
                // 38 / 48 后面跟 `5;n`（256 色）或 `2;r;g;b`（直接色）
                38 | 48 => {
                    let Some((color, consumed)) = extended_color(&params[index..]) else {
                        break;
                    };
                    if code == 38 {
                        self.pen.foreground = Some(color);
                    } else {
                        self.pen.background = Some(color);
                    }
                    index += consumed;
                }
                _ => {}
            }
        }
    }

    /// OSC：只认标题（0 和 2；1 是图标名，没有图标，忽略）。
    /// 剪贴板（52）、超链接（8）、目录（7）、提示符标记（133）还没做，整条丢掉
    fn osc(&mut self, code: i32, text: &str) {
        match code {
            8 => self.hyperlink(text),
            133 => self.semantic_prompt(text),
            1337 => self.iterm_image(text),
            // iTerm 的写法：`9;正文`。`9;4;…` 是 ConEmu 的进度条，不是通知
            9 if !text.starts_with("4;") => self.push_alert(String::new(), text),
            // `777;notify;标题;正文`（rxvt、Kitty 等）
            777 => {
                if let Some(rest) = text.strip_prefix("notify;") {
                    let (title, body) = rest.split_once(';').unwrap_or(("", rest));
                    self.push_alert(title.to_string(), body);
                }
            }
            _ => {}
        }
        if matches!(code, 0 | 2) {
            // 标题会显示在窗口上：控制字符去掉，长度封顶，免得程序拿它刷屏
            self.title = text
                .chars()
                .filter(|ch| !ch.is_control())
                .take(256)
                .collect();
        }
    }
}

/// 一次取走之前最多攒几条通知：程序刷屏式地发通知时不会攒出一大堆
pub const MAX_ALERTS: usize = 4;

/// 链接表最多这么多条：程序不断换新地址时不会无限涨内存（满了以后新的链接当普通文字）
const MAX_LINKS: usize = 4096;

impl Terminal {
    /// OSC 8：`参数;地址`，地址为空是结束这段链接。只认 http、https、mailto：别的协议点一下就交给系统
    /// 是危险的（`file:`、自定义协议）
    fn hyperlink(&mut self, text: &str) {
        let uri = text.split_once(';').map_or("", |(_, uri)| uri);
        let allowed = ["http://", "https://", "mailto:"].iter().any(|scheme| {
            uri.get(..scheme.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
        });
        if uri.is_empty() || !allowed || uri.len() > 2048 || uri.chars().any(char::is_control) {
            self.link = 0;
            return;
        }
        self.link = match self.link_uris.iter().position(|known| known == uri) {
            Some(index) => index as u32 + 1,
            None if self.link_uris.len() < MAX_LINKS => {
                self.link_uris.push(uri.to_string());
                self.link_uris.len() as u32
            }
            None => 0,
        };
    }

    /// 出现过的超链接地址（`Attributes::link` 减一是下标）
    pub fn link_uris(&self) -> &[String] {
        &self.link_uris
    }

    fn push_alert(&mut self, title: String, body: &str) {
        // 通知是显示给人看的：控制字符去掉、长度封顶，空的不发
        let clean = |text: &str, limit: usize| -> String {
            text.chars()
                .filter(|ch| !ch.is_control())
                .take(limit)
                .collect()
        };
        let (title, body) = (clean(&title, 80), clean(body, 200));
        if (title.is_empty() && body.is_empty()) || self.alerts.len() >= MAX_ALERTS {
            return;
        }
        self.alerts.push((title, body));
    }

    /// 取走程序要求弹的通知
    pub fn take_alerts(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.alerts)
    }
}

/// 解析分号写法的 `5;n` / `2;r;g;b`（`38;` 或 `48;` 后面的部分），返回颜色和吃掉了几个参数
fn extended_color(rest: &[i32]) -> Option<(ColorSpec, usize)> {
    match (*rest.first()?).max(0) {
        5 => Some((indexed(*rest.get(1)?), 2)),
        2 => Some((rgb(*rest.get(1)?, *rest.get(2)?, *rest.get(3)?), 4)),
        _ => None,
    }
}

/// 解析冒号写法的 `5:n` / `2:r:g:b` / `2:色彩空间:r:g:b`（色彩空间那一格常常留空）
fn colon_color(subs: &[i32]) -> Option<ColorSpec> {
    match (*subs.first()?).max(0) {
        5 => Some(indexed(*subs.get(1)?)),
        2 if subs.len() >= 5 => Some(rgb(subs[2], subs[3], subs[4])),
        2 => Some(rgb(*subs.get(1)?, *subs.get(2)?, *subs.get(3)?)),
        _ => None,
    }
}

fn indexed(value: i32) -> ColorSpec {
    ColorSpec::Indexed(value.clamp(0, 255) as u8)
}

fn rgb(red: i32, green: i32, blue: i32) -> ColorSpec {
    let channel = |value: i32| value.clamp(0, 255) as u32;
    ColorSpec::Rgb(channel(red) << 16 | channel(green) << 8 | channel(blue))
}

/// 第 `index` 个参数：没写、写空（解析器给 -1）时用默认值
fn param(params: &[i32], index: usize, default: i32) -> i32 {
    match params.get(index) {
        Some(&value) if value >= 0 => value,
        _ => default,
    }
}

/// 把一支笔写成一条 SGR：先复位再逐项打开
fn push_sgr(text: &mut String, pen: Attributes) {
    use std::fmt::Write;

    text.push_str("\x1b[0");
    for (on, code) in [
        (pen.bold, 1),
        (pen.dim, 2),
        (pen.italic, 3),
        (pen.underline, 4),
        (pen.blink, 5),
        (pen.inverse, 7),
        (pen.hidden, 8),
        (pen.strikethrough, 9),
    ] {
        if on {
            let _ = write!(text, ";{code}");
        }
    }
    for (color, base) in [(pen.foreground, 38), (pen.background, 48)] {
        match color {
            Some(ColorSpec::Indexed(index)) => {
                let _ = write!(text, ";{base};5;{index}");
            }
            Some(ColorSpec::Rgb(value)) => {
                let _ = write!(
                    text,
                    ";{base};2;{};{};{}",
                    (value >> 16) & 0xff,
                    (value >> 8) & 0xff,
                    value & 0xff
                );
            }
            None => {}
        }
    }
    text.push('m');
}
