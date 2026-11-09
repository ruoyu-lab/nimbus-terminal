//! 终端内查找条（⌘F）：浮在终端右上角的一小条，输入框、第几处/一共几处、上一处、下一处、关闭。

use std::rc::Rc;

use aureole::prelude::*;

use crate::app::Session;
use crate::chrome::widgets::{icon_button, pop};
use crate::icons::{Icon, icon};
use crate::tabs::Shortcut;
use crate::theme::{card_bg, popup_shadow, text_3};

/// 输入框里 Shift+回车往下（更新的）找：Windows、Linux 的习惯。macOS 上照旧和回车一样（往下是 ⌘⇧G）
const SHIFT_ENTER_FINDS_NEWER: bool = !Shortcut::MAC;

pub(super) fn bar(session: Rc<Session>) -> Element {
    let (query, status) = (session.find_query(), session.find_status());
    let (submit, shifted, older, newer, close) = (
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
    );
    let newer_keys = if SHIFT_ENTER_FINDS_NEWER {
        format!("Shift+回车 / {}", Shortcut::FindNewer.keys())
    } else {
        Shortcut::FindNewer.keys().to_string()
    };
    pop(row()
        .name("find-bar")
        .align(Align::Center)
        .gap(2)
        .padding_x(8)
        .padding_y(4)
        .radius(9)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            row()
                .padding_x(4)
                .color(text_3())
                .child(icon(Icon::Search, 15.0)),
        )
        .child(
            text_field(query)
                .name("find-input")
                .placeholder("在终端里查找")
                .width(170)
                // 框和焦点环都去掉：整条就是输入区
                .background(Color::TRANSPARENT)
                .border(0, Color::TRANSPARENT)
                .focus_ring(FocusRing::NONE)
                .font_size(13)
                .on_key_down(move |event: &KeyEvent| {
                    let shift_enter = matches!(event.key, Key::Named(NamedKey::Enter))
                        && event.modifiers == Modifiers::SHIFT;
                    if SHIFT_ENTER_FINDS_NEWER && shift_enter {
                        // 不让输入框把它也当回车提交
                        event.prevent_builtin(Builtin::EDIT);
                        event.stop_propagation();
                        shifted.find_step(false);
                    }
                })
                // 回车：往上（更早的）找下一处，和从最新往回找的习惯一致
                .on_submit(move |_| submit.find_step(true)),
        )
        .child(
            text(move || status.get())
                .name("find-status")
                .min_width(48)
                .font_size(12)
                .color(text_3())
                .max_lines(1),
        )
        .child(
            icon_button(Icon::ArrowUp, 15.0)
                .name("find-older")
                .tooltip(format!("上一处  回车 / {}", Shortcut::FindOlder.keys()))
                .on_click(move || older.find_step(true)),
        )
        .child(
            icon_button(Icon::ArrowDown, 15.0)
                .name("find-newer")
                .tooltip(format!("下一处  {newer_keys}"))
                .on_click(move || newer.find_step(false)),
        )
        .child(
            icon_button(Icon::Close, 15.0)
                .name("find-close")
                .tooltip("关闭  Esc")
                .on_click(move || close.close_find()),
        ))
}
