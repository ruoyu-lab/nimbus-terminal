//! 侧栏：外壳（宽度、收起、拖动改宽度）。里面放什么由 `main_nav`（平时）和 `settings_nav`（设置打开时：返回加分类）决定。

mod main_nav;
mod nav;
mod settings_nav;
mod targets;

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::widgets::swap;
use crate::chrome::{SIDEBAR_MAX, SIDEBAR_MIN};
use crate::tabs::Workspace;

const SIDEBAR_CLOSED: f32 = 48.0;
/// 换面板时滑动的距离
const PANEL_SHIFT: f32 = 14.0;

/// 侧栏右边缘可以按住拖动的那一条有多宽（和侧栏右内边距一样宽）
const RESIZE_HANDLE: f32 = 8.0;

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let (collapsed, width, settings_open) = (
        workspace.collapsed,
        workspace.sidebar_width,
        workspace.settings_open,
    );
    let (main_owner, settings_owner) = (workspace.clone(), workspace.clone());
    // 设置页开着时整个侧栏换成设置的分类列表；换到别的标签就换回来
    column()
        .name("sidebar")
        .font_weight(600)
        .width(move || {
            px(if collapsed.get() {
                SIDEBAR_CLOSED
            } else {
                width.get()
            })
        })
        .transition(Property::Width, Spring::snappy())
        .padding_x(move || if collapsed.get() { 6.0 } else { 8.0 })
        .padding_y(8)
        .min_height(0)
        // 两块面板盖满同一块地方，换的时候一块滑出去、一块滑进来：平时的在左，设置的在右，
        // 进设置是往左翻，退出来是往右翻
        .child(
            column()
                .grow(1)
                .min_height(0)
                .child_when(
                    move || !settings_open.get(),
                    move || swap(main_nav::view(&main_owner).inset(0), -PANEL_SHIFT),
                )
                .child_when(
                    move || settings_open.get(),
                    move || swap(settings_nav::view(&settings_owner).inset(0), PANEL_SHIFT),
                ),
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
            aureole::without_animation(|| {
                width.set((event.window_position[0] + offset).clamp(SIDEBAR_MIN, SIDEBAR_MAX))
            });
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
