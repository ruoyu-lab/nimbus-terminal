//! 一个终端的界面：字符网格，加叠在上面的图片，和挂在网格上的各种事件。

use super::*;

/// 会话的视图：网格加上把按键、指针、滚轮、输入法接到会话上的处理器。
/// 底色和文字颜色跟着界面的深浅走。`wanted` 说这个终端现在该不该拿键盘焦点（拆分出来的几个里聚焦的那个）：
/// 挂上时、它由假变真时，网格就拿到焦点。`on_menu` 在该弹右键菜单时调用，参数是点的位置（窗口坐标）
pub fn view(
    session: Rc<Session>,
    wanted: impl Fn() -> bool + 'static,
    on_menu: impl Fn([f32; 2]) + 'static,
) -> Element {
    let grid = session.grid.clone();
    let form = session.form;
    // 窗口的内容区（算放得下几列几行）和网格自己的边界（反算格子大小）
    let viewport = signal(Rect::default());
    let shown = signal(Rect::default());
    let fitter = session.clone();
    Effect::new(move || fitter.fit(viewport.get(), shown.get()));

    // 配色、光标变了整屏重画；挂上时也画一次（设置是在别的页面改的，这个视图当时没挂着）
    let styler = session.clone();
    Effect::new(move || {
        let _ = (
            crate::theme::is_dark(),
            form.cursor.get(),
            form.cursor_blink.get(),
        );
        styler.restyle();
    });
    // 字体、字号、行距变了：旧的格子大小作废，等网格按新字体布局出来再量
    let measurer = session.clone();
    Effect::new(move || {
        let _ = (
            form.font.get(),
            form.font_weight.get(),
            form.font_size.get(),
            form.line_height.get(),
        );
        measurer.remeasure(shown.get_untracked());
    });
    // 响铃设成「画面闪一下」：底色亮一下再回来。数字比挂上时多了才闪，切回这个标签不会补闪
    let flashing = signal(false);
    let flashes = session.flashes;
    let seen = Rc::new(Cell::new(flashes.get_untracked()));
    Effect::new(move || {
        let count = flashes.get();
        if seen.replace(count) != count {
            flashing.set(true);
            aureole::spawn_local(async move {
                aureole::sleep(ms(110)).await;
                flashing.set(false);
            });
        }
    });

    let focus = FocusHandle::new();
    let wanted = Rc::new(wanted);
    let on_change = wanted.clone();
    Effect::new(move || {
        if on_change() {
            focus.focus();
        }
    });
    // 明确要焦点（放下东西之后）：第一次（0）是刚建好，不算
    let requested = session.focus_requests;
    let on_request = wanted.clone();
    Effect::new(move || {
        if requested.get() > 0 && on_request() {
            focus.focus();
        }
    });
    let (keys_session, ime_session, wheel_session, leave_session, image_wheel_session) = (
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
    );
    let images = session.images;
    let hovering = session.hovering_link;
    let (down_session, up_session, move_session, focus_session, click_session) = (
        session.clone(),
        session.clone(),
        session.clone(),
        session.clone(),
        session,
    );
    column()
        .grow(1)
        // 终端区域的大小由窗口决定、不由里面的网格决定：网格比窗口宽时要被压住（再由 `fit` 缩小行列），
        // 不能把整块卡片撑出窗口
        .min_width(0)
        .min_height(0)
        .clip(true)
        .padding(PADDING)
        .background(move || {
            let colors = view::colors(crate::theme::is_dark());
            if flashing.get() {
                colors.flash
            } else {
                colors.background
            }
        })
        .transition(Property::Background, Tween::new(ms(90)).ease_out())
        .color(move || view::colors(crate::theme::is_dark()).foreground)
        .report_bounds(viewport)
        .on_mount(move || {
            if wanted() {
                focus.focus();
            }
        })
        .child(
            // 网格和叠在它上面的图片放在同一个容器里：图片按格子算位置，和网格的左上角对齐
            column()
                .align_self(Align::Start)
                // 鼠标在图片上滚也要滚终端（图片不是网格，滚轮事件到不了网格那儿）
                .on_wheel(move |event: &WheelEvent| {
                    image_wheel_session.wheel(event);
                    event.stop_propagation();
                })
                .child(
                    char_grid(&grid)
                        .name("grid")
                        .align_self(Align::Start)
                        .focus_handle(focus)
                        // 键盘切标签后终端拿到焦点：整块终端就是输入区，不在外面画框
                        .focus_ring(FocusRing::NONE)
                        .report_bounds(shown)
                        .on_secondary_click(move |event: &ClickEvent| {
                            if click_session.secondary_click(event.position, event.modifiers) {
                                event.stop_propagation();
                                on_menu(event.window_position);
                            }
                        })
                        .font_weight(move || form.font_weight.get().value())
                        .font_size(move || form.font_size.get() as f32)
                        .line_height(move || form.line_height.get() as f32)
                        .font_family(move || form.font.get().family().to_string())
                        .on_key_down(move |event: &KeyEvent| {
                            if keys_session.key(event) {
                                event.stop_propagation();
                            }
                        })
                        // 程序要鼠标时（vim、htop、tmux）把指针事件编码了写给它；不要时这几个处理器什么都不做，
                        // 选字、复制照旧由网格自己处理
                        .on_pointer_down(move |event: &PointerEvent| {
                            down_session.pointer_down(event)
                        })
                        .on_pointer_up(move |event: &PointerEvent| up_session.pointer_up(event))
                        .on_pointer_move(move |event: &PointerEvent| {
                            move_session.pointer_move(event)
                        })
                        .on_pointer_leave(move |_: &PointerEvent| leave_session.pointer_over(None))
                        // 指针在链接上是手指，别处是文字光标
                        .cursor(move || {
                            if hovering.get() {
                                Cursor::Pointer
                            } else {
                                Cursor::Text
                            }
                        })
                        .on_keyboard_focus(move |focused: bool| {
                            focus_session.focus_changed(focused)
                        })
                        .on_wheel(move |event: &WheelEvent| {
                            wheel_session.wheel(event);
                            event.stop_propagation();
                        })
                        // 输入法：预编辑画在插入点处，提交的字原样写给 shell
                        .on_text_insert(move |text: String| ime_session.commit_text(&text)),
                )
                // 图片直接是容器的子元素（各自按位置摆）：不铺一层盖满网格的东西，它会挡住网格上的鼠标事件
                .clip(true)
                .each(
                    move || {
                        images
                            .get()
                            .into_iter()
                            .map(|image| image.id)
                            .collect::<Vec<_>>()
                    },
                    |id| *id,
                    move |id| terminal_image(images, id),
                ),
        )
}
