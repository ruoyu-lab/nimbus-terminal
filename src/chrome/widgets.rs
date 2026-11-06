//! 几个模块共用的小零件：悬停的过渡、出现和弹出的动画、图标按钮、占位页。

use std::rc::Rc;

use aureole::prelude::*;

use crate::icons::{Icon, icon};
use crate::tabs::Workspace;
use crate::theme::{card_bg, fg, hover, line, text_2};

/// 给样式加上「状态一变就平滑地变」：悬停、选中的底色和字色淡入淡出，按下时轻轻缩一点
pub(super) fn soft(style: Style) -> Style {
    style
        .transition(Property::Background, Tween::new(ms(140)).ease_out())
        .transition(Property::Color, Tween::new(ms(140)).ease_out())
        .transition(Property::Scale, Spring::snappy())
        .pressed(|s| s.scale(0.97))
}

/// 元素出现时淡入、从上面落下一点点（会话新开出来时）。换深浅色重建界面时挂上的不算新出现的，直接就位
pub(super) fn appear(element: Element) -> Element {
    let shown = signal(crate::theme::just_switched());
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(180)).ease_out())
        .translation_y(move || if shown.get() { 0.0 } else { -5.0 })
        .transition(Property::TranslationY, Spring::snappy())
        .on_mount(move || shown.set(true))
}

/// 元素出现时只淡入（不位移）：标签条的视口和标签一样高，往上落下来的那几个像素会被它裁掉
pub(super) fn fade_in(element: Element) -> Element {
    let shown = signal(crate::theme::just_switched());
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(180)).ease_out())
        .on_mount(move || shown.set(true))
}

/// 叠在同一块地方、互相替换的面板：`side` 是它待的那一侧（负数在左、正数在右）。
/// 换进来时从那一侧滑过来并淡入，被换走时往那一侧滑出去并淡出，两块面板在交接时是叠着的
pub(super) fn swap(element: Element, side: f32) -> Element {
    let shown = signal(crate::theme::just_switched());
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(200)).ease_out())
        .translation_x(move || if shown.get() { 0.0 } else { side })
        .transition(Property::TranslationX, Spring::snappy())
        .on_mount(move || shown.set(true))
        .exit_with(Property::Opacity, 0.0, Tween::new(ms(120)).ease_out())
        .exit_with(Property::TranslationX, side, Tween::new(ms(200)).ease_out())
}

/// 浮层出现时淡入、从 97% 放大到原样
pub(super) fn pop(element: Element) -> Element {
    let shown = signal(false);
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(140)).ease_out())
        .scale(move || if shown.get() { 1.0 } else { 0.97 })
        .transition(Property::Scale, Spring::snappy())
        .on_mount(move || shown.set(true))
        .exit(Property::Opacity, 0.0)
}

/// 小图标按钮：方的、图标居中，悬停时一层淡灰
pub(super) fn icon_button(glyph: Icon, size: f32) -> Element {
    row()
        .style(&icon_style())
        .shrink(0)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(icon(glyph, size))
}

fn icon_style() -> Style {
    soft(
        Style::new()
            .width(28)
            .height(28)
            .radius(6)
            .color(text_2())
            .hovered(|s| s.background(hover()).color(fg())),
    )
}

/// 带细边框的按钮：图标（可以没有）加文字，悬停时一层淡灰
pub(super) fn outline_button(glyph: Option<Icon>, label: &'static str) -> Element {
    let mut button = row()
        .style(&outline_style())
        .align(Align::Center)
        .justify(Justify::Center)
        .gap(8)
        .focusable(true)
        .focus_ring(FocusRing::new(crate::theme::ink()))
        .accessibility_name(label);
    if let Some(glyph) = glyph {
        button = button.child(icon(glyph, 15.0));
    }
    button.child(text(label))
}

fn outline_style() -> Style {
    soft(
        Style::new()
            .height(32)
            .padding_x(14)
            .radius(7)
            .font_size(13)
            .color(fg())
            .background(card_bg())
            .border(1, line())
            .hovered(|s| s.background(hover())),
    )
}

/// 让元素能接从访达拖来的目录：拖着文件在上面时 `over` 为真（用来高亮），放下就把里面的目录加进「目标」
/// （文件和不存在的路径忽略）
pub(super) fn directory_drop(
    element: Element,
    workspace: &Rc<Workspace>,
    over: Signal<bool>,
) -> Element {
    let workspace = workspace.clone();
    element
        .on_file_hover(move |hovering: bool| over.set(hovering))
        .on_file_drop(move |event: &FileDropEvent| {
            over.set(false);
            for path in &event.paths {
                workspace.add_directory(path);
            }
        })
}
