//! 屏幕模型：多少行多少列、每格一个什么字、光标在哪、滚动区域、滚出去的历史。
//!
//! 这一层不认识字节（那是 [`super::parser`]），也不认识界面（那是 `view::paint`）：
//! 只接受「把某个字写到某个位置」「擦掉几行」这类操作，并且记下**哪些行变了**，
//! 让同步到字符网格时能只重画变过的行。
//!
//! # 改动怎么交给网格
//!
//! [`Screen::take_damage`] 给出两样东西：整屏上滚了几行（网格 `scroll_up` 只挪行，很便宜），
//! 以及挪完之后还要重画的行。两者的次序是固定的——**先挪、后画**——所以脏行的下标记的是
//! 「挪完之后的行号」：整屏上滚时，已经记下的脏行要跟着往上挪，不然同一批输出里
//! 「先写再滚」的行会画到错的位置。

use std::collections::VecDeque;

/// 滚出去的行最多留这么多（滚动缓冲和翻页、搜索都在这上面）
pub const HISTORY_LIMIT: usize = 10_000;

/// 颜色的写法
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    /// 256 色表的下标（0..16 是 ANSI 16 色）
    Indexed(u8),
    /// 直接给的 24 位色（`0xRRGGBB`）
    Rgb(u32),
}

/// SGR 攒出来的一支笔：写下一个字时用什么样式
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    /// 前景色，`None` 用主题的默认色
    pub foreground: Option<ColorSpec>,
    /// 背景色，`None` 透明
    pub background: Option<ColorSpec>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub inverse: bool,
    /// SGR 8：看不见（画成空格）
    pub hidden: bool,
    pub strikethrough: bool,
}

impl Attributes {
    /// 擦除、插入空白时空格上留下的样式：只有背景色（BCE）。
    /// 下划线、反色之类留在空格上，擦出来的一片区域会拖着一条线
    fn erased(self) -> Attributes {
        Attributes { background: self.background, ..Attributes::default() }
    }
}

/// 一格。宽字（汉字、emoji）占两格，右半 `width == 0`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    /// 这一格本身的宽度：1 普通，2 宽字的头，0 宽字的右半
    pub width: u8,
    pub attrs: Attributes,
}

impl Cell {
    pub const PLAIN: Cell = Cell {
        ch: ' ',
        width: 1,
        attrs: Attributes {
            foreground: None,
            background: None,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            blink: false,
            inverse: false,
            hidden: false,
            strikethrough: false,
        },
    };

    /// 带样式（擦除时用当前的笔，背景色要留下来）的空格
    pub const fn blank(attrs: Attributes) -> Cell {
        Cell { ch: ' ', width: 1, attrs }
    }

    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.width == 1
    }
}

/// 一行。屏幕上的行总是满列宽；滚进历史的行会去掉行尾的默认空格（省内存），
/// 读历史时自己补齐
pub type Line = Vec<Cell>;

/// 格子里的位置
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

impl Point {
    pub fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

/// 一次同步要重画什么
#[derive(Clone, Debug, Default)]
pub struct Damage {
    /// 整屏往上滚了几行：网格 `scroll_up` 只是挪行，比逐行重画便宜得多。**先挪**
    pub scrolled_out: usize,
    /// 挪完之后要重画的行，从小到大（整屏重画时是全部行）
    pub rows: Vec<usize>,
}

/// 一块屏幕：主屏或者副屏（`smcup` 切过去的那个）
pub struct Screen {
    pub columns: usize,
    pub rows: usize,
    pub lines: VecDeque<Line>,
    pub cursor: Point,
    /// 写在最后一列之后先不换行，等下一个字才换（DEC 的延迟换行，`vim`、`less` 依赖它）
    pub pending_wrap: bool,
    /// 滚动区域（DECSTBM），行的下标区间，含头不含尾
    pub region_top: usize,
    pub region_bottom: usize,
    saved: Option<(Point, Attributes)>,
    /// 制表位，`true` 表示这一列是制表位（默认每 8 列一个）
    tabs: Vec<bool>,
    /// 每行变没变。写一个字就要标一次，所以用平铺的数组，不用哈希表
    dirty: Vec<bool>,
    scrolled_out: usize,
    /// 改动太散（换列数、换屏），整屏重画更划算也更保险
    repaint_all: bool,
    /// 滚出去的历史。只有主屏记（副屏是给全屏程序用的临时画布）
    history: VecDeque<Line>,
    /// 一共往历史里放过多少行（历史满了挤掉旧的也照样累加）。回看时靠它知道「又滚出去几行」，
    /// 好让看着的那一屏不跟着动
    archived_total: u64,
    pub records_history: bool,
}

impl Screen {
    pub fn new(columns: usize, rows: usize) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        let mut screen = Self {
            columns,
            rows,
            lines: VecDeque::with_capacity(rows),
            cursor: Point::default(),
            pending_wrap: false,
            region_top: 0,
            region_bottom: rows,
            saved: None,
            tabs: Vec::new(),
            dirty: vec![false; rows],
            scrolled_out: 0,
            repaint_all: true,
            history: VecDeque::new(),
            archived_total: 0,
            records_history: false,
        };
        for _ in 0..rows {
            screen.lines.push_back(vec![Cell::PLAIN; columns]);
        }
        screen.reset_tabs();
        screen
    }

    fn reset_tabs(&mut self) {
        self.tabs = (0..self.columns).map(|column| column % 8 == 0).collect();
    }

    /// 记下这一行变了
    fn mark(&mut self, row: usize) {
        if let Some(flag) = self.dirty.get_mut(row) {
            *flag = true;
        }
    }

    /// 记下 `top..bottom` 这几行变了
    fn mark_range(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.dirty.len());
        if top < bottom {
            self.dirty[top..bottom].fill(true);
        }
    }

    /// 整屏重画（换列数、换屏之后调用）
    pub fn repaint_everything(&mut self) {
        self.repaint_all = true;
    }

    /// 取走这次的改动，交给网格同步
    pub fn take_damage(&mut self) -> Damage {
        let rows = if self.repaint_all {
            (0..self.lines.len()).collect()
        } else {
            self.dirty.iter().enumerate().filter(|(_, dirty)| **dirty).map(|(row, _)| row).collect()
        };
        self.repaint_all = false;
        self.dirty.fill(false);
        Damage { scrolled_out: std::mem::take(&mut self.scrolled_out), rows }
    }

    /// 滚出屏幕的一行进历史：行尾的默认空格去掉，一行只占它真正用到的那么多
    fn archive(&mut self, mut line: Line) {
        while line.last() == Some(&Cell::PLAIN) {
            line.pop();
        }
        line.shrink_to_fit();
        self.archived_total += 1;
        self.history.push_back(line);
        if self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
    }

    /// 改大小。行数变少时，光标下面的行先扔（光标要留在看得见的地方），再从顶上挤进历史；
    /// 行数变多时先把历史里的行拉回来。列数的变化只做截断和补空，不重排
    pub fn resize(&mut self, columns: usize, rows: usize) {
        let (columns, rows) = (columns.max(1), rows.max(1));
        if columns == self.columns && rows == self.rows {
            return;
        }
        if columns != self.columns {
            for line in &mut self.lines {
                line.resize(columns, Cell::PLAIN);
                // 切在宽字中间：左半留不住，补成空格
                if let Some(cell) = line.last_mut()
                    && cell.width == 2
                {
                    *cell = Cell::PLAIN;
                }
            }
            self.columns = columns;
            self.reset_tabs();
        }
        while self.lines.len() > rows {
            if self.cursor.row + 1 < self.lines.len() {
                self.lines.pop_back();
            } else if let Some(top) = self.lines.pop_front() {
                if self.records_history {
                    self.archive(top);
                }
                self.cursor.row = self.cursor.row.saturating_sub(1);
            }
        }
        while self.lines.len() < rows {
            if self.records_history
                && let Some(mut line) = self.history.pop_back()
            {
                line.resize(self.columns, Cell::PLAIN);
                self.lines.push_front(line);
                self.cursor.row += 1;
            } else {
                self.lines.push_back(vec![Cell::PLAIN; self.columns]);
            }
        }
        self.rows = rows;
        self.dirty = vec![false; rows];
        self.region_top = 0;
        self.region_bottom = rows;
        self.pending_wrap = false;
        self.clamp_cursor();
        self.repaint_everything();
    }

    fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.column = self.cursor.column.min(self.columns - 1);
    }

    /// 光标移到第 `row` 行第 `column` 列（越界截到屏内），清掉待换行
    pub fn move_to(&mut self, row: usize, column: usize) {
        self.cursor.row = row.min(self.rows - 1);
        self.cursor.column = column.min(self.columns - 1);
        self.pending_wrap = false;
    }

    fn put_cell(&mut self, row: usize, column: usize, cell: Cell) {
        let Some(line) = self.lines.get_mut(row) else { return };
        if column >= line.len() {
            return;
        }
        line[column] = cell;
        self.mark(row);
    }

    /// 要在 `from..from + count` 这几格上写东西（或者从 `from` 切开）之前，先把被切到的宽字另一半清掉：
    /// 盖到右半，左边的头变空格；盖住头而右半露在外面，右半变空格。不然留下没有头的右半，
    /// 复制出来的字就少了、对不上
    fn break_wide(&mut self, row: usize, from: usize, count: usize) {
        let Some(line) = self.lines.get_mut(row) else { return };
        if from < line.len() && line[from].width == 0 && from > 0 {
            line[from - 1] = Cell::blank(line[from - 1].attrs.erased());
        }
        let end = from + count;
        if end < line.len() && line[end].width == 0 {
            line[end] = Cell::blank(line[end].attrs.erased());
        }
    }

    /// 把一行 `from..to` 擦成空格（样式只留背景色）。切到宽字就连另一半一起擦
    fn blank_range(&mut self, row: usize, from: usize, to: usize, attrs: Attributes) {
        let columns = self.columns;
        let Some(line) = self.lines.get_mut(row) else { return };
        let (mut from, mut to) = (from.min(columns), to.min(columns));
        if from >= to {
            return;
        }
        if from > 0 && line[from].width == 0 {
            from -= 1;
        }
        while to < columns && line[to].width == 0 {
            to += 1;
        }
        line[from..to].fill(Cell::blank(attrs.erased()));
        self.mark(row);
    }

    /// 写一个字。`wrap` 是自动换行开关（DECAWM）
    pub fn put(&mut self, ch: char, width: u8, attrs: Attributes, wrap: bool) {
        let width = width.clamp(1, 2) as usize;
        if wrap && self.pending_wrap {
            self.line_feed(1);
            self.cursor.column = 0;
        }
        self.pending_wrap = false;
        if self.cursor.column + width > self.columns {
            // 最后一列放不下两格的字：这一格补空格，字挪到下一行行首
            let (row, column) = (self.cursor.row, self.cursor.column);
            self.break_wide(row, column, 1);
            self.put_cell(row, column, Cell::blank(attrs.erased()));
            if !wrap {
                return;
            }
            self.line_feed(1);
            self.cursor.column = 0;
            if width > self.columns {
                return;
            }
        }
        let (row, column) = (self.cursor.row, self.cursor.column);
        self.break_wide(row, column, width);
        self.put_cell(row, column, Cell { ch, width: width as u8, attrs });
        for tail in column + 1..column + width {
            self.put_cell(row, tail, Cell { ch: ' ', width: 0, attrs });
        }
        if column + width >= self.columns {
            self.cursor.column = self.columns - 1;
            if wrap {
                self.pending_wrap = true;
            }
        } else {
            self.cursor.column += width;
        }
    }

    /// LF：光标下移一行，到区域底部就滚动
    pub fn line_feed(&mut self, n: usize) {
        for _ in 0..n.max(1) {
            if self.cursor.row + 1 == self.region_bottom {
                self.scroll_up(1);
            } else if self.cursor.row + 1 < self.rows {
                self.cursor.row += 1;
            }
        }
        self.pending_wrap = false;
    }

    /// RI（`ESC M`）：反向换行，内容整体往下走
    pub fn reverse_line_feed(&mut self, n: usize) {
        for _ in 0..n.max(1) {
            if self.cursor.row == self.region_top {
                self.scroll_down(1);
            } else {
                self.cursor.row = self.cursor.row.saturating_sub(1);
            }
        }
        self.pending_wrap = false;
    }

    /// 区域内容上移 `n` 行，底下补空行
    pub fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.region_top, self.region_bottom);
        if top >= bottom || n == 0 {
            return;
        }
        let whole = top == 0 && bottom == self.rows;
        let n = n.min(bottom - top);
        for _ in 0..n {
            let Some(mut line) = self.lines.remove(top) else { break };
            if whole && self.records_history {
                let blank = vec![Cell::PLAIN; self.columns];
                self.archive(std::mem::replace(&mut line, blank));
            } else {
                line.fill(Cell::PLAIN);
            }
            self.lines.insert(bottom - 1, line);
        }
        if whole {
            // 网格跟着 `scroll_up(n)` 就行：挪完之后每一格都和模型对得上。
            // 已经记下的脏行也跟着挪，滚出去的那几行不用画了，补进来的空行网格自己清
            self.scrolled_out += n;
            self.dirty.rotate_left(n);
            let len = self.dirty.len();
            self.dirty[len - n..].fill(false);
        } else {
            // 网格没法只挪一部分（区域外还有不动的行），区域里的行重画
            self.mark_range(top, bottom);
        }
        self.pending_wrap = false;
    }

    /// 区域内容下移 `n` 行，顶上补空行
    pub fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.region_top, self.region_bottom);
        if top >= bottom || n == 0 {
            return;
        }
        let n = n.min(bottom - top);
        for _ in 0..n {
            let Some(mut line) = self.lines.remove(bottom - 1) else { break };
            line.fill(Cell::PLAIN);
            self.lines.insert(top, line);
        }
        self.mark_range(top, bottom);
        self.pending_wrap = false;
    }

    /// 光标回本行行首
    pub fn carriage_return(&mut self) {
        self.cursor.column = 0;
        self.pending_wrap = false;
    }

    /// 退格：只把光标往左挪（终端的惯例，不越到上一行）
    pub fn backspace(&mut self, n: usize) {
        self.cursor.column = self.cursor.column.saturating_sub(n.max(1));
        self.pending_wrap = false;
    }

    /// 光标左移（不越到上一行）
    pub fn cursor_left(&mut self, n: usize) {
        self.backspace(n);
    }

    pub fn cursor_right(&mut self, n: usize) {
        self.cursor.column = (self.cursor.column + n.max(1)).min(self.columns - 1);
        self.pending_wrap = false;
    }

    /// 光标上移：在滚动区域里时停在区域顶，在区域上面时停在屏幕顶
    pub fn cursor_up(&mut self, n: usize) {
        let floor = if self.cursor.row >= self.region_top { self.region_top } else { 0 };
        self.cursor.row = self.cursor.row.saturating_sub(n.max(1)).max(floor);
        self.pending_wrap = false;
    }

    /// 光标下移：在滚动区域里时停在区域底，在区域下面时停在屏幕底
    pub fn cursor_down(&mut self, n: usize) {
        let ceiling = if self.cursor.row < self.region_bottom { self.region_bottom - 1 } else { self.rows - 1 };
        self.cursor.row = (self.cursor.row + n.max(1)).min(ceiling);
        self.pending_wrap = false;
    }

    /// 下一个制表位（HT）
    pub fn tab(&mut self) {
        let start = self.cursor.column + 1;
        let next = (start..self.columns).find(|column| self.tabs[*column]);
        self.cursor.column = next.unwrap_or(self.columns - 1);
        self.pending_wrap = false;
    }

    /// 上一个制表位（CSI Z）
    pub fn back_tab(&mut self, n: usize) {
        let mut column = self.cursor.column;
        for _ in 0..n.max(1) {
            while column > 0 {
                column -= 1;
                if self.tabs[column] {
                    break;
                }
            }
        }
        self.cursor.column = column;
        self.pending_wrap = false;
    }

    /// HTS（`ESC H`）：这一列设成制表位
    pub fn set_tab_here(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.cursor.column) {
            *tab = true;
        }
    }

    /// TBC 0（`CSI g`）：清掉这一列的制表位
    pub fn clear_tab_here(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.cursor.column) {
            *tab = false;
        }
    }

    /// TBC 3（`CSI 3 g`）：清掉全部制表位
    pub fn clear_all_tabs(&mut self) {
        self.tabs.fill(false);
    }

    /// 擦一行：0 光标到行尾，1 行首到光标（含光标），2 整行
    pub fn erase_line(&mut self, mode: u8, attrs: Attributes) {
        let (row, column, columns) = (self.cursor.row, self.cursor.column, self.columns);
        match mode {
            0 => {
                // 刚写满一行、光标还停在最后一格上：那一格的字保留，等换行
                let from = if self.pending_wrap { column + 1 } else { column };
                self.blank_range(row, from, columns, attrs);
            }
            1 => self.blank_range(row, 0, column + 1, attrs),
            _ => self.blank_range(row, 0, columns, attrs),
        }
    }

    /// 擦屏：0 光标到屏尾，1 屏头到光标，2 整屏，3 整屏加滚出去的历史
    pub fn erase_display(&mut self, mode: u8, attrs: Attributes) {
        let (row, column, columns) = (self.cursor.row, self.cursor.column, self.columns);
        match mode {
            0 => {
                let from = if self.pending_wrap { column + 1 } else { column };
                self.blank_range(row, from, columns, attrs);
                for below in row + 1..self.rows {
                    self.blank_range(below, 0, columns, attrs);
                }
            }
            1 => {
                for above in 0..row {
                    self.blank_range(above, 0, columns, attrs);
                }
                self.blank_range(row, 0, column + 1, attrs);
            }
            3 => {
                self.history.clear();
                self.erase_display(2, attrs);
            }
            _ => {
                for each in 0..self.rows {
                    self.blank_range(each, 0, columns, attrs);
                }
            }
        }
    }

    /// 擦掉 `n` 格（ECH）：光标不动
    pub fn erase_chars(&mut self, n: usize, attrs: Attributes) {
        let (row, column) = (self.cursor.row, self.cursor.column);
        self.blank_range(row, column, column + n.max(1), attrs);
    }

    /// 在光标处插 `n` 个空行（IL），区域底下的行被挤出去。光标不在滚动区域里时什么也不做
    pub fn insert_lines(&mut self, n: usize, attrs: Attributes) {
        let row = self.cursor.row;
        if row < self.region_top || row >= self.region_bottom {
            return;
        }
        let n = n.max(1).min(self.region_bottom - row);
        for _ in 0..n {
            let Some(mut line) = self.lines.remove(self.region_bottom - 1) else { break };
            line.fill(Cell::blank(attrs.erased()));
            self.lines.insert(row, line);
        }
        self.mark_range(row, self.region_bottom);
        self.cursor.column = 0;
        self.pending_wrap = false;
    }

    /// 删掉光标处 `n` 行（DL），底下补空行。光标不在滚动区域里时什么也不做
    pub fn delete_lines(&mut self, n: usize, attrs: Attributes) {
        let row = self.cursor.row;
        if row < self.region_top || row >= self.region_bottom {
            return;
        }
        let n = n.max(1).min(self.region_bottom - row);
        for _ in 0..n {
            let Some(mut line) = self.lines.remove(row) else { break };
            line.fill(Cell::blank(attrs.erased()));
            self.lines.insert(self.region_bottom - 1, line);
        }
        self.mark_range(row, self.region_bottom);
        self.cursor.column = 0;
        self.pending_wrap = false;
    }

    /// 在光标处插 `n` 个空格（ICH），行尾挤掉
    pub fn insert_blanks(&mut self, n: usize, attrs: Attributes) {
        let (row, from, columns) = (self.cursor.row, self.cursor.column, self.columns);
        let n = n.max(1).min(columns - from);
        self.break_wide(row, from, 0);
        let Some(line) = self.lines.get_mut(row) else { return };
        line[from..].rotate_right(n);
        line[from..from + n].fill(Cell::blank(attrs.erased()));
        // 挤到行尾的宽字只剩头：补成空格
        if line[columns - 1].width == 2 {
            line[columns - 1] = Cell::blank(attrs.erased());
        }
        self.mark(row);
        self.pending_wrap = false;
    }

    /// 删掉光标处 `n` 格（DCH），右边整体往左挪
    pub fn delete_chars(&mut self, n: usize, attrs: Attributes) {
        let (row, from, columns) = (self.cursor.row, self.cursor.column, self.columns);
        let n = n.max(1).min(columns - from);
        self.break_wide(row, from, n);
        let Some(line) = self.lines.get_mut(row) else { return };
        line[from..].rotate_left(n);
        line[columns - n..].fill(Cell::blank(attrs.erased()));
        self.mark(row);
        self.pending_wrap = false;
    }

    /// DECSTBM：设滚动区域（`top` 含，`bottom` 不含）。区域至少两行，设得不对就不理，光标回原点
    pub fn set_region(&mut self, top: usize, bottom: usize) {
        let (top, bottom) = (top.min(self.rows - 1), bottom.min(self.rows));
        if top + 1 < bottom {
            self.region_top = top;
            self.region_bottom = bottom;
            self.cursor = Point::new(0, 0);
            self.pending_wrap = false;
        }
    }

    pub fn reset_region(&mut self) {
        self.region_top = 0;
        self.region_bottom = self.rows;
    }

    pub fn save_cursor(&mut self, pen: Attributes) {
        self.saved = Some((self.cursor, pen));
    }

    pub fn restore_cursor(&mut self) -> Option<Attributes> {
        let (cursor, pen) = self.saved?;
        self.move_to(cursor.row, cursor.column);
        Some(pen)
    }

    /// 滚出去的历史，`from_end` 为 0 是最近滚出去的那行。行尾的默认空格已经去掉，可能比屏幕窄
    pub fn history_line(&self, from_end: usize) -> Option<&Line> {
        let index = self.history.len().checked_sub(from_end + 1)?;
        self.history.get(index)
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn archived_total(&self) -> u64 {
        self.archived_total
    }

    pub fn line_text(&self, row: usize) -> String {
        let Some(line) = self.lines.get(row) else { return String::new() };
        let text: String = line.iter().filter(|cell| cell.width != 0).map(|cell| cell.ch).collect();
        text.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_str(screen: &mut Screen, text: &str) {
        for ch in text.chars() {
            screen.put(ch, 1, Attributes::default(), true);
        }
    }

    /// 最后一列放不下两格的字时，字挪到下一行行首，原来那格补空格
    #[test]
    fn wide_char_at_last_column_wraps() {
        let mut screen = Screen::new(3, 3);
        put_str(&mut screen, "ab");
        screen.put('你', 2, Attributes::default(), true);
        assert_eq!(screen.line_text(0), "ab");
        assert_eq!(screen.line_text(1), "你");
        assert_eq!(screen.cursor, Point::new(1, 2));
    }

    /// 写到底行再换行：整屏上滚一行，顶上那行进历史
    #[test]
    fn bottom_line_feeds_history_and_reports_scroll() {
        let mut screen = Screen::new(10, 2);
        screen.records_history = true;
        screen.put('a', 1, Attributes::default(), true);
        screen.move_to(1, 0);
        screen.put('b', 1, Attributes::default(), true);
        screen.line_feed(1);
        assert_eq!(screen.take_damage().scrolled_out, 1);
        assert_eq!(screen.line_text(0), "b");
        assert_eq!(screen.history_len(), 1);
        assert_eq!(screen.history_line(0).map(|line| line.len()), Some(1), "行尾的空格不进历史");
    }

    /// 只滚区域时，只有区域里的行要重画（区域外的行没动）
    #[test]
    fn region_scroll_repaints_only_the_region() {
        let mut screen = Screen::new(10, 4);
        let _ = screen.take_damage();
        screen.set_region(1, 3);
        screen.scroll_up(1);
        let damage = screen.take_damage();
        assert_eq!(damage.scrolled_out, 0);
        assert_eq!(damage.rows, vec![1, 2]);
    }

    /// 整屏上滚时，滚之前记下的脏行要跟着挪：同一批输出里「先写再滚」的行不能画到错的位置
    #[test]
    fn dirty_rows_follow_a_whole_screen_scroll() {
        let mut screen = Screen::new(5, 3);
        let _ = screen.take_damage();
        screen.move_to(2, 0);
        put_str(&mut screen, "x");
        screen.line_feed(1);
        let damage = screen.take_damage();
        assert_eq!(damage.scrolled_out, 1);
        assert_eq!(damage.rows, vec![1], "x 现在在第 1 行");
    }

    /// 列数变窄切在宽字中间时，右半不能留下
    #[test]
    fn narrowing_columns_drops_wide_head() {
        let mut screen = Screen::new(4, 1);
        screen.move_to(0, 2);
        screen.put('你', 2, Attributes::default(), true);
        screen.resize(3, 1);
        assert_eq!(screen.lines[0][2], Cell::PLAIN);
    }

    /// 擦到行首（EL 1 / ED 1）只擦光标前面，后面的字要留着
    #[test]
    fn erase_to_start_keeps_text_after_the_cursor() {
        let mut screen = Screen::new(10, 2);
        put_str(&mut screen, "abcdef");
        screen.move_to(0, 2);
        screen.erase_line(1, Attributes::default());
        assert_eq!(screen.line_text(0), "   def");
        screen.erase_display(1, Attributes::default());
        assert_eq!(screen.line_text(0), "   def");
    }

    /// 擦出来的空格不带下划线、反色，只带背景色
    #[test]
    fn erased_cells_keep_only_the_background() {
        let mut screen = Screen::new(4, 1);
        let pen = Attributes { underline: true, inverse: true, background: Some(ColorSpec::Indexed(4)), ..Attributes::default() };
        screen.erase_line(2, pen);
        let attrs = screen.lines[0][0].attrs;
        assert_eq!(attrs.background, Some(ColorSpec::Indexed(4)));
        assert!(!attrs.underline && !attrs.inverse);
    }

    /// 在滚动区域最后一行插一行：那一行被换成空行（以前 n 被多减了 1，什么也不做）
    #[test]
    fn insert_line_on_the_last_row_of_the_region() {
        let mut screen = Screen::new(5, 3);
        for (row, text) in ["one", "two", "three"].into_iter().enumerate() {
            screen.move_to(row, 0);
            put_str(&mut screen, text);
        }
        screen.move_to(2, 0);
        screen.insert_lines(1, Attributes::default());
        assert_eq!(screen.line_text(2), "");
        assert_eq!(screen.line_text(1), "two");
    }

    /// 光标在滚动区域外面时，IL / DL 不动东西
    #[test]
    fn line_insert_outside_the_region_is_ignored() {
        let mut screen = Screen::new(5, 4);
        screen.set_region(1, 3);
        screen.move_to(0, 0);
        put_str(&mut screen, "top");
        screen.move_to(0, 0);
        screen.insert_lines(1, Attributes::default());
        screen.delete_lines(1, Attributes::default());
        assert_eq!(screen.line_text(0), "top");
    }

    /// 盖住宽字的一半时，另一半不能留成没有头的右半
    #[test]
    fn overwriting_half_a_wide_char_blanks_the_other_half() {
        let mut screen = Screen::new(6, 1);
        screen.put('你', 2, Attributes::default(), true);
        screen.move_to(0, 1);
        screen.put('x', 1, Attributes::default(), true);
        assert_eq!(screen.lines[0][0], Cell::PLAIN);
        assert_eq!(screen.line_text(0), " x");
    }

    #[test]
    fn delete_and_insert_chars_shift_the_rest_of_the_line() {
        let mut screen = Screen::new(6, 1);
        put_str(&mut screen, "abcdef");
        screen.move_to(0, 1);
        screen.delete_chars(2, Attributes::default());
        assert_eq!(screen.line_text(0), "adef");
        screen.insert_blanks(1, Attributes::default());
        assert_eq!(screen.line_text(0), "a def");
    }

    /// 缩小窗口：光标下面的空行先扔，光标所在的内容留着，不往历史里挤
    #[test]
    fn shrinking_rows_drops_empty_lines_below_the_cursor_first() {
        let mut screen = Screen::new(5, 4);
        screen.records_history = true;
        put_str(&mut screen, "$");
        screen.resize(5, 2);
        assert_eq!(screen.line_text(0), "$");
        assert_eq!(screen.history_len(), 0);
    }

    #[test]
    fn cursor_moves_respect_the_region() {
        let mut screen = Screen::new(5, 6);
        screen.set_region(2, 4);
        screen.move_to(3, 0);
        screen.cursor_up(10);
        assert_eq!(screen.cursor.row, 2, "在区域里往上走，停在区域顶");
        screen.move_to(0, 0);
        screen.cursor_down(10);
        assert_eq!(screen.cursor.row, 3, "在区域上面往下走，到区域底为止");
        screen.move_to(5, 0);
        screen.cursor_up(10);
        assert_eq!(screen.cursor.row, 2);
    }
}
