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
use crate::tabs::Workspace;
use crate::theme::{card_bg, fg, hover, mark, text_2};

/// 标签：定宽，高 28，之间 2px；右边关闭按钮 20，所以上、右、下都留 4
const TAB_WIDTH: f32 = 140.0;

const TAB_HEIGHT: f32 = 28.0;

const TAB_GAP: f32 = 2.0;

const TAB_STRIDE: f32 = TAB_WIDTH + TAB_GAP;

/// 标签再多，右边也至少留这么宽的空白给拖动窗口
const DRAG_AREA_MIN: f32 = 64.0;

/// 拖动标签时的状态：被拖的是谁、从第几格起、现在指针停在第几格上。
/// 被拖的标签用框架的拖放（`DragPreview::Lift`，它自己在合成层上跟着指针走，框架每帧管着）；
/// 拖的过程中标签条的顺序不变（不重新布局），别的标签只靠平移让出位置；松手才真的换顺序
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    id: u64,
    from: usize,
    to: usize,
}

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let me = workspace.window;
    let settings_open = workspace.settings_open;
    let (toggle, strip_owner) = (workspace.clone(), workspace.clone());
    let (split_columns, split_rows) = (workspace.clone(), workspace.clone());
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
    let drag = signal(None::<Drag>);
    // 按下处离被拖的标签左边多远：算被拖的标签现在占哪一格
    let grab = Rc::new(Cell::new(0.0f32));
    let (over_grab, drop_owner) = (grab.clone(), workspace.clone());
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
            scroll_x(
                row()
                    .gap(TAB_GAP)
                    .align(Align::Center)
                    .each(
                        move || bar.get(),
                        |id| *id,
                        move |id| {
                            fade_in(with_context_menu(
                                tab_view(&owner, id, drag, grab.clone()),
                                &owner,
                                "tab",
                                id,
                            ))
                        },
                    )
                    // 标签条是放置目标：拖着标签经过时算它该落在哪一格，别的标签让位；放下时才真的换顺序
                    .accepts_drop(move |id: &u64| bar.get_untracked().contains(id))
                    .on_drag_over(move |event: &DropEvent| {
                        let Some(&id) = event.data::<u64>() else {
                            return;
                        };
                        let last = bar.get_untracked().len().saturating_sub(1) as f32;
                        let left = event.position[0] - over_grab.get();
                        let to = (left / TAB_STRIDE).round().clamp(0.0, last) as usize;
                        if let Some(dragged) = drag.get_untracked()
                            && dragged.id == id
                            && dragged.to != to
                        {
                            drag.set(Some(Drag { to, ..dragged }));
                        }
                    })
                    .on_drop(move |id: u64, _: &DropEvent| {
                        let Some(dragged) = drag.get_untracked().filter(|dragged| dragged.id == id)
                        else {
                            return;
                        };
                        // 先把被拖的设成当前标签，再清拖动状态：标签的选中样式在拖动中看的是「谁被拖」，
                        // 清掉以后看的是「谁是当前」，反过来做的话，原来的当前标签会在中间闪回选中再淡出
                        drop_owner.select(id);
                        // 换完顺序，别的标签的格子各挪了一格，让位的平移要同时归零，不然画面会跳
                        aureole::without_animation(|| {
                            drag.set(None);
                            drop_owner.move_tab(id, dragged.to);
                        });
                    })
                    // 拖出标签条：回到原来的格子，别的标签不再让位
                    .on_drag_leave(move || {
                        if let Some(dragged) = drag.get_untracked() {
                            drag.set(Some(Drag {
                                to: dragged.from,
                                ..dragged
                            }));
                        }
                    }),
            )
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
fn tab_view(
    workspace: &Rc<Workspace>,
    id: u64,
    drag: Signal<Option<Drag>>,
    grab: Rc<Cell<f32>>,
) -> Element {
    let (active, titles, unseen, bar) = (
        workspace.active,
        workspace.titles,
        workspace.unseen,
        workspace.bar,
    );
    let (select, close, reorder) = (workspace.clone(), workspace.clone(), workspace.clone());
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
    // 框架拖标签是跟着指针上下左右走的，没有「只许横向」的选项，而且它每帧都把被拖的元素放到指针下面，
    // 元素自己的平移抵消不了。所以分两层：外层是被拖的（框架拖它，它负责接拖动、点击），
    // 里面一层是看得见的标签，用反方向的平移把纵向的位移抵消掉。
    // 松手后框架用 280 毫秒的弹簧把外层的额外位移落回 0，这里用同样的弹簧把抵消的量也落回 0，两边始终互相抵消
    let cancel_y = signal(0.0f32);
    let visible = row()
        .name(format!("tab-{id}"))
        .style_with(&tab_style(), &States::new().selected(selected))
        .width(TAB_WIDTH)
        .shrink(0)
        .align(Align::Center)
        .gap(4)
        .translation_y(move || cancel_y.get())
        .transition(Property::TranslationY, Spring::new(ms(280), 1.0))
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
        );
    column()
        .width(TAB_WIDTH)
        .height(TAB_HEIGHT)
        .shrink(0)
        .translation_x(shift)
        .transition(Property::TranslationX, Spring::snappy())
        .on_click(move || select.select(id))
        .draggable(move || id)
        .drag_preview(DragPreview::Lift)
        .on_drag(move |event: &DragEvent| match event.phase {
            GesturePhase::Start => {
                grab.set(event.position[0] - event.translation[0]);
                let from = reorder.bar_index(id).unwrap_or(0);
                drag.set(Some(Drag { id, from, to: from }));
            }
            // 放下（被标签条接住）时顺序已经换好、状态已经清掉；取消（Esc、放在别处）时落回原来的格子
            GesturePhase::End | GesturePhase::Cancel => {
                if drag.get_untracked().is_some() {
                    drag.set(None);
                }
                // 带动画落回 0（和框架落回额外位移同步）
                cancel_y.set(0.0);
            }
            GesturePhase::Move => {
                // 指针每动一下都要立刻抵消，不能让弹簧在后面追；变化不到半个像素不重算
                let target = -event.translation[1];
                if (cancel_y.get_untracked() - target).abs() >= 0.5 {
                    aureole::without_animation(|| cancel_y.set(target));
                }
            }
        })
        .child(visible)
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
            .selected(|s| s.background(card_bg()).color(fg())),
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
