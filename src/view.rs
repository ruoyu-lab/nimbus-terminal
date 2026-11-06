//! 屏幕模型 → 字符网格。
//!
//! 框架的 `CharGrid` 只认识「第几行第几列写什么字、什么样式」，仿真层的屏幕是另一套东西，
//! 这一层负责把改动搬过去，并且尽量只搬变了的那几行：
//!
//! - 整屏上滚 → [`CharGrid::scroll_up`]：只挪行，不动图元，是快路径
//! - 单行变了 → 只重写那一行（按同样式的段一次 `write`）
//! - 区域滚动、换列数 → 整屏重画
//!
//! 另外这里放着主题的颜色表和「窗口多大放得下几列几行」的算法。

use std::sync::LazyLock;

use aureole::prelude::*;

use crate::settings::CursorStyle;
use crate::theme::{DARK_CARD, DARK_INK, LIGHT_CARD, LIGHT_INK};
use crate::vt::{Attributes, Cell, ColorSpec, CursorShape, Terminal};

/// 默认字号（逻辑像素）
pub const FONT_SIZE: f32 = 13.0;
/// 默认行高倍数：终端一般 1.2 上下
pub const LINE_HEIGHT: f32 = 1.2;
/// 还没量到真实格子大小时用的估计值（等宽字的字宽约是字号的 0.6 倍）
const ESTIMATED_CELL: (f32, f32) = (FONT_SIZE * 0.6, FONT_SIZE * LINE_HEIGHT);
/// 最少最少也得给这么多行列，免得拖窗口拖出一个 1×1 的终端
const MINIMUM: (usize, usize) = (20, 4);
const MAXIMUM: (usize, usize) = (400, 200);

/// 浅色：白底上看得清的 16 色，普通色偏深，亮色是同色相更鲜的一档（不是更浅：更浅在白底上就看不见了）
const LIGHT16: [u32; 16] = [
    0x2b3036, 0xd1383d, 0x2a8a5c, 0xa8740a, 0x2f6fd0, 0xa23fbf, 0x1b8a9a, 0x6b7279, //
    0x565c63, 0xe5484d, 0x30a46c, 0xc28a12, 0x3e63dd, 0xb84fd6, 0x12a0b3, 0x1f2328,
];

/// 深色：深底上看得清，亮色更浅更鲜
const DARK16: [u32; 16] = [
    0x4b5260, 0xf2777a, 0x7ccf8f, 0xe5c07b, 0x7aa7ff, 0xc792ea, 0x56c8d8, 0xd0d4db, //
    0x7f8797, 0xff8f92, 0x98e3a8, 0xf0d28f, 0x8dbcff, 0xdba5f5, 0x7ee0ed, 0xffffff,
];

fn rgb(value: u32) -> Color {
    Color::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// 一套终端配色：默认的底和字，加 256 色表
pub struct Colors {
    pub background: Color,
    pub foreground: Color,
    /// 响铃闪一下时的底色
    pub flash: Color,
    table: [Color; 256],
}

impl Colors {
    /// 256 色表里的第 `index` 个（0–15 是 ANSI 16 色）
    pub fn ansi(&self, index: usize) -> Color {
        self.table[index.min(255)]
    }
}

/// 256 色表，前 16 个是 `base`；16..232 是 6×6×6 的色块，232..256 是灰阶：xterm 的算法
fn build_table(base: &[u32; 16]) -> [Color; 256] {
    let mut palette = [Color::BLACK; 256];
    for (index, value) in base.iter().enumerate() {
        palette[index] = rgb(*value);
    }
    for index in 0..216 {
        let level = |shift: usize| {
            let digit = (index / shift) % 6;
            if digit == 0 { 0 } else { 55 + digit * 40 }
        };
        palette[16 + index] = Color::rgb(level(36) as u8, level(6) as u8, level(1) as u8);
    }
    for index in 0..24 {
        let gray = 8 + index * 10;
        palette[232 + index] = Color::rgb(gray as u8, gray as u8, gray as u8);
    }
    palette
}

static LIGHT: LazyLock<Colors> = LazyLock::new(|| Colors {
    background: LIGHT_CARD,
    foreground: LIGHT_INK,
    flash: Color::hex(0xe9eaec),
    table: build_table(&LIGHT16),
});

static DARK: LazyLock<Colors> = LazyLock::new(|| Colors {
    background: DARK_CARD,
    foreground: DARK_INK,
    flash: Color::hex(0x2a2f38),
    table: build_table(&DARK16),
});

/// 终端的配色：跟着界面的深浅走，不单独配
pub fn colors(dark: bool) -> &'static Colors {
    if dark { &DARK } else { &LIGHT }
}

/// 画屏幕时用的外观：配色，加默认的光标
#[derive(Clone, Copy)]
pub struct Look {
    pub colors: &'static Colors,
    pub cursor: CursorStyle,
    /// 设置允许光标闪烁（程序还可以自己关）
    pub blink: bool,
}

/// 窗口这么大的话放得下几列几行。`cell` 是一格的逻辑像素大小，没量到之前用估计值
pub fn fit(width: f32, height: f32, cell: (f32, f32)) -> (usize, usize) {
    let (cell_width, cell_height) = if cell.0 > 0.5 && cell.1 > 0.5 {
        cell
    } else {
        ESTIMATED_CELL
    };
    let columns = (width / cell_width).floor() as usize;
    let rows = (height / cell_height).floor() as usize;
    (
        columns.clamp(MINIMUM.0, MAXIMUM.0),
        rows.clamp(MINIMUM.1, MAXIMUM.1),
    )
}

/// 从网格自己报出来的边界反算一格的真实大小：框架不往外给格子度量，
/// 但元素的尺寸就是「列数 × 格宽、行数 × 格高」，除一下就拿到了
pub fn measure(bounds: Rect, columns: usize, rows: usize) -> Option<(f32, f32)> {
    if bounds.width <= 0.0 || bounds.height <= 0.0 || columns == 0 || rows == 0 {
        return None;
    }
    Some((bounds.width / columns as f32, bounds.height / rows as f32))
}

/// 回看时第 `row` 行显示哪一行：往回翻了 `offset` 行，显示的是「历史 + 屏幕」拼起来的那一长条里
/// 从 `历史长度 - offset` 开始的 `rows` 行。`offset` 为 0 就是屏幕本身
fn source_line(terminal: &Terminal, row: usize, offset: usize) -> Option<&[Cell]> {
    let screen = terminal.screen();
    let history = screen.history_len();
    let index = (history + row).checked_sub(offset)?;
    if index < history {
        // 历史里的行：`history_line(0)` 是最近滚出去的，也就是紧挨着屏幕顶的那一行
        screen
            .history_line(history - 1 - index)
            .map(|line| line.as_slice())
    } else {
        screen
            .lines
            .get(index - history)
            .map(|line| line.as_slice())
    }
}

/// 把屏幕的改动搬到网格上。
///
/// `offset` 是往回翻了多少行（0 是跟着最新的输出走），`shown_offset` 是上一次画的时候的 `offset`。
/// 平时只搬改动；翻着看时屏幕和网格的行对不上了，任何变化（翻动、新输出）都整屏重画一遍；
/// 从回看回到最新时也整屏重画，把网格还原成现在的屏幕
pub fn paint(
    terminal: &mut Terminal,
    grid: &CharGrid,
    look: Look,
    offset: usize,
    shown_offset: usize,
    full: bool,
) {
    let (columns, rows) = terminal.size();
    if grid.size() != [columns, rows] {
        grid.resize(columns, rows);
    }
    let damage = terminal.damage();
    let colors = look.colors;
    if full {
        // 配色换了：每个格子的颜色都要重算
        for row in 0..rows {
            paint_row(grid, colors, row, source_line(terminal, row, offset));
        }
    } else if offset == 0 && shown_offset == 0 {
        if damage.scrolled_out > 0 {
            grid.scroll_up(damage.scrolled_out);
        }
        for row in damage.rows {
            paint_row(
                grid,
                colors,
                row,
                terminal.screen().lines.get(row).map(|line| line.as_slice()),
            );
        }
    } else if offset != shown_offset || !damage.rows.is_empty() || damage.scrolled_out > 0 {
        for row in 0..rows {
            paint_row(grid, colors, row, source_line(terminal, row, offset));
        }
    }
    let caret = caret(terminal, look, offset);
    if grid.caret() != caret {
        grid.set_caret(caret);
    }
}

/// 重写一行：先清成默认底色，再把同样式的一段一次写进去。`line` 比网格窄（历史行去掉了行尾空格）没关系
fn paint_row(grid: &CharGrid, colors: &Colors, row: usize, line: Option<&[Cell]>) {
    grid.clear_row(row, CharCellStyle::default());
    let Some(line) = line else { return };
    let mut column = 0;
    while column < line.len() {
        let attrs = line[column].attrs;
        let start = column;
        let mut text = String::new();
        while column < line.len() && line[column].attrs == attrs {
            // 宽字的右半不带字，交给 `write` 自己按宽度占格
            if line[column].width != 0 {
                text.push(if attrs.hidden { ' ' } else { line[column].ch });
            }
            column += 1;
        }
        if !text.is_empty() {
            grid.write(row, start, &text, style(colors, attrs));
        }
    }
}

/// 一格的样式：仿真层的写法 → 网格的写法
fn style(colors: &Colors, attrs: Attributes) -> CharCellStyle {
    let mut style = CharCellStyle::default()
        .bold(attrs.bold)
        .italic(attrs.italic)
        .underline(attrs.underline)
        .strikethrough(attrs.strikethrough)
        .inverse(attrs.inverse);
    if let Some(color) = attrs.foreground {
        style = style.foreground(color_of(colors, color, attrs.bold));
    }
    if let Some(color) = attrs.background {
        // 背景色不吃「粗体变亮色」那条规则
        style = style.background(color_of(colors, color, false));
    }
    style
}

fn color_of(colors: &Colors, color: ColorSpec, bold: bool) -> Color {
    match color {
        // 粗体把前 8 色提亮到后 8 色（xterm 的 `boldAsBright`，多数 shell 的高亮文件列表靠它）
        ColorSpec::Indexed(index) if bold && index < 8 => colors.table[index as usize + 8],
        ColorSpec::Indexed(index) => colors.table[index as usize],
        ColorSpec::Rgb(value) => rgb(value),
    }
}

/// 插入点：往回翻着看的时候它在屏幕上、不在看着的那一屏里，藏起来
fn caret(terminal: &Terminal, look: Look, offset: usize) -> CharGridCaret {
    let screen = terminal.screen();
    // 程序指定了形状听程序的，没指定用设置里的
    let shape = match terminal.cursor_shape {
        Some(CursorShape::Block) => CaretShape::Block,
        Some(CursorShape::Bar) => CaretShape::Bar,
        Some(CursorShape::Underline) => CaretShape::Underline,
        None => match look.cursor {
            CursorStyle::Block => CaretShape::Block,
            CursorStyle::Underline => CaretShape::Underline,
            CursorStyle::Bar => CaretShape::Bar,
        },
    };
    let mut caret = CharGridCaret::at(screen.cursor.row, screen.cursor.column)
        .shape(shape)
        .blinking(terminal.modes.cursor_blink && look.blink);
    caret.hidden = !terminal.modes.cursor_visible || offset > 0;
    caret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look {
            colors: colors(false),
            cursor: CursorStyle::Block,
            blink: true,
        }
    }

    fn rows(grid: &CharGrid) -> Vec<String> {
        (0..grid.size()[1])
            .map(|row| grid.row_text(row).trim_end().to_string())
            .collect()
    }

    fn terminal_with_history() -> Terminal {
        // 3 行的屏幕，写 6 行：1、2、3 在历史里，4、5、6 在屏幕上
        let mut terminal = Terminal::new(10, 3);
        terminal.feed(b"1\r\n2\r\n3\r\n4\r\n5\r\n6");
        terminal
    }

    #[test]
    fn live_view_shows_the_screen() {
        let mut terminal = terminal_with_history();
        let grid = CharGrid::new(10, 3);
        paint(&mut terminal, &grid, look(), 0, 0, false);
        assert_eq!(rows(&grid), ["4", "5", "6"]);
    }

    /// 往回翻 2 行：顶上是历史的最后两行，底下是屏幕的第一行
    #[test]
    fn scrolled_back_view_joins_history_and_screen() {
        let mut terminal = terminal_with_history();
        let grid = CharGrid::new(10, 3);
        paint(&mut terminal, &grid, look(), 0, 0, false);
        paint(&mut terminal, &grid, look(), 2, 0, false);
        assert_eq!(rows(&grid), ["2", "3", "4"]);
        paint(&mut terminal, &grid, look(), 3, 2, false);
        assert_eq!(rows(&grid), ["1", "2", "3"]);
    }

    /// 回到最新：整屏还原，不能留着历史的残影
    #[test]
    fn returning_to_live_restores_the_screen() {
        let mut terminal = terminal_with_history();
        let grid = CharGrid::new(10, 3);
        paint(&mut terminal, &grid, look(), 0, 0, false);
        paint(&mut terminal, &grid, look(), 3, 0, false);
        paint(&mut terminal, &grid, look(), 0, 3, false);
        assert_eq!(rows(&grid), ["4", "5", "6"]);
    }

    /// 翻着看的时候又来了新输出：画出来的是拼起来的那条长条，不是被滚动打乱的网格
    #[test]
    fn new_output_while_scrolled_back_repaints_the_view() {
        let mut terminal = terminal_with_history();
        let grid = CharGrid::new(10, 3);
        paint(&mut terminal, &grid, look(), 0, 0, false);
        paint(&mut terminal, &grid, look(), 2, 0, false);
        terminal.feed(b"\r\n7");
        // 又滚出去一行，看着的那一屏要跟着多翻一行才不动
        paint(&mut terminal, &grid, look(), 3, 2, false);
        assert_eq!(rows(&grid), ["2", "3", "4"]);
    }

    #[test]
    fn caret_hides_while_scrolled_back() {
        let mut terminal = terminal_with_history();
        let grid = CharGrid::new(10, 3);
        paint(&mut terminal, &grid, look(), 1, 0, false);
        assert!(grid.caret().hidden);
        paint(&mut terminal, &grid, look(), 0, 1, false);
        assert!(!grid.caret().hidden);
    }
}
