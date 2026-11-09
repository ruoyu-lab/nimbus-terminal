//! 窗口的外观：标题栏（标签）、侧栏、主卡片、状态条、命令面板。只管画和接事件，状态在 [`Workspace`]。
//!
//! 布局（参考 Qoder 的清爽）：
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ ● ● ●  ▭  [标签] [标签] [+]                      ▥ ▤     │ 标题栏（拖动窗口）
//! ├────────────┬─────────────────────────────────────────────┤
//! │ ＋ 新建会话 │ ┌─────────────────────────────────────────┐ │
//! │ ⌕ 搜索     │ │               终端                       │ │
//! │ 目标       │ ├─────────────────────────────────────────┤ │
//! │  文件夹    │ │ 本机 · zsh                      120×36   │ │ 状态条
//! └────────────┴─┴─────────────────────────────────────────┴─┘
//! ```
//!
//! 侧栏底色和标题栏是同一块灰白，主卡片是白色的圆角卡片浮在上面，靠投影区分。
//! 整个界面全是灰阶，没有彩色。侧栏和卡片的交界处可以左右拖动改侧栏宽度。
//!
//! 设置页打开时：标题栏只剩侧栏开关和「设置」，标签条和分屏按钮收起；侧栏顶上是「返回」，下面是设置的分类。
//!
//! 每一块是一个子模块，各自暴露一个 `view(workspace)`，这里只管把它们拼起来：
//! `title_bar`、`sidebar`（平时和设置两种内容）、`card`（设置页、终端、没有标签时的引导）、`palette`、`menu`，
//! 共用的小零件在 `widgets`。

mod card;
mod menu;
mod palette;
mod panel;
mod settings;
mod sidebar;
mod title_bar;
mod widgets;

use std::rc::Rc;

use aureole::prelude::*;
use aureole::{Appearance, capabilities, system_appearance};

use crate::tabs::{FolderId, Workspace};
use crate::theme::{fg, is_dark, scrim, sidebar_bg};

/// 标题栏的最小高度；系统给的更高就用系统的
pub const BAR_HEIGHT: f32 = 40.0;
/// 侧栏展开时的默认宽度
pub const SIDEBAR_DEFAULT: f32 = 232.0;
/// 拖交界处改侧栏宽度时的下限和上限
pub const SIDEBAR_MIN: f32 = 176.0;
pub const SIDEBAR_MAX: f32 = 360.0;
/// 右侧面板的默认宽度和拖动范围
pub const PANEL_DEFAULT: f32 = 280.0;
pub const PANEL_MIN: f32 = 220.0;
pub const PANEL_MAX: f32 = 520.0;
const STATUS_HEIGHT: f32 = 34.0;
/// 主卡片离窗口右边、下边的空隙
const GUTTER: f32 = 8.0;

/// 窗口这么大（内容区）时终端区域（卡片去掉状态条）有多大，用来估第一版行列数
pub fn terminal_area((width, height): (f32, f32)) -> (f32, f32) {
    (
        width - SIDEBAR_DEFAULT - GUTTER,
        height - BAR_HEIGHT - GUTTER - STATUS_HEIGHT - 1.0,
    )
}

/// 窗口的根视图。深浅色变了就整个重新建一遍：颜色都是建元素时取的值（见 `theme`），
/// 状态都在 [`Workspace`] 里，重建不丢东西
pub fn build(workspace: Rc<Workspace>) -> Element {
    // 窗口标题跟着当前标签：任务切换、读屏软件用得上
    if let Some(window) = workspace.window {
        let (active, titles) = (workspace.active, workspace.titles);
        Effect::new(move || {
            let title = titles.get().get(&active.get()).cloned().unwrap_or_default();
            window.set_title(if title.is_empty() {
                crate::product::NAME.to_string()
            } else {
                title
            });
        });
    }
    let dark = crate::theme::dark();
    let (shortcuts, dropper) = (workspace.clone(), workspace.clone());
    column()
        .grow(1)
        .min_width(0)
        .min_height(0)
        // 快捷键挂在最外层：键盘没落在哪个元素上时事件也到这里
        .on_key_down_capture(move |event: &KeyEvent| {
            if shortcuts.shortcut(event) {
                event.stop_propagation();
            }
        })
        // 拖着标签放在窗口里的哪儿都算放下（换顺序按被拖的标签自己的位置算，不看指针在哪），
        // 手稍微偏出标签条也不会变成取消
        .accepts_drop(|_: &u64| true)
        .on_drop(move |id: u64, _: &DropEvent| dropper.drop_tab(id))
        .each(
            move || vec![dark.get()],
            |dark| *dark,
            move |_| frame(workspace.clone()),
        )
}

/// 一套深浅色下的整个窗口
fn frame(workspace: Rc<Workspace>) -> Element {
    let folder_dropper = workspace.clone();
    let (palette_owner, rename_owner, folder_owner, closing_owner) = (
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
    );
    let connect_owner = workspace.clone();
    let connect_prompt = workspace.connect_prompt;
    let docker_owner = workspace.clone();
    let docker_prompt = workspace.docker_prompt;
    let (palette_open, renaming, folder_prompt, closing) = (
        workspace.palette_open,
        workspace.renaming,
        workspace.folder_prompt,
        workspace.closing,
    );
    // 外框（标题栏和侧栏）用系统的窗口材质，这里只盖一层很淡的底，保证字看得清：
    // 浅色系统里材质本身是浅的，几乎不用盖；深色系统里材质是深的，要盖一层浅色，不然深色的字看不见。
    // 平台不支持材质时退回不透明的底色
    let has_backdrop = capabilities().supports_window_backdrop;
    let backdrop = move || {
        if !has_backdrop {
            return sidebar_bg();
        }
        let system_dark = system_appearance() == Appearance::Dark;
        match (is_dark(), system_dark) {
            (false, false) => Color::WHITE.with_alpha(0.28),
            (true, true) => sidebar_bg().with_alpha(0.84),
            // 界面和系统的深浅不一样：系统的材质是另一种颜色，要盖得厚一点才看不出来
            _ => sidebar_bg().with_alpha(0.98),
        }
    };
    column()
        .grow(1)
        .min_width(0)
        .min_height(0)
        .font_size(13)
        .color(fg())
        .background(backdrop)
        // 拖着文件夹放在窗口里的哪儿都算放下（换顺序按被拖的文件夹自己的位置算）
        .accepts_drop(|_: &FolderId| true)
        .on_drop(move |id: FolderId, _: &DropEvent| folder_dropper.drop_folder(id.0))
        .child(title_bar::view(&workspace))
        .child(
            row()
                .grow(1)
                .min_height(0)
                .child(sidebar::view(&workspace))
                .child(card::view(&workspace))
                .child(panel::view(&workspace)),
        )
        .overlay(
            palette_open,
            Overlay::new(move || palette::view(palette_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| palette_open.set(false)),
        )
        .overlay(
            move || renaming.get().is_some(),
            Overlay::new(move || menu::rename_dialog(rename_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| renaming.set(None)),
        )
        .overlay(
            move || folder_prompt.get().is_some(),
            Overlay::new(move || menu::folder_dialog(folder_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| folder_prompt.set(None)),
        )
        .overlay(
            connect_prompt,
            Overlay::new(move || menu::connect_dialog(connect_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| connect_prompt.set(false)),
        )
        .overlay(
            docker_prompt,
            Overlay::new(move || menu::docker_dialog(docker_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| docker_prompt.set(false)),
        )
        .overlay(
            move || closing.get().is_some(),
            Overlay::new(move || menu::close_dialog(closing_owner.clone()))
                .centered(true)
                .modal(true)
                .backdrop(scrim())
                .backdrop_transition(Tween::new(ms(160)).ease_out())
                .on_dismiss(move |_| closing.set(None)),
        )
}
