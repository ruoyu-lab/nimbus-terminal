//! 设置页：侧栏选的那一类的内容。
//!
//! 这里每一项都是真的：读写的是 [`crate::settings::Form`] 的信号，会话、标签、窗口直接读它，
//! 改了马上生效，也马上写盘。

use aureole::prelude::*;

use crate::chrome::widgets::{outline_button, soft};
use crate::icons::Icon;
use crate::settings::{
    Bell, CursorStyle, DirectoryKind, FONT_SIZE, FontChoice, FontWeight, Form, LINE_HEIGHT,
    LastTab, SCROLLBACK, Settings, ShellKind, SidebarStart, Startup, ThemeChoice, TitleSource,
    WHEEL,
};
use crate::tabs::{SettingsPage, Workspace};
use crate::theme::{accent, card_bg, fg, hover, ink, line, popup_shadow, text_2, text_3};
use crate::view;

pub(super) fn page_glyph(page: SettingsPage) -> Icon {
    match page {
        SettingsPage::General => Icon::Sliders,
        SettingsPage::Appearance => Icon::Sun,
        SettingsPage::Terminal => Icon::Terminal,
        SettingsPage::Shortcuts => Icon::Keyboard,
        SettingsPage::About => Icon::Info,
    }
}

/// 设置页：侧栏选了哪一类，这里就显示哪一类
pub(super) fn view(workspace: &Workspace) -> Element {
    let (page, form) = (workspace.settings_page, workspace.form);
    let location = workspace
        .config_directory()
        .map_or_else(|| "没有启用".to_string(), |path| path.display().to_string());
    let mut view = column()
        .name("settings-view")
        .grow(1)
        .min_width(0)
        .min_height(0);
    for entry in SettingsPage::ALL {
        let location = location.clone();
        view = view.child_when(
            move || page.get() == entry,
            move || settings_page(entry, form, &location).name(format!("{}-page", entry.name())),
        );
    }
    view
}

fn settings_page(page: SettingsPage, form: Form, location: &str) -> Element {
    match page {
        SettingsPage::General => general(form),
        SettingsPage::Appearance => appearance(form),
        SettingsPage::Terminal => terminal(form),
        SettingsPage::Shortcuts => shortcuts_page(),
        SettingsPage::About => about_page(form, location),
    }
}

fn general(form: Form) -> Element {
    settings_list(
        "通用",
        "新终端从哪里、用什么开始，标签和窗口怎么收尾。",
        groups(vec![
            (
                "新终端",
                vec![
                    setting(
                        "启动时",
                        segments(
                            "set-startup",
                            &[
                                ("empty", "空白", Startup::Empty),
                                ("restore", "恢复上次的标签", Startup::Restore),
                            ],
                            form.startup,
                        ),
                    ),
                    setting(
                        "恢复时带上终端里的内容",
                        toggle(
                            form.save_output,
                            "set-save-output",
                            "恢复时带上终端里的内容",
                        ),
                    ),
                    followed_by(
                        setting(
                            "shell",
                            segments(
                                "set-shell",
                                &[
                                    ("login", "系统默认", ShellKind::Login),
                                    ("custom", "指定程序", ShellKind::Program),
                                ],
                                form.shell,
                            ),
                        ),
                        move || form.shell.get() == ShellKind::Program,
                        move || {
                            picker_field(
                                form.shell_program,
                                "set-shell-path",
                                "/bin/zsh",
                                Picks::File,
                            )
                        },
                    ),
                    followed_by(
                        setting(
                            "起始目录",
                            segments(
                                "set-directory",
                                &[
                                    ("home", "主目录", DirectoryKind::Home),
                                    ("custom", "指定目录", DirectoryKind::Custom),
                                ],
                                form.directory,
                            ),
                        ),
                        move || form.directory.get() == DirectoryKind::Custom,
                        move || {
                            picker_field(
                                form.directory_path,
                                "set-directory-path",
                                "~/",
                                Picks::Directory,
                            )
                        },
                    ),
                ],
            ),
            (
                "标签",
                vec![
                    setting(
                        "标签名称",
                        segments(
                            "set-title",
                            &[
                                ("fixed", "固定叫「终端」", TitleSource::Fixed),
                                ("reported", "跟随程序的标题", TitleSource::Reported),
                            ],
                            form.title,
                        ),
                    ),
                    setting(
                        "有程序在跑时，关闭前先确认",
                        toggle(form.confirm_close, "set-confirm-close", "关闭前先确认"),
                    ),
                ],
            ),
            (
                "窗口",
                vec![
                    setting(
                        "关掉最后一个标签",
                        segments(
                            "set-last-tab",
                            &[
                                ("keep", "留着窗口", LastTab::Keep),
                                ("quit", "退出应用", LastTab::Quit),
                            ],
                            form.last_tab,
                        ),
                    ),
                    setting(
                        "启动时的侧栏",
                        segments(
                            "set-sidebar",
                            &[
                                ("open", "展开", SidebarStart::Open),
                                ("closed", "收起", SidebarStart::Closed),
                            ],
                            form.sidebar,
                        ),
                    ),
                ],
            ),
        ]),
    )
}

fn appearance(form: Form) -> Element {
    let body = column()
        .child(preview(form))
        .child(rect().height(18))
        .child(groups(vec![
            (
                "主题",
                vec![setting(
                    "外观",
                    segments(
                        "set-appearance",
                        &[
                            ("light", "浅色", ThemeChoice::Light),
                            ("dark", "深色", ThemeChoice::Dark),
                            ("system", "跟随系统", ThemeChoice::System),
                        ],
                        form.appearance,
                    ),
                )],
            ),
            (
                "文字",
                vec![
                    setting("字体", font_menu(form.font)),
                    setting("粗细", weight_menu(form.font_weight)),
                    setting(
                        "字号",
                        stepper(
                            form.font_size,
                            "set-font-size",
                            FONT_SIZE.0,
                            FONT_SIZE.1,
                            1.0,
                            0,
                            36.0,
                        ),
                    ),
                    setting(
                        "行距",
                        stepper(
                            form.line_height,
                            "set-line-height",
                            LINE_HEIGHT.0,
                            LINE_HEIGHT.1,
                            0.1,
                            1,
                            40.0,
                        ),
                    ),
                ],
            ),
            (
                "光标",
                vec![
                    setting(
                        "形状",
                        segments(
                            "set-cursor",
                            &[
                                ("block", "方块", CursorStyle::Block),
                                ("underline", "下划线", CursorStyle::Underline),
                                ("bar", "竖线", CursorStyle::Bar),
                            ],
                            form.cursor,
                        ),
                    ),
                    setting(
                        "闪烁",
                        toggle(form.cursor_blink, "set-cursor-blink", "光标闪烁"),
                    ),
                ],
            ),
            (
                "界面",
                vec![setting(
                    "底部状态条",
                    toggle(form.status_bar, "set-status", "显示底部状态条"),
                )],
            ),
        ]));
    settings_list(
        "外观",
        "界面和终端用同一套配色，终端的颜色跟着外观走。上面的样子是实时的。",
        body,
    )
}

fn terminal(form: Form) -> Element {
    settings_list(
        "终端",
        "滚动、鼠标和响铃。",
        groups(vec![
            (
                "滚动",
                vec![
                    setting(
                        "历史",
                        count(
                            form.scrollback,
                            "set-scrollback",
                            SCROLLBACK.0,
                            SCROLLBACK.1,
                            1_000.0,
                            72.0,
                            "行",
                        ),
                    ),
                    setting(
                        "滚轮一格滚",
                        count(form.wheel, "set-wheel", WHEEL.0, WHEEL.1, 1.0, 36.0, "行"),
                    ),
                ],
            ),
            (
                "鼠标",
                vec![
                    setting(
                        "选中后自动复制",
                        toggle(form.copy_on_select, "set-copy-on-select", "选中后自动复制"),
                    ),
                    setting(
                        "右键直接粘贴（关着就弹菜单）",
                        toggle(form.right_click_paste, "set-right-click", "右键粘贴"),
                    ),
                ],
            ),
            (
                "响铃",
                vec![setting(
                    "程序响铃时",
                    segments(
                        "set-bell",
                        &[
                            ("off", "什么也不做", Bell::Off),
                            ("sound", "系统提示音", Bell::Sound),
                            ("flash", "画面闪一下", Bell::Flash),
                        ],
                        form.bell,
                    ),
                )],
            ),
        ]),
    )
}

/// 有内容的设置页：标题、一句说明，下面是分组或一列设置
fn settings_list(title: &'static str, hint: &'static str, body: Element) -> Element {
    scroll_y(
        column()
            .padding(32)
            .child(text(title).font_size(20).font_weight(600))
            .child(
                // 没有说明的页（关于）：标题下面只留空隙
                text(hint)
                    .font_size(12)
                    .color(text_3())
                    .margin_top(if hint.is_empty() { 0 } else { 4 })
                    .margin_bottom(20),
            )
            .child(body.name("settings-list")),
    )
    .grow(1)
    .min_height(0)
    .scrollbar_color(ink().with_alpha(0.25))
}

fn groups(sections: Vec<(&'static str, Vec<Element>)>) -> Element {
    let mut body = column();
    for (index, (title, rows)) in sections.into_iter().enumerate() {
        if index > 0 {
            body = body.child(rect().height(14));
        }
        body = body
            .child(text(title).font_size(11).color(text_3()).margin_bottom(6))
            .child(stack(rows));
    }
    body
}

fn stack(rows: Vec<Element>) -> Element {
    let mut list = column();
    for (index, entry) in rows.into_iter().enumerate() {
        if index > 0 {
            list = list.child(rect().height(1).background(line()));
        }
        list = list.child(entry);
    }
    list
}

/// 一行：左边名字，右边控件
fn setting(label: &'static str, control: impl View) -> Element {
    row()
        .min_height(40)
        .padding_y(6)
        .align(Align::Center)
        .gap(16)
        .child(text(label).grow(1).min_width(0).max_lines(1).font_size(13))
        .child(control)
}

/// 主行下面、靠右多出一个输入框。条件不成立时这一行不占地方
fn followed_by(
    entry: Element,
    open: impl Fn() -> bool + 'static,
    detail: impl Fn() -> Element + 'static,
) -> Element {
    column().child(entry).child_when(open, move || {
        row()
            .justify(Justify::End)
            .margin_top(2)
            .margin_bottom(8)
            .child(detail())
    })
}

/// 分段开关：灰槽里浮起白色的当前项
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(&'static str, &'static str, T)],
    value: Signal<T>,
) -> Element {
    let mut choices = row().padding(2).gap(2).radius(8).background(hover());
    for &(slug, label, option) in options {
        let (current, pick) = (value, value);
        choices = choices.child(
            row()
                .name(format!("{id}-{slug}"))
                .style_with(
                    &segment_style(),
                    &States::new().selected(move || current.get() == option),
                )
                .align(Align::Center)
                .justify(Justify::Center)
                .focusable(true)
                .focus_ring(FocusRing::new(ink()))
                .accessibility_name(label)
                .on_click(move || pick.set(option))
                .on_key_down(move |event: &KeyEvent| {
                    if event.is("Enter") || event.is("Space") {
                        pick.set(option);
                        event.prevent_builtin(Builtin::ACTIVATE);
                        event.stop_propagation();
                    }
                })
                .child(text(label).font_size(12)),
        );
    }
    choices
}

fn segment_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .padding_x(10)
            .radius(6)
            .color(text_2())
            .background(Color::TRANSPARENT)
            .hovered(|s| s.background(hover()))
            .selected(|s| s.background(card_bg()).color(fg())),
    )
}

/// 开关：打开时是墨色，不用控件默认的蓝色
fn toggle(on: Signal<bool>, name: &'static str, label: &'static str) -> Element {
    switch(on)
        .name(name)
        .accessibility_name(label)
        .focus_ring(FocusRing::new(ink()))
        .track_style(&Style::new().checked(|style| {
            style
                .background(accent())
                .hovered(|style| style.background(accent()))
                .pressed(|style| style.background(accent()))
        }))
        .thumb_style(&Style::new().transition(Property::TranslationX, Spring::snappy()))
        .build()
}

/// 路径框右边的「选择…」选的是文件还是目录
#[derive(Clone, Copy)]
enum Picks {
    File,
    Directory,
}

/// 路径输入框，右边一个「选择…」弹系统的对话框，选好填进框里
fn picker_field(
    value: Signal<String>,
    name: &'static str,
    placeholder: &'static str,
    picks: Picks,
) -> Element {
    row()
        .align(Align::Center)
        .gap(8)
        .child(path_field(value, name, placeholder))
        .child(
            outline_button(None, "选择…")
                .name(format!("{name}-pick"))
                .height(30)
                .on_click(move || {
                    // 任务不记在这个按钮上：对话框开着时设置页可能被切走、卸载
                    aureole::detached(|| {
                        aureole::spawn_local(async move {
                            let dialog = aureole::FileDialog::new();
                            let chosen = match picks {
                                Picks::File => dialog.title("选择程序").pick_file().await,
                                Picks::Directory => dialog.title("选择目录").pick_directory().await,
                            };
                            if let Some(path) = chosen {
                                value.set(path.display().to_string());
                            }
                        })
                    });
                }),
        )
}

fn path_field(value: Signal<String>, name: &'static str, placeholder: &'static str) -> Element {
    text_field(value)
        .name(name)
        .placeholder(placeholder)
        .width(240)
        .height(30)
        .font_size(12)
        .radius(6)
        .padding_x(8)
        .background(card_bg())
        .border(1, line())
        .focus_ring(FocusRing::new(ink()))
        .style(&Style::new().focused(|style| style.border_color(ink())))
}

fn font_menu(value: Signal<FontChoice>) -> Element {
    menu(
        value,
        [
            (FontChoice::System, "系统等宽"),
            (FontChoice::Menlo, "Menlo"),
            (FontChoice::Monaco, "Monaco"),
            (FontChoice::CourierNew, "Courier New"),
        ],
        "set-font",
        "终端字体",
    )
}

fn weight_menu(value: Signal<FontWeight>) -> Element {
    menu(
        value,
        [
            (FontWeight::Light, "细"),
            (FontWeight::Regular, "常规"),
            (FontWeight::Medium, "中等"),
            (FontWeight::Semibold, "半粗"),
            (FontWeight::Bold, "粗"),
        ],
        "set-font-weight",
        "字体粗细",
    )
}

/// 下拉选择，样式统一
fn menu<V: Clone + PartialEq + 'static>(
    value: Signal<V>,
    options: impl IntoIterator<Item = (V, &'static str)>,
    name: &'static str,
    label: &'static str,
) -> Element {
    select(value, options)
        .name(name)
        .accessibility_name(label)
        .width(168)
        .focus_ring(FocusRing::new(ink()))
        .style(
            &Style::new()
                .radius(6)
                .font_size(12)
                .background(card_bg())
                .color(fg())
                .border(1, line())
                .open(|style| style.border_color(ink()))
                .focused(|style| style.border_color(ink())),
        )
        .arrow_style(&Style::new().color(text_3()))
        .list_style(
            &Style::new()
                .radius(8)
                .padding(4)
                .background(card_bg())
                .border(1, line())
                .shadow(popup_shadow()),
        )
        .option_style(
            &Style::new()
                .radius(5)
                .font_size(12)
                .focused(|style| style.background(hover()))
                .pressed(|style| style.background(hover()))
                .selected(|style| style.color(fg())),
        )
        .build()
}

/// 数字加减，右边带单位。`width` 是中间数字的宽度
fn count(
    value: Signal<f64>,
    name: &'static str,
    min: f64,
    max: f64,
    step: f64,
    width: f32,
    unit: &'static str,
) -> Element {
    row()
        .align(Align::Center)
        .gap(8)
        .child(stepper(value, name, min, max, step, 0, width))
        .child(text(unit).font_size(12).color(text_3()))
}

fn stepper(
    value: Signal<f64>,
    name: &'static str,
    min: f64,
    max: f64,
    step: f64,
    digits: usize,
    width: f32,
) -> NumberField {
    number_field(value)
        .name(name)
        .range(min, max)
        .step(step)
        .precision(digits)
        .focus_ring(FocusRing::new(ink()))
        .style(
            &Style::new()
                .radius(6)
                .background(card_bg())
                .color(fg())
                .border(1, line())
                .focused(|style| style.border_color(ink())),
        )
        .field_style(
            &Style::new()
                .width(width)
                .font_size(12)
                .background(Color::TRANSPARENT)
                .color(fg()),
        )
        .button_style(
            &Style::new()
                .color(text_2())
                .hovered(|style| style.color(fg())),
        )
}

/// 外观页顶上的样子：用现在的配色、字体、字号、行距画几行假的终端输出，设置一改它就跟着变
fn preview(form: Form) -> Element {
    let colors = move || view::colors(crate::theme::is_dark());
    let family = move || form.font.get().family().to_string();
    let sample = move |parts: Vec<(&'static str, Option<usize>)>| {
        let mut line = row();
        for (content, ansi) in parts {
            line = line.child(
                text(content)
                    .font_family(family)
                    .font_size(move || form.font_size.get() as f32)
                    .font_weight(move || form.font_weight.get().value())
                    .line_height(move || form.line_height.get() as f32)
                    .color(move || match ansi {
                        Some(index) => colors().ansi(index),
                        None => colors().foreground,
                    }),
            );
        }
        line
    };
    column()
        .name("settings-preview")
        .radius(8)
        .padding(14)
        .border(1, line())
        .background(move || colors().background)
        .child(sample(vec![
            ("licha", Some(2)),
            (" ", None),
            ("~/终端", Some(4)),
            (" % cargo test", None),
        ]))
        .child(sample(vec![("running 77 tests", None)]))
        .child(sample(vec![
            ("warning", Some(3)),
            (": unused variable", None),
        ]))
        .child(sample(vec![("error", Some(1)), (": 找不到文件", None)]))
        .child(sample(vec![
            ("test result: ", None),
            ("ok", Some(2)),
            (". 77 passed", None),
        ]))
}

fn shortcuts_page() -> Element {
    const KEYS: [(&str, &str); 12] = [
        ("新建会话", "⌘T"),
        ("左右分屏", "⌘D"),
        ("上下分屏", "⌘⇧D"),
        ("关闭当前终端（拆分过的）/ 标签；设置页里是返回", "⌘W"),
        ("命令面板", "⌘K"),
        ("收起 / 展开侧栏", "⌘B"),
        ("打开 / 退出设置", "⌘,"),
        ("切到第 1 – 8 个标签", "⌘1 – ⌘8"),
        ("切到最后一个标签", "⌘9"),
        ("上一个 / 下一个标签", "⌘⇧[  ⌘⇧]"),
        ("切换标签", "Ctrl+Tab"),
        ("粘贴", "⌘V"),
    ];
    let rows = KEYS
        .iter()
        .map(|&(label, keys)| {
            setting(
                label,
                text(keys)
                    .font_size(12)
                    .padding_x(8)
                    .padding_y(3)
                    .radius(5)
                    .background(hover()),
            )
        })
        .collect();
    settings_list(
        "快捷键",
        "Windows 和 Linux 上用 Ctrl+Shift 代替 ⌘。",
        stack(rows),
    )
}

fn about_page(form: Form, location: &str) -> Element {
    let value = |content: String| {
        text(content)
            .color(text_2())
            .max_lines(1)
            .min_width(0)
            .font_size(12)
    };
    let rows = vec![
        setting("版本", value(env!("CARGO_PKG_VERSION").to_string())),
        setting(
            "界面框架",
            outline_button(None, crate::product::FRAMEWORK)
                .name("about-framework")
                .tooltip(crate::product::FRAMEWORK_URL)
                .on_click(|| {
                    aureole::open_url(crate::product::FRAMEWORK_URL);
                }),
        ),
        setting(
            "源代码",
            outline_button(Some(Icon::Github), "GitHub")
                .name("about-github")
                .tooltip(crate::product::REPOSITORY)
                .on_click(|| {
                    aureole::open_url(crate::product::REPOSITORY);
                }),
        ),
        setting(
            "配置文件夹",
            value(location.to_string()).name("settings-location"),
        ),
        setting(
            "全部设置",
            outline_button(None, "恢复默认")
                .name("set-reset")
                .on_click(move || form.apply(&Settings::default())),
        ),
    ];
    settings_list(crate::product::NAME, "", stack(rows))
}
