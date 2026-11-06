//! 侧栏里一行一行的零件：图标加文字、快捷键提示、角标、分组标题。

use aureole::prelude::*;

use crate::chrome::widgets::soft;
use crate::icons::{Icon, icon};
use crate::theme::{fg, hover, line, selected, text_3};

/// 侧栏里的一行：图标加文字。收起时只剩图标（居中），文字放进提示里
pub(super) fn nav(collapsed: Signal<bool>, glyph: Icon, label: &'static str) -> Element {
    nav_row(collapsed, glyph, label).style(&nav_style())
}

/// 带选中态的侧栏行（设置的分类）
pub(super) fn nav_selected(
    collapsed: Signal<bool>,
    glyph: Icon,
    label: &'static str,
    is_selected: impl Fn() -> bool + 'static,
) -> Element {
    nav_row(collapsed, glyph, label).style_with(&nav_style(), &States::new().selected(is_selected))
}

fn nav_row(collapsed: Signal<bool>, glyph: Icon, label: &'static str) -> Element {
    row()
        .padding_x(move || if collapsed.get() { 0.0 } else { 10.0 })
        .align(Align::Center)
        .gap(10)
        .justify(move || {
            if collapsed.get() {
                Justify::Center
            } else {
                Justify::Start
            }
        })
        .tooltip(label)
        .child(icon(glyph, 17.0))
        .child_when(
            move || !collapsed.get(),
            move || text(label).grow(1).max_lines(1),
        )
}

fn nav_style() -> Style {
    soft(
        Style::new()
            .height(32)
            .radius(6)
            .color(fg())
            .background(Color::TRANSPARENT)
            .hovered(|s| s.background(hover()))
            .selected(|s| s.background(selected()).color(fg())),
    )
}

/// 给侧栏的行加右边的小字（快捷键）
pub(super) trait NavExt {
    fn hint(self, collapsed: Signal<bool>, hint: &'static str) -> Element;
}

impl NavExt for Element {
    /// 快捷键提示：只在展开时显示，收起时只剩图标
    fn hint(self, collapsed: Signal<bool>, hint: &'static str) -> Element {
        self.child_when(
            move || !collapsed.get(),
            move || text(hint).font_size(11).color(text_3()),
        )
    }
}

/// 「目标」分组的标题；收起时变成一条细线
pub(super) fn section_label(collapsed: Signal<bool>, label: &'static str) -> Element {
    column()
        .child_when(
            move || !collapsed.get(),
            move || {
                text(label)
                    .font_size(11)
                    .color(text_3())
                    .padding_x(10)
                    .margin_top(12)
                    .margin_bottom(2)
            },
        )
        .child_when(
            move || collapsed.get(),
            || rect().height(1).background(line()).margin_x(6).margin_y(8),
        )
}
