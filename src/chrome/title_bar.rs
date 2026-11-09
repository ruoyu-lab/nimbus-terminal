//! 标题栏：窗口按钮、侧栏开关、标签条（拖动换位置）和右上角的布局按钮。
//! 设置页开着时标签条、新建按钮和分屏按钮都收起，只留侧栏开关和「设置」两个字。

use std::cell::Cell;
use std::rc::Rc;

use aureole::prelude::*;
use aureole::title_bar_info;

use crate::chrome::BAR_HEIGHT;
use crate::chrome::menu::with_context_menu;
use crate::chrome::widgets::{fade_in, icon_button, soft};
use crate::icons::Icon;
use crate::pane::Axis;
use crate::tabs::{TabDrag, Workspace};
use crate::theme::{fg, hover, mark, tab_selected_bg, text_2};

/// 标签：定宽，高 28，之间 2px；右边关闭按钮 20，所以上、右、下都留 4
const TAB_WIDTH: f32 = 140.0;

const TAB_HEIGHT: f32 = 28.0;

const TAB_GAP: f32 = 2.0;

const TAB_STRIDE: f32 = TAB_WIDTH + TAB_GAP;

/// 标签再多，右边也至少留这么宽的空白给拖动窗口
const DRAG_AREA_MIN: f32 = 64.0;

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let me = workspace.window;
    let settings_open = workspace.settings_open;
    let (toggle, strip_owner) = (workspace.clone(), workspace.clone());
    let (split_columns, split_rows) = (workspace.clone(), workspace.clone());
    let (panel_toggle, panel_available) = (workspace.clone(), workspace.clone());
    row()
        .window_drag_area(true) // 按住空白处拖动窗口，双击按系统设置缩放
        .height(move || px(title_bar_info().height.max(BAR_HEIGHT)))
        .align(Align::Center)
        .gap(2)
        .font_weight(600)
        // 给 macOS 的红绿灯让开（系统给的边距里有一截是空的，收掉，按钮离红绿灯近一些）；全屏时系统给 0，就只留一点边
        .child(
            rect()
                .width(move || px((title_bar_info().inset_left - 8.0).max(10.0)))
                .shrink(0),
        )
        .child(
            icon_button(Icon::Sidebar, 17.0)
                .name("toggle-sidebar")
                .tooltip("收起或展开侧栏  ⌘B")
                .on_click(move || toggle.toggle_sidebar()),
        )
        // 设置页里没有标签可言：标签条和新建按钮换成一个标题
        .child_when(
            move || !settings_open.get(),
            move || tab_strip(&strip_owner),
        )
        .child_when(
            move || settings_open.get(),
            || {
                text("设置")
                    .name("settings-title")
                    .font_size(13)
                    .margin_left(6)
                    .color(fg())
            },
        )
        .child(rect().min_width(DRAG_AREA_MIN).grow(1).shrink(0))
        // 分屏：把聚焦的终端拆成左右或上下两个；设置页里不显示
        .child_when(
            move || !settings_open.get(),
            move || {
                let (columns, rows) = (split_columns.clone(), split_rows.clone());
                row()
                    .gap(2)
                    .shrink(0)
                    .child(
                        icon_button(Icon::Columns, 17.0)
                            .name("split-columns")
                            .tooltip("左右分屏  ⌘D")
                            .on_click(move || columns.split(Axis::Columns)),
                    )
                    .child(
                        icon_button(Icon::Rows, 17.0)
                            .name("split-rows")
                            .tooltip("上下分屏  ⌘⇧D")
                            .on_click(move || rows.split(Axis::Rows)),
                    )
            },
        )
        // 右侧面板的开关：只有当前会话的目标带面板（连到服务器）时才出现，本机永远没有
        .child_when(
            move || !settings_open.get() && panel_available.has_panels(),
            move || {
                let toggle = panel_toggle.clone();
                icon_button(Icon::PanelRight, 17.0)
                    .name("toggle-panel")
                    .tooltip("显示或隐藏右侧面板")
                    .on_click(move || toggle.toggle_panel())
            },
        )
        // 系统不留窗口按钮的平台（Windows、Linux）自己画
        .child_when(
            move || title_bar_info().controls.is_none(),
            move || window_buttons(me),
        )
        .child(
            rect()
                .width(move || px(title_bar_info().inset_right.max(10.0)))
                .shrink(0),
        )
}

/// 标签条加后面的新建按钮
fn tab_strip(workspace: &Rc<Workspace>) -> Element {
    let (bar, active) = (workspace.bar, workspace.active);
    let (new_tab, owner) = (workspace.clone(), workspace.clone());
    let scroller = ScrollHandle::new();
    // 切到的标签（新开的、点侧栏选的、快捷键切的、刚从设置页回来）在视口外面时，滚到看得见为止。
    // 等一帧再滚：新标签刚挂上，视口还没量到它，现在滚会被夹回旧的末尾
    Effect::new(move || {
        let _ = (active.get(), bar.get().len());
        aureole::spawn_local(async move {
            aureole::sleep(ms(20)).await;
            let id = active.get_untracked();
            let Some(index) = bar.get_untracked().iter().position(|&shown| shown == id) else {
                return;
            };
            let (left, offset, shown) = (
                index as f32 * TAB_STRIDE,
                scroller.offset_x(),
                scroller.viewport_size()[0],
            );
            let right = left + TAB_WIDTH;
            if left < offset {
                scroller.scroll_to(left, 0.0);
            } else if right > offset + shown {
                scroller.scroll_to(right - shown, 0.0);
            }
        });
    });
    // 标签定宽、离按钮 4px；放不下时在这里横向滚动（不显示滚动条，鼠标滚轮的竖向滚动也翻成横向）。
    // 新建按钮跟在标签后面，最右边永远留一段空白给拖动窗口
    row()
        .align(Align::Center)
        .gap(2)
        .min_width(0)
        .child(
            scroll_x(row().gap(TAB_GAP).align(Align::Center).each(
                move || bar.get(),
                |id| *id,
                move |id| {
                    fade_in(with_context_menu(
                        tab_view(&owner, id, scroller),
                        &owner,
                        "tab",
                        id,
                    ))
                },
            ))
            .scroll_handle(scroller)
            .scrollbar_color(Color::TRANSPARENT)
            .margin_left(6)
            .min_width(0)
            .width(move || px((bar.get().len() as f32 * TAB_STRIDE - TAB_GAP).max(0.0)))
            .on_wheel(move |event: &WheelEvent| {
                let (horizontal, vertical) = match event.delta {
                    WheelDelta::Lines(delta) => (delta[0] * 40.0, delta[1] * 40.0),
                    WheelDelta::Pixels(delta) => (delta[0], delta[1]),
                };
                if vertical.abs() > horizontal.abs() {
                    scroller.scroll_by(vertical, 0.0);
                    event.stop_propagation();
                }
            }),
        )
        .child(
            icon_button(Icon::Plus, 17.0)
                .name("new-tab")
                .tooltip("新建会话  ⌘T")
                .on_click(move || new_tab.new_tab()),
        )
}

/// 一个标签：名字加关闭按钮。选中的是一块白色的小圆角，和下面的卡片同色
fn tab_view(workspace: &Rc<Workspace>, id: u64, scroller: ScrollHandle) -> Element {
    let drag = workspace.tab_drag;
    let (active, titles, unseen, bar) = (
        workspace.active,
        workspace.titles,
        workspace.unseen,
        workspace.bar,
    );
    let (select, close, reorder) = (workspace.clone(), workspace.clone(), workspace.clone());
    // 开始拖的时候标签条滚到哪了
    let start_scroll = Rc::new(Cell::new(0.0f32));
    // 拖着别的标签时，只有被拖的这个是白底（松手后它就是当前标签）
    let selected = move || match drag.get() {
        Some(dragged) => dragged.id == id,
        None => active.get() == id,
    };
    // 拖的时候这个标签该往哪边挪一格：被拖的从左往右越过它，它往左让；从右往左越过，它往右让。
    // 被拖的这一个自己不挪：它在合成层上被框架拖着，松手后框架从指针处把它落进新的格子，
    // 它自己再有平移，两边一起算就会在松手时多跳一下
    let shift = move || -> f32 {
        let Some(dragged) = drag.get() else {
            return 0.0;
        };
        if dragged.id == id {
            return 0.0;
        }
        let Some(index) = bar.get().iter().position(|&shown| shown == id) else {
            return 0.0;
        };
        if dragged.from < dragged.to && index > dragged.from && index <= dragged.to {
            -TAB_STRIDE
        } else if dragged.to < dragged.from && index >= dragged.to && index < dragged.from {
            TAB_STRIDE
        } else {
            0.0
        }
    };
    row()
        .name(format!("tab-{id}"))
        .style_with(&tab_style(), &States::new().selected(selected))
        .width(TAB_WIDTH)
        .shrink(0)
        .align(Align::Center)
        .gap(4)
        .translation_x(shift)
        .transition(Property::TranslationX, Spring::snappy())
        .on_click(move || select.select(id))
        .draggable(move || id)
        // 被拖的标签由框架在合成层上跟着指针走：只沿横向、不出标签条
        .drag_preview(DragPreview::Lift)
        .drag_preview_axis(Direction::Row)
        .drag_preview_contained(true)
        .on_drag(move |event: &DragEvent| match event.phase {
            GesturePhase::Start => {
                let from = reorder.bar_index(id).unwrap_or(0);
                start_scroll.set(scroller.offset_x());
                drag.set(Some(TabDrag { id, from, to: from }));
            }
            // 放下（落在窗口里的哪儿都算）时顺序已经换好、状态已经清掉；
            // 取消（Esc、拖出了窗口）时落回原来的格子
            GesturePhase::End | GesturePhase::Cancel => {
                if drag.get_untracked().is_some() {
                    drag.set(None);
                }
            }
            GesturePhase::Move => {
                let Some(dragged) = drag.get_untracked() else {
                    return;
                };
                // 被拖的标签现在的左边：起点的格子加上指针走过的横向距离（拖到边上标签条自己滚动了也算进去），
                // 夹在第一格和最后一格之间（它不出标签条）。左边每过半格就换一格——也就是
                // 它的边越过相邻标签的一半时换，和指针在哪、从标签的哪里抓起来都没有关系
                let last = reorder.bar.get_untracked().len().saturating_sub(1) as f32;
                let left =
                    (dragged.from as f32 * TAB_STRIDE + event.translation[0] + scroller.offset_x()
                        - start_scroll.get())
                    .clamp(0.0, last * TAB_STRIDE);
                let to = (left / TAB_STRIDE).round() as usize;
                if dragged.to != to {
                    drag.set(Some(TabDrag { to, ..dragged }));
                }
            }
        })
        // 左边留 4 + 4，关闭按钮上、右、下都是 4：标签高 28、按钮 20
        .child(
            row()
                .grow(1)
                .min_width(0)
                .margin_left(4)
                .align(Align::Center)
                .gap(4)
                // 后台有新输出：标题前一个深灰的小点
                .child_when(
                    move || unseen.get().contains(&id),
                    || rect().size(6, 6).radius(3).background(mark()).shrink(0),
                )
                .child(
                    text(move || titles.get().get(&id).cloned().unwrap_or_default())
                        .grow(1)
                        .min_width(0)
                        .max_lines(1),
                ),
        )
        .child(
            icon_button(Icon::Close, 12.0)
                .name(format!("close-{id}"))
                .size(20, 20)
                .radius(5)
                .on_click(move |event: &ClickEvent| {
                    // 点关闭不能同时算点了标签
                    event.stop_propagation();
                    close.request_close(id);
                }),
        )
}

fn tab_style() -> Style {
    soft(
        Style::new()
            .height(TAB_HEIGHT)
            .padding_x(4)
            .radius(6)
            .font_size(12)
            .background(Color::TRANSPARENT)
            .color(text_2())
            .hovered(|s| s.background(hover()))
            .selected(|s| s.background(tab_selected_bg()).color(fg())),
    )
}

/// Windows、Linux 上自己画的窗口按钮
fn window_buttons(window: Option<WindowHandle>) -> Element {
    let minimize = move || {
        if let Some(window) = window {
            window.set_minimized(true);
        }
    };
    let maximize = move || {
        if let Some(window) = window {
            window.set_maximized(!window.is_maximized());
        }
    };
    let close = move || {
        if let Some(window) = window {
            window.close();
        }
    };
    row()
        .gap(2)
        .child(
            icon_button(Icon::Rows, 14.0)
                .tooltip("最小化")
                .on_click(minimize),
        )
        .child(
            icon_button(Icon::Sidebar, 14.0)
                .tooltip("最大化")
                .window_maximize_button(true)
                .on_click(maximize),
        )
        .child(
            icon_button(Icon::Close, 14.0)
                .tooltip("关闭")
                .on_click(close),
        )
}
