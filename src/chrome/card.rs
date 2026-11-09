//! 主卡片：设置页、终端或没有标签时的引导，加底部的状态条。

use std::rc::Rc;

use aureole::prelude::*;

use crate::app;
use crate::chrome::menu::pane_menu_overlay;
use crate::chrome::settings;
use crate::chrome::widgets::{directory_drop, outline_button};
use crate::chrome::{GUTTER, STATUS_HEIGHT};
use crate::icons::{Icon, icon};
use crate::pane::{Axis, Pane};
use crate::tabs::{Tab, Workspace};
use crate::target::Target;
use crate::theme::{card_bg, card_shadow, fg, hover, line, text_2, text_3};

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let (active, tabs, settings_open, form) = (
        workspace.active,
        workspace.tabs,
        workspace.settings_open,
        workspace.form,
    );
    let status_owner = workspace.clone();
    let panel_shown = {
        let workspace = workspace.clone();
        move || workspace.panel_shown()
    };
    // 状态条说的是会话（在哪个目标、跑什么、多大）：设置页里没有这些，设置里关掉了也不显示
    let status_shown =
        move || !settings_open.get() && session_open(tabs, active) && form.status_bar.get();
    // 卡片靠投影浮起来，不画边框；内容在圆角里裁剪，窗口缩小时被压住而不是撑出去
    column()
        .name("card")
        .grow(1)
        .min_width(0)
        .min_height(0)
        // 右边有面板时，两张卡片之间的空隙是面板左边那条拖动条，不再另留
        .margin_right(move || if panel_shown() { 0.0 } else { GUTTER })
        .margin_bottom(GUTTER)
        .radius(10)
        .background(card_bg())
        .shadow(card_shadow())
        .clip(true)
        .child(content(workspace))
        .child_when(status_shown, || rect().height(1).background(line()))
        .child_when(status_shown, move || status_bar(&status_owner))
}

/// 卡片里的内容：设置页，或者终端，或者没有标签时的引导
fn content(workspace: &Rc<Workspace>) -> Element {
    let (tabs, active, settings_open) = (workspace.tabs, workspace.active, workspace.settings_open);
    let (tabs_list, active_list) = (tabs, active);
    let settings_owner = workspace.clone();
    let empty_owner = workspace.clone();
    let panes_owner = workspace.clone();
    column()
        .grow(1)
        .min_width(0)
        .min_height(0)
        .child_when(
            move || settings_open.get(),
            move || settings::view(&settings_owner),
        )
        .child_when(
            move || !settings_open.get() && !session_open(tabs, active),
            move || empty_state(&empty_owner),
        )
        // 只挂当前标签的终端；key 是标签的 id 加上它里面终端排布的结构：
        // 换标签时旧的卸载、新的挂上，拆分、关掉一个终端时这个标签的排布重建（拖分隔条不重建）
        .child_when(
            move || !settings_open.get() && session_open(tabs_list, active_list),
            move || {
                let panes_owner = panes_owner.clone();
                column().grow(1).min_width(0).min_height(0).each(
                    move || {
                        tabs_list
                            .get()
                            .into_iter()
                            .filter(|tab| tab.id == active_list.get())
                            .map(|tab| {
                                (format!("{}:{}", tab.id, tab.layout.get().signature()), tab)
                            })
                            .collect::<Vec<_>>()
                    },
                    |(key, _)| key.clone(),
                    move |(_, tab)| {
                        let layout = tab.layout.get_untracked();
                        let multi = layout.leaves().len() > 1;
                        pane_node(&panes_owner, &tab, &layout, multi)
                    },
                )
            },
        )
}

/// 排布里的一个节点：一个终端，或者一次拆分（两边加中间的分隔条）
fn pane_node(workspace: &Rc<Workspace>, tab: &Tab, pane: &Pane, multi: bool) -> Element {
    match pane {
        Pane::Leaf { id, session } => pane_leaf(workspace, tab, *id, session.clone(), multi),
        Pane::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let ratio = *ratio;
            let bounds = signal(Rect::default());
            // 按比例分：两边的初始尺寸都是 0，剩下的空间按 ratio : 1 - ratio 分
            let sized = |element: Element, share: fn(f32) -> f32| {
                element
                    .grow(move || share(ratio.get()))
                    .basis(0)
                    .min_width(0)
                    .min_height(0)
            };
            let first = sized(pane_node(workspace, tab, first, multi), |ratio| ratio);
            let second = sized(pane_node(workspace, tab, second, multi), |ratio| {
                1.0 - ratio
            });
            let container = match axis {
                Axis::Columns => row(),
                Axis::Rows => column(),
            };
            container
                .grow(1)
                .min_width(0)
                .min_height(0)
                .report_bounds(bounds)
                .child(first)
                .child(divider(*axis, ratio, bounds))
                .child(second)
        }
    }
}

/// 一个终端。拆分过时聚焦的那个有一圈深一点的边，点哪个就聚焦哪个
fn pane_leaf(
    workspace: &Rc<Workspace>,
    tab: &Tab,
    pane: u64,
    session: Rc<crate::app::Session>,
    multi: bool,
) -> Element {
    let (focus, owner, tab_id) = (tab.focus, workspace.clone(), tab.id);
    let focused = move || focus.get() == pane;
    // 右键菜单：点在哪、开没开
    let (open, at) = (signal(false), signal([0.0f32, 0.0]));
    let leaf = column()
        .name(format!("pane-{pane}"))
        .grow(1)
        .min_width(0)
        .min_height(0)
        .clip(true)
        .on_pointer_down(move |_: &PointerEvent| owner.focus_pane(tab_id, pane))
        // 拆分过时，聚焦的那个终端顶上一条深一点的线，别的是透明的
        .child_when(
            move || multi,
            move || {
                rect()
                    .name(format!("pane-focus-{pane}"))
                    .height(2)
                    .shrink(0)
                    .background(move || {
                        if focused() {
                            text_2()
                        } else {
                            Color::TRANSPARENT
                        }
                    })
            },
        )
        .child(app::view(session, focused, move |position| {
            // 按下的时候这个终端已经聚焦了（见上面的 `on_pointer_down`），菜单里的动作落在它身上
            at.set(position);
            open.set(true);
        }));
    pane_menu_overlay(leaf, open, at, workspace, tab_id, pane)
}

/// 两个终端之间的分隔条：一条细线，两边各留几像素好抓；按住左右（或上下）拖就改比例
fn divider(axis: Axis, ratio: Signal<f32>, bounds: Signal<Rect>) -> Element {
    const GRAB: f32 = 7.0;
    let grab = signal(None::<(f32, f32)>);
    let along = move |event: &PointerEvent| match axis {
        Axis::Columns => event.window_position[0],
        Axis::Rows => event.window_position[1],
    };
    let drag = move |event: &PointerEvent| {
        let Some((start, ratio_then)) = grab.get_untracked() else {
            return;
        };
        let total = match axis {
            Axis::Columns => bounds.get_untracked().width,
            Axis::Rows => bounds.get_untracked().height,
        } - GRAB;
        if total > 0.0 {
            let next = ratio_then + (along(event) - start) / total;
            aureole::without_animation(|| {
                ratio.set(next.clamp(crate::pane::RATIO_RANGE.0, crate::pane::RATIO_RANGE.1));
            });
        }
    };
    let (handle, cursor) = match axis {
        Axis::Columns => (
            column()
                .width(GRAB)
                .align(Align::Center)
                .child(rect().width(1).grow(1).background(line())),
            Cursor::ResizeHorizontal,
        ),
        Axis::Rows => (
            row()
                .height(GRAB)
                .align(Align::Center)
                .child(rect().height(1).grow(1).background(line())),
            Cursor::ResizeVertical,
        ),
    };
    handle
        .name(match axis {
            Axis::Columns => "pane-divider-columns",
            Axis::Rows => "pane-divider-rows",
        })
        .shrink(0)
        .cursor(cursor)
        .on_pointer_down(move |event: &PointerEvent| {
            event.capture_pointer();
            // 分隔条不是文字，按下也别让窗口当成要选字或者拖窗口
            event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
            grab.set(Some((along(event), ratio.get_untracked())));
        })
        .on_pointer_move(drag)
        .on_pointer_up(move || grab.set(None))
        .on_pointer_cancel(move || grab.set(None))
}

fn session_open(tabs: Signal<Vec<Tab>>, active: Signal<u64>) -> bool {
    tabs.get().iter().any(|tab| tab.id == active.get())
}

/// 没有标签：写明白，下面给一个选目录的按钮。选好的目录进「目标」，并在里面开一个终端。
/// 把文件夹拖到这里也一样会加进「目标」
fn empty_state(workspace: &Rc<Workspace>) -> Element {
    let choose = workspace.clone();
    let over = signal(false);
    let state = column()
        .name("empty-state")
        .grow(1)
        .align(Align::Center)
        .justify(Justify::Center)
        .gap(10)
        .color(text_3())
        .background(move || {
            if over.get() {
                hover()
            } else {
                Color::TRANSPARENT
            }
        })
        .child(icon(Icon::Terminal, 30.0))
        .child(text("没有标签").name("no-tabs").font_size(18).color(fg()))
        .child(
            text("选一个目录开始，也可以把文件夹拖到左边的「目标」里。")
                .color(text_2())
                .margin_bottom(6),
        )
        .child(
            outline_button(Some(Icon::Folder), "选择目录")
                .name("choose-directory")
                .on_click(move || choose.choose_directory()),
        );
    directory_drop(state, workspace, over)
}

/// 底部一排安静的小标签：现在在哪个目标、跑的什么、屏幕多大
fn status_bar(workspace: &Rc<Workspace>) -> Element {
    let (active, titles, size, tabs) = (
        workspace.active,
        workspace.titles,
        workspace.size,
        workspace.tabs,
    );
    let chip = |glyph: Icon| {
        row()
            .align(Align::Center)
            .gap(6)
            .color(text_2())
            .font_size(12)
            .child(icon(glyph, 14.0))
    };
    let target = move || {
        tabs.get()
            .into_iter()
            .find(|tab| tab.id == active.get())
            .map_or(Target::Local, |tab| tab.target)
    };
    row()
        .height(STATUS_HEIGHT)
        .padding_x(14)
        .align(Align::Center)
        .gap(18)
        .child(
            row()
                .align(Align::Center)
                .gap(6)
                .color(text_2())
                .font_size(12)
                // 本机是笔记本，服务器是机柜
                .child_when(move || target().host().is_none(), || icon(Icon::Laptop, 14.0))
                .child_when(move || target().host().is_some(), || icon(Icon::Server, 14.0))
                .child(text(move || target().label()).name("status-target")),
        )
        .child(chip(Icon::Terminal).child(text(move || {
            titles.get().get(&active.get()).cloned().unwrap_or_default()
        })))
        .child(rect().grow(1))
        .child(
            text(move || format!("{} × {}", size.get().0, size.get().1))
                .font_size(12)
                .color(text_3())
                .tabular_numbers(true),
        )
}
