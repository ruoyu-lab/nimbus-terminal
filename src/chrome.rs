//! 窗口的外观：标题栏（标签）、侧栏、主卡片、状态条、命令面板。只管画和接事件，状态在 [`Workspace`]。
//!
//! 布局（参考 Qoder 的清爽）：
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ ● ● ●  ▭  [标签] [标签] [+]                      ▥ ▤     │ 标题栏（拖动窗口）
//! ├────────────┬─────────────────────────────────────────────┤
//! │ 终端 | 监控 │ ┌─────────────────────────────────────────┐ │
//! │ ＋ 新建会话 │ │                                         │ │
//! │ 🔔 需要我   │ │               终端                       │ │
//! │ ⌕ 搜索     │ │                                         │ │
//! │ 目标       │ ├─────────────────────────────────────────┤ │
//! │  本机      │ │ 本机 · zsh                      120×36   │ │ 状态条
//! └────────────┴─┴─────────────────────────────────────────┴─┘
//! ```
//!
//! 侧栏底色和标题栏是同一块灰白，主卡片是白色的圆角卡片浮在上面，靠投影区分。
//! 整个界面全是灰阶，没有彩色。侧栏和卡片的交界处可以左右拖动改侧栏宽度。

use std::rc::Rc;

use aureole::prelude::*;
use aureole::{Appearance, capabilities, system_appearance, title_bar_info};

use crate::app;
use crate::icons::{Icon, icon};
use crate::tabs::{Mode, Workspace};
use crate::theme::{CARD_BG, INK, LINE, MARK, SIDEBAR_BG, TEXT, TEXT_2, TEXT_3, card_shadow, hover, popup_shadow, selected};

/// 标题栏的最小高度；系统给的更高就用系统的
pub const BAR_HEIGHT: f32 = 40.0;
/// 侧栏展开时的默认宽度，和能拖到的范围
pub const SIDEBAR_DEFAULT: f32 = 232.0;
const SIDEBAR_MIN: f32 = 176.0;
const SIDEBAR_MAX: f32 = 360.0;
const SIDEBAR_CLOSED: f32 = 48.0;
/// 侧栏右边缘可以按住拖动的那一条有多宽（和侧栏右内边距一样宽）
const RESIZE_HANDLE: f32 = 8.0;
const STATUS_HEIGHT: f32 = 34.0;
/// 标签：定宽，高 28，之间 2px；右边关闭按钮 20，所以上、右、下都留 4
const TAB_WIDTH: f32 = 80.0;
const TAB_HEIGHT: f32 = 28.0;
const TAB_GAP: f32 = 2.0;
const TAB_STRIDE: f32 = TAB_WIDTH + TAB_GAP;
/// 标签再多，右边也至少留这么宽的空白给拖动窗口
const DRAG_AREA_MIN: f32 = 64.0;
/// 主卡片离窗口右边、下边的空隙
const GUTTER: f32 = 8.0;
/// 打开窗口时终端区域（卡片去掉状态条）有多大，用来估第一版行列数
pub fn terminal_area() -> (f32, f32) {
    let (width, height) = crate::tabs::WINDOW_SIZE;
    (width as f32 - SIDEBAR_DEFAULT - GUTTER, height as f32 - BAR_HEIGHT - GUTTER - STATUS_HEIGHT - 1.0)
}

/// 窗口的根视图：标题栏在上，下面是侧栏和主卡片
pub fn build(workspace: Rc<Workspace>) -> Element {
    // 窗口标题跟着当前标签：任务切换、读屏软件用得上
    if let Some(window) = workspace.window {
        let (active, titles) = (workspace.active, workspace.titles);
        Effect::new(move || {
            let title = titles.get().get(&active.get()).cloned().unwrap_or_default();
            window.set_title(if title.is_empty() { "终端".to_string() } else { title });
        });
    }
    let shortcuts = workspace.clone();
    let palette_owner = workspace.clone();
    let palette_open = workspace.palette_open;
    // 外框（标题栏和侧栏）用系统的窗口材质，这里只盖一层很淡的底，保证字看得清：
    // 浅色系统里材质本身是浅的，几乎不用盖；深色系统里材质是深的，要盖一层浅色，不然深色的字看不见。
    // 平台不支持材质时退回不透明的底色
    let has_backdrop = capabilities().supports_window_backdrop;
    let frame = move || {
        if !has_backdrop {
            return SIDEBAR_BG;
        }
        if system_appearance() == Appearance::Dark { SIDEBAR_BG.with_alpha(0.84) } else { Color::WHITE.with_alpha(0.28) }
    };
    column()
        .grow(1)
        .min_width(0)
        .min_height(0)
        .font_size(13)
        .color(TEXT)
        .background(frame)
        .on_key_down_capture(move |event: &KeyEvent| {
            if shortcuts.shortcut(event) {
                event.stop_propagation();
            }
        })
        .child(title_bar(&workspace))
        .child(row().grow(1).min_height(0).child(sidebar(&workspace)).child(card(&workspace)))
        .overlay(
            palette_open,
            Overlay::new(move || palette(palette_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(INK.with_alpha(0.14))
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| palette_open.set(false)),
        )
}

// ───────────────────────── 标题栏 ─────────────────────────

fn title_bar(workspace: &Rc<Workspace>) -> Element {
    let me = workspace.window;
    let (tabs, active) = (workspace.tabs, workspace.active);
    let (toggle, new_tab, owner) = (workspace.clone(), workspace.clone(), workspace.clone());
    let scroller = ScrollHandle::new();
    // 切到的标签（新开的、点侧栏选的、快捷键切的）在视口外面时，滚到看得见为止。
    // 等一帧再滚：新标签刚挂上，视口还没量到它，现在滚会被夹回旧的末尾
    Effect::new(move || {
        let _ = (active.get(), tabs.get().len());
        aureole::spawn_local(async move {
            aureole::sleep(ms(20)).await;
            let id = active.get_untracked();
            let Some(index) = tabs.get_untracked().iter().position(|tab| tab.id == id) else { return };
            let (left, offset, shown) = (index as f32 * TAB_STRIDE, scroller.offset_x(), scroller.viewport_size()[0]);
            let right = left + TAB_WIDTH;
            if left < offset {
                scroller.scroll_to(left, 0.0);
            } else if right > offset + shown {
                scroller.scroll_to(right - shown, 0.0);
            }
        });
    });
    row()
        .window_drag_area(true) // 按住空白处拖动窗口，双击按系统设置缩放
        .height(move || px(title_bar_info().height.max(BAR_HEIGHT)))
        .align(Align::Center)
        .gap(2)
        .font_weight(600)
        // 给 macOS 的红绿灯让开（系统给的边距里有一截是空的，收掉，按钮离红绿灯近一些）；全屏时系统给 0，就只留一点边
        .child(rect().width(move || px((title_bar_info().inset_left - 8.0).max(10.0))).shrink(0))
        .child(icon_button(Icon::Sidebar, 17.0).name("toggle-sidebar").tooltip("收起或展开侧栏  ⌘B").on_click(move || toggle.toggle_sidebar()))
        // 标签定宽、离按钮 4px；放不下时在这里横向滚动（不显示滚动条，鼠标滚轮的竖向滚动也翻成横向）。
        // 新建按钮跟在标签后面，最右边永远留一段空白给拖动窗口
        .child(
            scroll_x(row().gap(TAB_GAP).align(Align::Center).each(
                move || {
                    let _ = active.get();
                    tabs.get()
                },
                |tab| tab.id,
                move |tab| appear(tab_view(&owner, tab.id)),
            ))
            .scroll_handle(scroller)
            .scrollbar_color(Color::TRANSPARENT)
            .margin_left(6)
            .min_width(0)
            .width(move || px((tabs.get().len() as f32 * TAB_STRIDE - TAB_GAP).max(0.0)))
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
        .child(icon_button(Icon::Plus, 17.0).name("new-tab").tooltip("新建会话  ⌘T").on_click(move || new_tab.new_tab()))
        .child(rect().min_width(DRAG_AREA_MIN).grow(1).shrink(0))
        // 分屏：位置先留着，之后再接
        .child(icon_button(Icon::Columns, 17.0).tooltip("左右分屏（即将支持）").color(TEXT_3))
        .child(icon_button(Icon::Rows, 17.0).tooltip("上下分屏（即将支持）").color(TEXT_3))
        // 系统不留窗口按钮的平台（Windows、Linux）自己画
        .child_when(move || title_bar_info().controls.is_none(), move || window_buttons(me))
        .child(rect().width(move || px(title_bar_info().inset_right.max(10.0))).shrink(0))
}

/// 一个标签：名字加关闭按钮。选中的是一块白色的小圆角，和下面的卡片同色
fn tab_view(workspace: &Rc<Workspace>, id: u64) -> Element {
    let (active, titles, unseen, mode) = (workspace.active, workspace.titles, workspace.unseen, workspace.mode);
    let (select, close, reorder) = (workspace.clone(), workspace.clone(), workspace.clone());
    let selected = move || active.get() == id && mode.get() == Mode::Terminal;
    // 拖动换位置：被拖的标签跟着指针走（`drag_x` 是它离自己格子的偏移），指针越过相邻标签的一半就和它换位
    let (drag_x, start_index, dragging) = (signal(0.0f32), signal(0usize), signal(false));
    row()
        .name(format!("tab-{id}"))
        .style_with(&tab_style(), &States::new().selected(selected))
        .width(TAB_WIDTH)
        .shrink(0)
        .align(Align::Center)
        .gap(4)
        .translation_x(move || drag_x.get())
        .transition(Property::TranslationX, Spring::snappy())
        .stack_order(move || if dragging.get() { 1 } else { 0 })
        .on_click(move || select.select(id))
        .on_drag(move |event: &DragEvent| match event.phase {
            GesturePhase::Start | GesturePhase::Move => {
                if event.phase == GesturePhase::Start {
                    start_index.set(reorder.index_of(id).unwrap_or(0));
                    dragging.set(true);
                    reorder.select(id);
                }
                let (start, total) = (start_index.get_untracked() as f32, event.translation[0]);
                let last = reorder.tabs.get_untracked().len().saturating_sub(1) as f32;
                let target = (start + total / TAB_STRIDE).round().clamp(0.0, last);
                reorder.move_tab(id, target as usize);
                // 标签条会裁掉自己范围外的内容：被拖的标签停在第一格和最后一格的位置，不跟着指针出去
                let offset = (total - (target - start) * TAB_STRIDE).clamp(-target * TAB_STRIDE, (last - target) * TAB_STRIDE);
                aureole::without_animation(|| drag_x.set(offset));
            }
            GesturePhase::End | GesturePhase::Cancel => {
                dragging.set(false);
                drag_x.set(0.0);
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
                .child_when(move || unseen.get().contains(&id), || rect().size(6, 6).radius(3).background(MARK).shrink(0))
                .child(text(move || titles.get().get(&id).cloned().unwrap_or_default()).grow(1).min_width(0).max_lines(1)),
        )
        .child(icon_button(Icon::Close, 12.0).name(format!("close-{id}")).size(20, 20).radius(5).on_click(move |event: &ClickEvent| {
            // 点关闭不能同时算点了标签
            event.stop_propagation();
            close.close_tab(id);
        }))
}

/// 给样式加上「状态一变就平滑地变」：悬停、选中的底色和字色淡入淡出，按下时轻轻缩一点
fn soft(style: Style) -> Style {
    style
        .transition(Property::Background, Tween::new(ms(140)).ease_out())
        .transition(Property::Color, Tween::new(ms(140)).ease_out())
        .transition(Property::Scale, Spring::snappy())
        .pressed(|s| s.scale(0.97))
}

/// 元素出现时淡入、从上面落下一点点（标签、会话新开出来时）
fn appear(element: Element) -> Element {
    let shown = signal(false);
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(180)).ease_out())
        .translation_y(move || if shown.get() { 0.0 } else { -5.0 })
        .transition(Property::TranslationY, Spring::snappy())
        .on_mount(move || shown.set(true))
}

/// 浮层出现时淡入、从 97% 放大到原样
fn pop(element: Element) -> Element {
    let shown = signal(false);
    element
        .opacity(move || if shown.get() { 1.0 } else { 0.0 })
        .transition(Property::Opacity, Tween::new(ms(140)).ease_out())
        .scale(move || if shown.get() { 1.0 } else { 0.97 })
        .transition(Property::Scale, Spring::snappy())
        .on_mount(move || shown.set(true))
        .exit(Property::Opacity, 0.0)
}

fn tab_style() -> Style {
    soft(Style::new()
        .height(TAB_HEIGHT)
        .padding_x(4)
        .radius(6)
        .font_size(12)
        .background(Color::TRANSPARENT)
        .color(TEXT_2)
        .hovered(|s| s.background(hover()))
        .selected(|s| s.background(CARD_BG).color(TEXT)))
}

/// 小图标按钮：方的、图标居中，悬停时一层淡灰
fn icon_button(glyph: Icon, size: f32) -> Element {
    row().style(&icon_style()).shrink(0).align(Align::Center).justify(Justify::Center).child(icon(glyph, size))
}

fn icon_style() -> Style {
    soft(Style::new().width(28).height(28).radius(6).color(TEXT_2).hovered(|s| s.background(hover()).color(TEXT)))
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
        .child(icon_button(Icon::Rows, 14.0).tooltip("最小化").on_click(minimize))
        .child(icon_button(Icon::Sidebar, 14.0).tooltip("最大化").window_maximize_button(true).on_click(maximize))
        .child(icon_button(Icon::Close, 14.0).tooltip("关闭").on_click(close))
}

// ───────────────────────── 侧栏 ─────────────────────────

fn sidebar(workspace: &Rc<Workspace>) -> Element {
    let (collapsed, width) = (workspace.collapsed, workspace.sidebar_width);
    let (new_tab, needs_me, search, monitor, terminal) = (workspace.clone(), workspace.clone(), workspace.clone(), workspace.clone(), workspace.clone());
    let unseen = workspace.unseen;
    // 中间那段的滚动句柄和内容的位置：切到的会话在视口外面时靠它们滚进来
    let (list, content) = (ScrollHandle::new(), signal(Rect::default()));
    column()
        .name("sidebar")
        .font_weight(600)
        .width(move || px(if collapsed.get() { SIDEBAR_CLOSED } else { width.get() }))
        .transition(Property::Width, Spring::snappy())
        .padding_x(move || if collapsed.get() { 6.0 } else { 8.0 })
        .padding_y(8)
        .gap(2)
        .min_height(0)
        // 顶上一组固定不动：展开时是一个分段开关，收起时是两个图标
        .child(
            column()
                .shrink(0)
                .child_when(move || !collapsed.get(), {
                    let (terminal, monitor) = (terminal.clone(), monitor.clone());
                    move || mode_switch(&terminal, &monitor)
                })
                .child_when(move || collapsed.get(), {
                    let (terminal, monitor) = (terminal.clone(), monitor.clone());
                    move || {
                        let (to_terminal, to_monitor) = (terminal.clone(), monitor.clone());
                        column()
                            .gap(2)
                            .child(nav(collapsed, Icon::Terminal, "终端").name("mode-terminal-icon").on_click(move || to_terminal.select(to_terminal.active.get_untracked())))
                            .child(nav(collapsed, Icon::Activity, "监控").on_click(move || to_monitor.show_monitor()))
                    }
                }),
        )
        .child(rect().height(6).shrink(0))
        // 中间一段吃掉剩下的高度，会话多了在这里竖向滚动，不去挤上下两组
        .child(
            scroll_y(
                column()
                    .gap(2)
                    .report_bounds(content)
                    .child(nav(collapsed, Icon::Plus, "新建会话").name("nav-new").hint(collapsed, "⌘T").on_click(move || new_tab.new_tab()))
                    .child(
                        nav(collapsed, Icon::Bell, "需要我")
                            .name("needs-me")
                            .tooltip("有新输出的会话")
                            .badge(collapsed, move || unseen.get().len())
                            .on_click(move || needs_me.jump_to_unseen()),
                    )
                    .child(nav(collapsed, Icon::Search, "搜索").name("nav-search").hint(collapsed, "⌘K").on_click(move || search.palette_open.set(true)))
                    .child(section_label(collapsed, "目标"))
                    .child(local_target(workspace, list, content))
                    .child(soon(collapsed, Icon::Prompt, "WSL"))
                    .child(soon(collapsed, Icon::Server, "SSH"))
                    .child(soon(collapsed, Icon::Cube, "Docker")),
            )
            .scroll_handle(list)
            .grow(1)
            .min_height(0)
            .scrollbar_color(INK.with_alpha(0.25)),
        )
        // 底下一组固定不动
        .child(
            column()
                .shrink(0)
                .gap(2)
                .child(rect().height(1).background(LINE).margin_x(6).margin_y(4))
                .child(soon(collapsed, Icon::Folder, "文件"))
                .child(soon(collapsed, Icon::Sliders, "设置")),
        )
        .child_when(move || !collapsed.get(), move || resize_handle(width))
}

/// 侧栏和卡片交界处的拖动条：贴着侧栏右边缘（正好是内容和卡片之间那条空隙），指针变成左右箭头，
/// 按住左右拖就改侧栏宽度。子元素超出父元素的部分点不到，所以它整条都在侧栏里面。按下时记下指针和侧栏边缘差多少，拖的时候保持这个差，侧栏边缘不会在按下的一瞬间跳到指针下面
fn resize_handle(width: Signal<f32>) -> Element {
    let grab = signal(None::<f32>);
    let resize = move |event: &PointerEvent| {
        if let Some(offset) = grab.get_untracked() {
            // 拖的时候侧栏要紧跟指针，不能让宽度的弹簧动画在后面追
            aureole::without_animation(|| width.set((event.window_position[0] + offset).clamp(SIDEBAR_MIN, SIDEBAR_MAX)));
        }
    };
    rect()
        .name("sidebar-resize")
        .width(RESIZE_HANDLE)
        .inset_right(0)
        .inset_top(0)
        .inset_bottom(0)
        .cursor(Cursor::ResizeHorizontal)
        .on_pointer_down(move |event: &PointerEvent| {
            event.capture_pointer();
            // 拖动条不是文字，按下也别让窗口当成要选字或者拖窗口
            event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
            grab.set(Some(width.get_untracked() - event.window_position[0]));
        })
        .on_pointer_move(resize)
        .on_pointer_up(move || grab.set(None))
        .on_pointer_cancel(move || grab.set(None))
}

/// 展开时顶上的分段开关：终端 | 监控
fn mode_switch(terminal: &Rc<Workspace>, monitor: &Rc<Workspace>) -> Element {
    let (mode, active) = (terminal.mode, terminal.active);
    let (to_terminal, to_monitor) = (terminal.clone(), monitor.clone());
    let segment = |glyph: Icon, label: &'static str, on: Mode| {
        row()
            .style_with(&segment_style(), &States::new().selected(move || mode.get() == on))
            .grow(1)
            .align(Align::Center)
            .justify(Justify::Center)
            .gap(6)
            .child(icon(glyph, 15.0))
            .child(text(label))
    };
    row()
        .padding(2)
        .gap(2)
        .radius(8)
        .background(hover())
        .child(segment(Icon::Terminal, "终端", Mode::Terminal).name("mode-terminal").on_click(move || to_terminal.select(active.get_untracked())))
        .child(segment(Icon::Activity, "监控", Mode::Monitor).name("mode-monitor").on_click(move || to_monitor.show_monitor()))
}

fn segment_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .radius(6)
            .color(TEXT_2)
            .background(Color::TRANSPARENT)
            // 分段开关的底槽本身已经是一层灰，选中的这一段用白色浮起来（和标签一致），悬停用更深一点的灰
            .hovered(|s| s.background(hover()))
            .selected(|s| s.background(CARD_BG).color(TEXT)),
    )
}

/// 侧栏里的一行：图标加文字。收起时只剩图标（居中），文字放进提示里
fn nav(collapsed: Signal<bool>, glyph: Icon, label: &'static str) -> Element {
    row()
        .style(&nav_style())
        .padding_x(move || if collapsed.get() { 0.0 } else { 10.0 })
        .align(Align::Center)
        .gap(10)
        .justify(move || if collapsed.get() { Justify::Center } else { Justify::Start })
        .tooltip(label)
        .child(icon(glyph, 17.0))
        .child_when(move || !collapsed.get(), move || text(label).grow(1).max_lines(1))
}

fn nav_style() -> Style {
    soft(
        Style::new()
            .height(32)
            .radius(6)
            .color(TEXT)
            .background(Color::TRANSPARENT)
            .hovered(|s| s.background(hover()))
            .selected(|s| s.background(selected()).color(TEXT)),
    )
}

/// 给侧栏的行加右边的小字（快捷键）、角标
trait NavExt {
    fn hint(self, collapsed: Signal<bool>, hint: &'static str) -> Element;
    fn badge(self, collapsed: Signal<bool>, count: impl Fn() -> usize + 'static) -> Element;
}

impl NavExt for Element {
    /// 快捷键提示：只在展开时显示，收起时只剩图标
    fn hint(self, collapsed: Signal<bool>, hint: &'static str) -> Element {
        self.child_when(move || !collapsed.get(), move || text(hint).font_size(11).color(TEXT_3))
    }

    /// 角标：展开时是一个带数字的小胶囊；收起时是图标右上角的一个小点
    fn badge(self, collapsed: Signal<bool>, count: impl Fn() -> usize + 'static) -> Element {
        let count = Rc::new(count);
        let (pill, label, dot) = (count.clone(), count.clone(), count);
        self.child_when(
            move || !collapsed.get() && pill() > 0,
            move || {
                let label = label.clone();
                text(move || label().to_string()).font_size(11).color(TEXT).padding_x(6).radius(5).background(selected())
            },
        )
        .child_when(move || collapsed.get() && dot() > 0, || rect().size(7, 7).radius(4).background(MARK).inset_right(8).inset_top(6))
    }
}

/// 「目标」分组的标题；收起时变成一条细线
fn section_label(collapsed: Signal<bool>, label: &'static str) -> Element {
    column()
        .child_when(move || !collapsed.get(), move || text(label).font_size(11).color(TEXT_3).padding_x(10).margin_top(12).margin_bottom(2))
        .child_when(move || collapsed.get(), || rect().height(1).background(LINE).margin_x(6).margin_y(8))
}

/// 本机：目标里现在唯一真的，展开时下面列出它的会话
fn local_target(workspace: &Rc<Workspace>, list: ScrollHandle, content: Signal<Rect>) -> Element {
    let collapsed = workspace.collapsed;
    let (tabs, active, titles, unseen, mode) = (workspace.tabs, workspace.active, workspace.titles, workspace.unseen, workspace.mode);
    let owner = workspace.clone();
    column()
        .gap(2)
        .child(nav(collapsed, Icon::Laptop, "本机").child_when(move || !collapsed.get(), || rect().size(7, 7).radius(4).background(MARK)))
        .child_when(move || !collapsed.get(), move || {
            let owner = owner.clone();
            column().gap(1).each(
                move || tabs.get(),
                |tab| tab.id,
                move |tab| {
                    let id = tab.id;
                    let select = owner.clone();
                    // 这一行成了当前会话时（新开的、点标签栏切的、快捷键切的）滚进视口。
                    // 等一帧再滚：新行刚挂上，位置还没量到
                    let bounds = signal(Rect::default());
                    Effect::new(move || {
                        if active.get() == id && mode.get() == Mode::Terminal {
                            aureole::spawn_local(async move {
                                aureole::sleep(ms(20)).await;
                                let (row, content) = (bounds.get_untracked(), content.get_untracked());
                                let (top, offset, shown) = (row.y - content.y, list.offset_y(), list.viewport_size()[1]);
                                let bottom = top + row.height;
                                if top < offset {
                                    list.scroll_to(0.0, top);
                                } else if bottom > offset + shown {
                                    list.scroll_to(0.0, bottom - shown);
                                }
                            });
                        }
                    });
                    appear(row()
                        .name(format!("session-{id}"))
                        .report_bounds(bounds)
                        .style_with(&session_style(), &States::new().selected(move || active.get() == id && mode.get() == Mode::Terminal))
                        .align(Align::Center)
                        .gap(6)
                        .on_click(move || select.select(id))
                        .child(text(move || titles.get().get(&id).cloned().unwrap_or_default()).grow(1).max_lines(1))
                        .child_when(move || unseen.get().contains(&id), || rect().size(6, 6).radius(3).background(MARK)))
                },
            )
        })
}

fn session_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .padding_x(10)
            .margin_left(18)
            .radius(6)
            .font_size(12)
            .color(TEXT_2)
            .background(Color::TRANSPARENT)
            .hovered(|s| s.background(hover()).color(TEXT))
            .selected(|s| s.background(selected()).color(TEXT)),
    )
}

/// 还没做的目标和功能：位置留着，灰着，带一个「即将」
fn soon(collapsed: Signal<bool>, glyph: Icon, label: &'static str) -> Element {
    nav(collapsed, glyph, label).color(TEXT_3).tooltip(format!("{label}（即将支持）")).child_when(
        move || !collapsed.get(),
        || text("即将").font_size(11).color(TEXT_3).padding_x(6).radius(5).background(hover()),
    )
}

// ───────────────────────── 主卡片 ─────────────────────────

fn card(workspace: &Rc<Workspace>) -> Element {
    // 卡片靠投影浮起来，不画边框；内容在圆角里裁剪，窗口缩小时被压住而不是撑出去
    column()
        .name("card")
        .grow(1)
        .min_width(0)
        .min_height(0)
        .margin_right(GUTTER)
        .margin_bottom(GUTTER)
        .radius(10)
        .background(CARD_BG)
        .shadow(card_shadow())
        .clip(true)
        .child(content(workspace))
        .child(rect().height(1).background(LINE))
        .child(status_bar(workspace))
}

/// 卡片里的内容：终端，或者监控页（占位）
fn content(workspace: &Rc<Workspace>) -> Element {
    let (tabs, active, mode) = (workspace.tabs, workspace.active, workspace.mode);
    column()
        .grow(1)
        .min_width(0)
        .min_height(0)
        // 只挂当前标签的终端；key 是标签的 id，换标签时旧的卸载、新的挂上
        .child_when(move || mode.get() == Mode::Terminal, move || {
            column().grow(1).min_width(0).min_height(0).each(
                move || tabs.get().into_iter().filter(|tab| tab.id == active.get()).collect::<Vec<_>>(),
                |tab| tab.id,
                |tab| app::view(tab.session),
            )
        })
        .child_when(move || mode.get() == Mode::Monitor, monitor_placeholder)
}

fn monitor_placeholder() -> Element {
    column()
        .grow(1)
        .align(Align::Center)
        .justify(Justify::Center)
        .gap(10)
        .color(TEXT_3)
        .child(icon(Icon::Activity, 30.0))
        .child(text("服务器监控").font_size(18).color(TEXT))
        .child(text("连上一台 SSH 目标后，这里会显示 CPU、内存、磁盘和容器。").color(TEXT_2))
}

/// 底部一排安静的小标签：现在在哪个目标、跑的什么、屏幕多大
fn status_bar(workspace: &Rc<Workspace>) -> Element {
    let (active, titles, size) = (workspace.active, workspace.titles, workspace.size);
    let chip = |glyph: Icon| row().align(Align::Center).gap(6).color(TEXT_2).font_size(12).child(icon(glyph, 14.0));
    row()
        .height(STATUS_HEIGHT)
        .padding_x(14)
        .align(Align::Center)
        .gap(18)
        .child(chip(Icon::Laptop).child(text("本机")))
        .child(chip(Icon::Terminal).child(text(move || titles.get().get(&active.get()).cloned().unwrap_or_default())))
        .child(rect().grow(1))
        .child(text(move || format!("{} × {}", size.get().0, size.get().1)).font_size(12).color(TEXT_3).tabular_numbers(true))
}

// ───────────────────────── 命令面板 ─────────────────────────

/// 面板里的一条命令
#[derive(Clone)]
struct Command {
    id: String,
    label: String,
    hint: &'static str,
    run: Rc<dyn Fn()>,
}

fn commands(workspace: &Rc<Workspace>) -> Vec<Command> {
    let command = |id: &str, label: &str, hint: &'static str, run: Rc<dyn Fn()>| Command { id: id.to_string(), label: label.to_string(), hint, run };
    let (new_tab, close, toggle, monitor) = (workspace.clone(), workspace.clone(), workspace.clone(), workspace.clone());
    let mut list = vec![
        command("cmd-new-tab", "新建会话", "⌘T", Rc::new(move || new_tab.new_tab())),
        command("cmd-close-tab", "关闭当前会话", "⌘W", Rc::new(move || close.close_active())),
        command("cmd-toggle-sidebar", "收起或展开侧栏", "⌘B", Rc::new(move || toggle.toggle_sidebar())),
        command("cmd-monitor", "前往：监控", "", Rc::new(move || monitor.show_monitor())),
    ];
    let titles = workspace.titles.get_untracked();
    for tab in workspace.tabs.get_untracked() {
        let select = workspace.clone();
        let title = titles.get(&tab.id).cloned().unwrap_or_default();
        list.push(command(&format!("cmd-session-{}", tab.id), &format!("切换到会话：{title}"), "", Rc::new(move || select.select(tab.id))));
    }
    list
}

/// 命令面板：搜索框加一列命令，打字过滤，回车执行第一条，点哪条执行哪条
fn palette(workspace: Rc<Workspace>) -> Element {
    let query = signal(String::new());
    let all = commands(&workspace);
    let filter = {
        let all = all.clone();
        move || {
            let needle = query.get().to_lowercase();
            all.iter().filter(|command| needle.is_empty() || command.label.to_lowercase().contains(&needle)).cloned().collect::<Vec<_>>()
        }
    };
    let (first, owner) = (filter.clone(), workspace.clone());
    let open = workspace.palette_open;
    pop(column()
        .name("palette")
        .width(560)
        .radius(10)
        .background(CARD_BG)
        .shadow(popup_shadow())
        .child(
            row()
                .align(Align::Center)
                .gap(10)
                .padding_x(16)
                .padding_y(10)
                .color(TEXT_2)
                .child(icon(Icon::Search, 17.0))
                .child(
                    text_field(query)
                        .placeholder("搜索命令和会话…")
                        .grow(1)
                        // 框和焦点环都去掉：整块面板就是输入区，不要再套一层框
                        .background(Color::TRANSPARENT)
                        .border(0, Color::TRANSPARENT)
                        .focus_ring(FocusRing::NONE)
                        .font_size(15)
                        .on_submit(move |_| {
                    if let Some(command) = first().first() {
                        (command.run)();
                    }
                    open.set(false);
                }),
                ),
        )
        .child(rect().height(1).background(LINE))
        .child(column().padding(6).gap(2).each(filter, |command| command.id.clone(), move |command| {
            let owner = owner.clone();
            row()
                .name(command.id.clone())
                .style(&palette_row_style())
                .align(Align::Center)
                .on_click(move || {
                    (command.run)();
                    owner.palette_open.set(false);
                })
                .child(text(command.label.clone()).grow(1).max_lines(1))
                .child(text(command.hint).font_size(11).color(TEXT_3))
        })))
}

fn palette_row_style() -> Style {
    soft(Style::new().height(34).padding_x(12).radius(6).color(TEXT).hovered(|s| s.background(hover())))
}

