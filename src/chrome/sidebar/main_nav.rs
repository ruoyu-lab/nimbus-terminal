//! 平时的侧栏：新建会话、搜索，下面是「目标」里的文件夹，底部的设置。
//! 「目标」那一块能接从访达拖来的目录。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::sidebar::nav::{NavExt, nav, section_label};
use crate::chrome::sidebar::targets::{self, Scrolling};
use crate::chrome::widgets::directory_drop;
use crate::icons::Icon;
use crate::tabs::{Shortcut, Workspace};
use crate::theme::{hover, ink, line};

/// 平时的侧栏：顶上新建、搜索，中间是目标和文件夹，底下设置
pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let collapsed = workspace.collapsed;
    let (new_tab, search, settings) = (workspace.clone(), workspace.clone(), workspace.clone());
    // 中间那段的滚动句柄和内容的位置：切到的会话在视口外面时靠它们滚进来
    let (list, content) = (ScrollHandle::new(), signal(Rect::default()));
    let scrolling = Scrolling { list, content };
    // 拖着目录在「目标」上面时，整块淡淡地亮一下，提示可以放
    let over = signal(false);
    let zone = column()
        .name("targets-zone")
        .grow(1)
        .radius(6)
        .background(move || {
            if over.get() {
                hover()
            } else {
                Color::TRANSPARENT
            }
        })
        .transition(Property::Background, Tween::new(ms(120)).ease_out())
        .child(section_label(collapsed, "目标"))
        .child(targets::list(workspace, scrolling))
        .child(targets::wsl(workspace, scrolling))
        .child(targets::remote(workspace, scrolling))
        .child(targets::containers(workspace, scrolling))
        .child(targets::blank(workspace));
    column()
        .grow(1)
        .min_height(0)
        .gap(2)
        // 中间一段吃掉剩下的高度，会话多了在这里竖向滚动，不去挤上下两组
        .child(
            scroll_y(
                column()
                    .gap(2)
                    .min_height(move || list.viewport_size()[1])
                    .report_bounds(content)
                    .child(
                        nav(collapsed, Icon::Plus, "新建会话")
                            .name("nav-new")
                            .hint(collapsed, Shortcut::NewTab.keys())
                            .on_click(move || new_tab.new_tab()),
                    )
                    .child(
                        nav(collapsed, Icon::Search, "搜索")
                            .name("nav-search")
                            .hint(collapsed, Shortcut::Palette.keys())
                            .on_click(move || search.palette_open.set(true)),
                    )
                    .child(directory_drop(zone, workspace, over)),
            )
            .scroll_handle(list)
            .grow(1)
            .min_height(0)
            .scrollbar_color(ink().with_alpha(0.25)),
        )
        // 底下一组固定不动
        .child(
            column()
                .shrink(0)
                .gap(2)
                .child(rect().height(1).background(line()).margin_x(6).margin_y(4))
                .child(
                    nav(collapsed, Icon::Sliders, "设置")
                        .name("nav-settings")
                        .hint(collapsed, Shortcut::Settings.keys())
                        .on_click(move || settings.open_settings()),
                ),
        )
}
