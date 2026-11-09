//! 设置打开时的侧栏：顶上是「返回」（在平时放新建会话那一行的位置），下面是设置的分类列表。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::settings::page_glyph;
use crate::chrome::sidebar::nav::{NavExt, nav, nav_selected};
use crate::icons::Icon;
use crate::tabs::{SettingsPage, Shortcut, Workspace};
use crate::theme::{ink, line};

/// 一行一类，选中的那一类的内容显示在卡片里
pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let (collapsed, page) = (workspace.collapsed, workspace.settings_page);
    let back = workspace.clone();
    column()
        .grow(1)
        .min_height(0)
        .gap(2)
        // 返回固定在最上面，分类多了也只有下面一段滚动
        .child(
            column()
                .shrink(0)
                .gap(2)
                .child(
                    nav(collapsed, Icon::ChevronLeft, "返回")
                        .name("settings-back")
                        .hint(collapsed, Shortcut::Settings.keys())
                        .on_click(move || back.close_settings()),
                )
                .child(rect().height(1).background(line()).margin_x(6).margin_y(4)),
        )
        .child(
            scroll_y(
                column()
                    .gap(2)
                    .children(SettingsPage::ALL.into_iter().map(move |entry| {
                        nav_selected(collapsed, page_glyph(entry), entry.label(), move || {
                            page.get() == entry
                        })
                        .name(entry.name())
                        .on_click(move || page.set(entry))
                    })),
            )
            .grow(1)
            .min_height(0)
            .scrollbar_color(ink().with_alpha(0.25)),
        )
}
