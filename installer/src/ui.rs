//! 安装、卸载的界面：一页，正中一个按钮（样子参考比邻的安装程序）。
//! 装的时候按钮换成一根细进度条，装完换成对勾和「打开 Nimbus」；卸载是同一个样子。
//!
//! 颜色和终端一样是中性的灰阶（`theme.rs` 里的那几档），没有彩色：只有完成的对勾是绿的、出错的字是红的。
//! 深浅跟着系统。窗口不要系统的标题栏，右上角自己画最小化和关闭，整个窗口按住都能拖。

use aureole::prelude::*;
use aureole::{Appearance, current_window, system_appearance};
use aureole_installer::{Mode, Session, State};

use crate::VERSION;

/// 窗口固定大小
pub const WIDTH: f32 = 560.0;
pub const HEIGHT: f32 = 360.0;

/// 右上角窗口按钮那一行的高
const CAPTION_HEIGHT: f32 = 32.0;
/// 窗口按钮的宽（和 Windows 自己的一样）
const CAPTION_WIDTH: f32 = 46.0;
/// 关闭按钮悬停时的红（Windows 自己的）
const CLOSE_RED: Color = Color::hex(0xc42b1c);
const MINIMIZE_GLYPH: &str = "M0 5.5h10";
const CLOSE_GLYPH: &str = "M0.5 0.5l9 9 M9.5 0.5l-9 9";
/// 完成时的对勾（24 × 24 的坐标）
const CHECK_GLYPH: &str = "M5 12.5l4.5 4.5L19 7.5";

/// 终端的图标
const LOGO: &[u8] = include_bytes!("../../assets/icon-windows.png");

/// 一句话介绍（和 README 的一样）
const TAGLINE: &str = "一个干净、好看、能记住你工作现场的终端";

/// 这一页用到的颜色
#[derive(Clone, Copy)]
struct Colors {
    background: Color,
    text: Color,
    secondary: Color,
    tertiary: Color,
    track: Color,
    button: Color,
    button_text: Color,
    hover: Color,
    success: Color,
    danger: Color,
}

impl Colors {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: Color::hex(0x1e1e1e),
                text: Color::hex(0xe2e2e2),
                secondary: Color::hex(0xb8b8b8),
                tertiary: Color::hex(0x909090),
                track: Color::hex(0x353535),
                button: Color::hex(0xe2e2e2),
                button_text: Color::hex(0x1e1e1e),
                hover: Color::hex(0xe2e2e2).with_alpha(0.08),
                success: Color::hex(0x3fb950),
                danger: Color::hex(0xff7b72),
            }
        } else {
            Self {
                background: Color::hex(0xffffff),
                text: Color::hex(0x1f2328),
                secondary: Color::hex(0x555b62),
                tertiary: Color::hex(0x838990),
                track: Color::hex(0xe3e4e6),
                button: Color::hex(0x1f2328),
                button_text: Color::hex(0xffffff),
                hover: Color::hex(0x1f2328).with_alpha(0.08),
                success: Color::hex(0x1a7f37),
                danger: Color::hex(0xcf222e),
            }
        }
    }
}

/// 整个窗口：右上角的窗口按钮，正中按进行到哪换的那一页
pub fn build(session: Session) -> Element {
    view(session, system_appearance() == Appearance::Dark)
}

fn view(session: Session, dark: bool) -> Element {
    let colors = Colors::new(dark);
    if let Some(window) = current_window() {
        window.set_title(match session.mode() {
            Mode::Install => "安装 Nimbus",
            Mode::Uninstall => "卸载 Nimbus",
        });
    }
    let state = session.state();
    let is = move |wanted: fn(&State) -> bool| move || wanted(&state.get());
    column()
        .width(pct(100))
        .height(pct(100))
        .background(colors.background)
        .window_drag_area(true)
        .child(caption(colors))
        .child(
            column()
                .grow(1.0)
                .align(Align::Center)
                .justify(Justify::Center)
                .padding_bottom(CAPTION_HEIGHT)
                .child_when(is(|s| *s == State::Ready), move || ready(session, colors))
                .child_when(is(|s| *s == State::Running), move || {
                    running(session, colors)
                })
                .child_when(is(|s| *s == State::Done), move || done(session, colors))
                .child_when(is(|s| *s == State::Cancelled), move || {
                    cancelled(session, colors)
                })
                .child_when(is(|s| matches!(s, State::Failed { .. })), move || {
                    failed(session, colors)
                }),
        )
}

/// 还没开始：安装时是图标、名字、介绍（升级时说从哪个版本升）、「立即安装」，下面装到哪里、桌面快捷方式；
/// 卸载时说清楚设置会留着
fn ready(session: Session, colors: Colors) -> Element {
    match session.mode() {
        Mode::Install => {
            let previous = session.previous_version();
            let action = row()
                .child_when(
                    move || previous.get().is_none(),
                    move || primary(colors, "立即安装", move || session.start()),
                )
                .child_when(
                    move || previous.get().is_some(),
                    move || primary(colors, "立即更新", move || session.start()),
                );
            let directory = session.directory();
            let below = row()
                .gap(8)
                .align(Align::Center)
                .child(
                    text(move || format!("安装到 {}", directory.get().display()))
                        .font_size(12)
                        .color(colors.tertiary)
                        .max_width(320)
                        .max_lines(1)
                        .name("安装位置"),
                )
                .child(link(colors, "更改", move || session.choose_directory()));
            page(
                logo(),
                name_with_version(colors),
                text(move || install_note(session))
                    .font_size(13)
                    .color(colors.secondary)
                    .name("说明"),
                action,
                below,
            )
            .child(shortcut_option(session, colors))
        }
        Mode::Uninstall => page(
            logo(),
            title(colors, "卸载 Nimbus"),
            note(
                colors.secondary,
                "你的设置、会话和命令历史都会留着，以后重新安装还能接着用",
            ),
            primary(colors, "卸载", move || session.start()),
            link(colors, "取消", move || close(session)),
        ),
    }
}

/// 正在进行：按钮的位置换成一根细进度条，下面写百分比
fn running(session: Session, colors: Colors) -> Element {
    let progress = session.progress();
    let fraction = move || progress.get().fraction.clamp(0.0, 1.0);
    let (heading, note_text, verb) = match session.mode() {
        Mode::Install => (name_with_version(colors), install_note(session), "正在安装"),
        Mode::Uninstall => (
            title(colors, "卸载 Nimbus"),
            "正在删掉程序和快捷方式".to_string(),
            "正在卸载",
        ),
    };
    let bar = row()
        .width(240)
        .height(4)
        .radius(2)
        .background(colors.track)
        .child(
            rect()
                .width(move || px(240.0 * fraction()))
                .height(4)
                .radius(2)
                .background(colors.text),
        )
        .name("进度条");
    page(
        logo(),
        heading,
        note(colors.secondary, &note_text),
        bar,
        text(move || format!("{verb}… {}%", (fraction() * 100.0).round()))
            .font_size(12)
            .color(colors.tertiary)
            .name("进度"),
    )
}

/// 完成了：图标换成对勾。装好了的可以直接打开
fn done(session: Session, colors: Colors) -> Element {
    match session.mode() {
        Mode::Install => page(
            check(colors),
            title(colors, "装好了"),
            note(colors.secondary, "以后在开始菜单里也能找到 Nimbus"),
            primary(colors, "打开 Nimbus", move || {
                session.launch_after().set(true);
                session.finish();
            }),
            link(colors, "完成", move || close(session)),
        ),
        Mode::Uninstall => page(
            check(colors),
            title(colors, "已卸载"),
            note(
                colors.secondary,
                "Nimbus 已经从这台电脑上删掉了，你的设置还留着",
            ),
            primary(colors, "完成", move || close(session)),
            row(),
        ),
    }
}

/// 取消了（已经撤销）：可以再来一次
fn cancelled(session: Session, colors: Colors) -> Element {
    let note_text = match session.mode() {
        Mode::Install => "已经撤销，这台电脑上什么也没改",
        Mode::Uninstall => "Nimbus 还在，什么也没删",
    };
    page(
        logo(),
        title(colors, "已取消"),
        note(colors.secondary, note_text),
        primary(colors, "再试一次", move || session.start()),
        link(colors, "关闭", move || close(session)),
    )
}

/// 失败了（已经撤销）：红字写原因。最常见的是 Nimbus 还开着，换成一句人话
fn failed(session: Session, colors: Colors) -> Element {
    let heading = match session.mode() {
        Mode::Install => "没装成",
        Mode::Uninstall => "没卸载完",
    };
    let reason = match session.state().get_untracked() {
        State::Failed {
            file_in_use: true, ..
        } => "Nimbus 还开着：关掉它以后再试一次".to_string(),
        State::Failed { message, .. } => breakable(&message),
        _ => String::new(),
    };
    page(
        logo(),
        title(colors, heading),
        note(colors.danger, &reason),
        primary(colors, "再试一次", move || session.start()),
        link(colors, "关闭", move || close(session)),
    )
}

/// 一页的样子：上面一个 64 的图，名字，一两行说明，按钮（或者进度条），下面一行小字
fn page(
    badge: Element,
    heading: Element,
    note: Element,
    action: Element,
    below: Element,
) -> Element {
    column()
        .width(400)
        .align(Align::Center)
        .gap(8)
        .child(badge)
        .child(heading.margin_top(6))
        .child(note)
        .child(
            row()
                .height(40)
                .margin_top(16)
                .align(Align::Center)
                .justify(Justify::Center)
                .child(action),
        )
        .child(
            row()
                .height(18)
                .margin_top(4)
                .align(Align::Center)
                .justify(Justify::Center)
                .child(below),
        )
}

/// 安装页的说明：第一次装是一句介绍；目录里已经装过时说清楚是升级还是重装
fn install_note(session: Session) -> String {
    match session.previous_version().get() {
        None => TAGLINE.to_string(),
        Some(previous) if previous == session.manifest().version => {
            format!("这台电脑上已经装了，会重新安装 {VERSION}")
        }
        Some(previous) => format!("会从 {previous} 更新到 {VERSION}"),
    }
}

fn logo() -> Element {
    image(ImageSource::from_static(LOGO))
        .size(64, 64)
        .name("图标")
}

/// 完成时换上的对勾：淡绿的圆底
fn check(colors: Colors) -> Element {
    row()
        .size(64, 64)
        .radius(32)
        .background(colors.success.with_alpha(0.14))
        .align(Align::Center)
        .justify(Justify::Center)
        .color(colors.success)
        .child(
            path(VectorPath::svg(CHECK_GLYPH).view_box(0.0, 0.0, 24.0, 24.0))
                .size(30.0, 30.0)
                .stroke(Stroke::new(2.4)),
        )
        .name("完成")
}

fn title(colors: Colors, value: &str) -> Element {
    text(value.to_string())
        .font_size(20)
        .font_weight(700.0)
        .color(colors.text)
        .name("标题")
}

/// 「Nimbus」后面跟一个小一号的版本号
fn name_with_version(colors: Colors) -> Element {
    row()
        .gap(8)
        .align(Align::End)
        .child(title(colors, "Nimbus"))
        .child(
            text(VERSION)
                .font_size(12)
                .color(colors.tertiary)
                .margin_bottom(4)
                .name("版本"),
        )
}

fn note(color: Color, value: &str) -> Element {
    text(value.to_string())
        .font_size(13)
        .color(color)
        .text_align(TextAlign::Center)
        .width(pct(100))
        .name("说明")
}

/// 深色的主按钮（深色模式里是浅色的）
fn primary(colors: Colors, label: &str, on_click: impl Fn() + 'static) -> Element {
    let hover = colors.button.with_alpha(0.88);
    let press = colors.button.with_alpha(0.78);
    let style = Style::new()
        .width(200)
        .height(40)
        .radius(8)
        .font_size(14)
        .background(colors.button)
        .border_color(colors.button)
        .color(colors.button_text)
        .align(Align::Center)
        .justify(Justify::Center)
        .hovered(move |s| s.background(hover).border_color(hover))
        .pressed(move |s| s.background(press).border_color(press));
    button(label.to_string())
        .style(&style)
        .on_click(on_click)
        .name(format!("按钮：{label}"))
}

/// 小字的链接：更改、取消、完成
fn link(colors: Colors, label: &str, on_click: impl Fn() + 'static) -> Element {
    let style = Style::new()
        .font_size(12)
        .color(colors.secondary)
        .hovered(move |s| s.color(colors.text));
    text(label.to_string())
        .style(&style)
        .cursor(Cursor::Pointer)
        .on_click(on_click)
        .name(format!("链接：{label}"))
}

/// 「在桌面上创建快捷方式」（开始菜单的总是建）
fn shortcut_option(session: Session, colors: Colors) -> Element {
    let indicator = Style::new()
        .size(15, 15)
        .radius(4)
        .border_color(colors.tertiary)
        .checked(move |s| s.background(colors.button).border_color(colors.button));
    let mark = Style::new().color(colors.button_text);
    let label = Style::new().font_size(12).color(colors.secondary);
    checkbox(session.desktop_shortcut())
        .label("在桌面上创建快捷方式")
        .style(&label)
        .indicator_style(&indicator)
        .mark_style(&mark)
        .into_element()
        .margin_top(2)
        .name("桌面快捷方式")
}

/// 右上角：最小化、关闭（窗口大小固定，没有最大化）
fn caption(colors: Colors) -> Element {
    let window = current_window();
    row()
        .width(pct(100))
        .height(CAPTION_HEIGHT)
        .justify(Justify::End)
        .child(
            caption_button(colors, false)
                .name("最小化")
                .child(glyph(MINIMIZE_GLYPH))
                .on_click(move || {
                    if let Some(window) = window {
                        window.set_minimized(true);
                    }
                }),
        )
        .child(
            caption_button(colors, true)
                .name("关闭")
                .child(glyph(CLOSE_GLYPH))
                .on_click(move || {
                    if let Some(window) = window {
                        window.close();
                    }
                }),
        )
}

/// 窗口按钮的底：悬停一层淡灰（关闭是红底白字）
fn caption_button(colors: Colors, close: bool) -> Element {
    let style = Style::new()
        .width(CAPTION_WIDTH)
        .height(CAPTION_HEIGHT)
        .color(colors.secondary)
        .transition(Property::Background, Tween::new(ms(100)).ease_out());
    let style = if close {
        style
            .hovered(|s| s.background(CLOSE_RED).color(Color::WHITE))
            .pressed(|s| s.background(CLOSE_RED.with_alpha(0.85)).color(Color::WHITE))
    } else {
        style
            .hovered(move |s| s.background(colors.hover).color(colors.text))
            .pressed(move |s| s.background(colors.track).color(colors.text))
    };
    row()
        .style(&style)
        .align(Align::Center)
        .justify(Justify::Center)
}

fn glyph(data: &'static str) -> Element {
    path(VectorPath::svg(data).view_box(0.0, 0.0, 10.0, 10.0))
        .size(10.0, 10.0)
        .stroke(Stroke::new(1.0))
}

/// 关窗口退出，不打开 Nimbus（完成、取消、关闭）
fn close(session: Session) {
    session.launch_after().set(false);
    session.finish();
}

/// 长路径在反斜杠后面能折行
fn breakable(message: &str) -> String {
    message.replace('\\', "\\\u{200b}")
}

#[cfg(test)]
mod tests;
