//! 终端内查找条（⌘F）：浮在终端右上角的一小条，输入框、第几处/一共几处、上一处、下一处、关闭。

use std::rc::Rc;

use aureole::prelude::*;

use crate::app::Session;
use crate::chrome::widgets::{icon_button, pop};
use crate::icons::{Icon, icon};
use crate::theme::{card_bg, popup_shadow, text_3};

pub(super) fn bar(session: Rc<Session>) -> Element {
    let (query, status) = (session.find_query(), session.find_status());
    let (submit, older, newer, close) = (
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
    );
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
                .tooltip("上一处  回车 / ⌘G")
                .on_click(move || older.find_step(true)),
        )
        .child(
            icon_button(Icon::ArrowDown, 15.0)
                .name("find-newer")
                .tooltip("下一处  ⌘⇧G")
                .on_click(move || newer.find_step(false)),
        )
        .child(
            icon_button(Icon::Close, 15.0)
                .name("find-close")
                .tooltip("关闭  Esc")
                .on_click(move || close.close_find()),
        ))
}
