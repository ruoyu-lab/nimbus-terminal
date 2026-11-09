//! 右键菜单（重命名、删除）和重命名、文件夹命名、确认关闭几个对话框；标签和侧栏的会话行共用。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::widgets::{pop, soft};
use crate::tabs::{FolderPrompt, Workspace};
use crate::theme::{accent, card_bg, fg, hover, line, popup_shadow, selected, text_3};

/// 给标签、会话行加右键菜单（重命名、删除）：右键、macOS 上 Control+点击、触屏长按时，
/// 在按下的那一点弹出。`scope` 区分标签和侧栏的行，菜单里元素的名字不会重复
pub(super) fn with_context_menu(
    element: Element,
    workspace: &Rc<Workspace>,
    scope: &'static str,
    id: u64,
) -> Element {
    let (open, at) = (signal(false), signal([0.0f32, 0.0]));
    let owner = workspace.clone();
    element
        .on_secondary_click(move |event: &ClickEvent| {
            event.stop_propagation();
            at.set(event.window_position);
            open.set(true);
        })
        .overlay(
            open,
            Overlay::new(move || context_menu(owner.clone(), open, scope, id))
                .at(at)
                .placement(Placement::bottom().start())
                .offset(2)
                .auto_focus(true)
                .on_dismiss(move |_| open.set(false)),
        )
}

fn context_menu(
    workspace: Rc<Workspace>,
    open: Signal<bool>,
    scope: &'static str,
    id: u64,
) -> Element {
    let item = |action: &'static str, label: &'static str, run: Rc<dyn Fn()>| {
        row()
            .name(format!("menu-{action}-{scope}-{id}"))
            .style(&menu_item_style())
            .align(Align::Center)
            .accessibility_role(Role::MenuItem)
            .focusable(true)
            .focus_ring(FocusRing::NONE)
            // 先关菜单再动作：删除会把挂着菜单的这一行一起卸掉
            .on_click(move || {
                open.set(false);
                run();
            })
            .child(text(label))
    };
    let (rename, delete) = (workspace.clone(), workspace);
    pop(column()
        .name(format!("menu-{scope}-{id}"))
        .width(132)
        .padding(4)
        .gap(1)
        .radius(8)
        .background(card_bg())
        .shadow(popup_shadow())
        .focus_arrows(Direction::Column)
        .child(item(
            "rename",
            "重命名",
            Rc::new(move || rename.renaming.set(Some(id))),
        ))
        .child(item(
            "delete",
            "删除",
            Rc::new(move || delete.request_close(id)),
        )))
}

/// 右键弹出一列动作。`menu_name` 是菜单自己的名字，每一项的名字是 `menu-{id}`
pub(super) struct Action {
    pub id: String,
    pub label: String,
    pub run: Rc<dyn Fn()>,
}

pub(super) fn with_actions(element: Element, menu_name: String, actions: Vec<Action>) -> Element {
    let (open, at) = (signal(false), signal([0.0f32, 0.0]));
    let actions = Rc::new(actions);
    let menu = menu_name.clone();
    element
        .on_secondary_click(move |event: &ClickEvent| {
            event.stop_propagation();
            at.set(event.window_position);
            open.set(true);
        })
        .overlay(
            open,
            Overlay::new(move || action_menu(menu.clone(), open, actions.clone()))
                .at(at)
                .placement(Placement::bottom().start())
                .offset(2)
                .auto_focus(true)
                .on_dismiss(move |_| open.set(false)),
        )
}

/// 终端里的右键菜单：复制（有选中的字才有）、粘贴、全选、清屏、拆分，拆分过的还有「关闭此分屏」。
/// 挂在终端所在的元素上；`at` 是点的位置（窗口坐标），`open` 为真时弹出。菜单每次弹出时现看有没有选中的字
pub(super) fn pane_menu_overlay(
    element: Element,
    open: Signal<bool>,
    at: Signal<[f32; 2]>,
    workspace: &Rc<Workspace>,
    tab: u64,
    pane: u64,
) -> Element {
    let owner = workspace.clone();
    element.overlay(
        open,
        Overlay::new(move || {
            let actions = pane_actions(&owner, tab, pane);
            action_menu(format!("menu-pane-{pane}"), open, Rc::new(actions))
        })
        .at(at)
        .placement(Placement::bottom().start())
        .offset(2)
        .auto_focus(true)
        .on_dismiss(move |_| open.set(false)),
    )
}

fn pane_actions(workspace: &Rc<Workspace>, tab_id: u64, pane: u64) -> Vec<Action> {
    let Some(tab) = workspace
        .tabs
        .get_untracked()
        .into_iter()
        .find(|tab| tab.id == tab_id)
    else {
        return Vec::new();
    };
    let layout = tab.layout.get_untracked();
    let Some(session) = layout.session(pane) else {
        return Vec::new();
    };
    let action = |id: &str, label: &str, run: Rc<dyn Fn()>| Action {
        id: format!("pane-{id}-{pane}"),
        label: label.to_string(),
        run,
    };
    let mut actions = Vec::new();
    // 右键点在链接上：最前面加打开和复制
    if let Some(link) = session.context_link() {
        let (noun, text) = match &link {
            crate::links::Link::Url(url) => ("链接", url.clone()),
            crate::links::Link::Path { path, .. } => ("路径", path.clone()),
        };
        let (opener, target) = (session.clone(), link);
        actions.push(action(
            "open-link",
            &format!("打开{noun}"),
            Rc::new(move || {
                opener.open_link_now(&target);
            }),
        ));
        actions.push(action(
            "copy-link",
            &format!("复制{noun}"),
            Rc::new(move || {
                aureole::clipboard::write_text(text.clone());
            }),
        ));
    }
    if session.selected_text().is_some() {
        let copy = session.clone();
        actions.push(action(
            "copy",
            "复制",
            Rc::new(move || copy.copy_selection()),
        ));
    }
    if session.last_command_output().is_some() {
        let copy = session.clone();
        actions.push(action(
            "copy-output",
            "复制上一条命令的输出",
            Rc::new(move || {
                copy.copy_last_output();
            }),
        ));
    }
    let (paste, select, clear) = (session.clone(), session.clone(), session);
    actions.push(action("paste", "粘贴", Rc::new(move || paste.paste())));
    actions.push(action(
        "select-all",
        "全选",
        Rc::new(move || select.select_all()),
    ));
    actions.push(action(
        "clear",
        "清屏",
        Rc::new(move || clear.clear_screen()),
    ));
    let (columns, rows) = (workspace.clone(), workspace.clone());
    actions.push(action(
        "split-columns",
        "左右分屏",
        Rc::new(move || columns.split(crate::pane::Axis::Columns)),
    ));
    actions.push(action(
        "split-rows",
        "上下分屏",
        Rc::new(move || rows.split(crate::pane::Axis::Rows)),
    ));
    // 拆分过才有：关掉这一个，剩下的补上它的位置
    if layout.leaves().len() > 1 {
        let close = workspace.clone();
        actions.push(action(
            "close",
            "关闭此分屏",
            Rc::new(move || close.request_close_pane(tab_id, pane)),
        ));
    }
    actions
}

fn action_menu(menu_name: String, open: Signal<bool>, actions: Rc<Vec<Action>>) -> Element {
    let mut menu = column()
        .name(menu_name)
        .width(176)
        .padding(4)
        .gap(1)
        .radius(8)
        .background(card_bg())
        .shadow(popup_shadow())
        .focus_arrows(Direction::Column);
    for action in actions.iter() {
        let (id, label, run) = (action.id.clone(), action.label.clone(), action.run.clone());
        menu = menu.child(
            row()
                .name(format!("menu-{id}"))
                .style(&menu_item_style())
                .align(Align::Center)
                .accessibility_role(Role::MenuItem)
                .focusable(true)
                .focus_ring(FocusRing::NONE)
                .on_click(move |event: &ClickEvent| {
                    event.stop_propagation();
                    open.set(false);
                    run();
                })
                .child(text(label)),
        );
    }
    pop(menu)
}

/// 新建会话的下拉（Windows）：命令提示符、Windows PowerShell、PowerShell 7 各一项，点了按那个 shell 开新标签。
/// 没装的灰着、写「未安装」，点不了。点 `element` 弹出，挂在它下面，再点一下收起
pub(super) fn with_shell_menu(element: Element, workspace: &Rc<Workspace>) -> Element {
    let open = signal(false);
    let owner = workspace.clone();
    element
        .on_click(move || open.update(|open| *open = !*open))
        .overlay(
            open,
            Overlay::new(move || shell_menu(&owner, open))
                .placement(Placement::bottom().start())
                .offset(4)
                .auto_focus(true)
                .on_dismiss(move |_| open.set(false)),
        )
}

fn shell_menu(workspace: &Rc<Workspace>, open: Signal<bool>) -> Element {
    let mut menu = column()
        .name("menu-shells")
        .width(216)
        .padding(4)
        .gap(1)
        .radius(8)
        .background(card_bg())
        .shadow(popup_shadow())
        .focus_arrows(Direction::Column);
    for shell in crate::pty::shells() {
        let item = row()
            .name(format!("menu-shell-{}", shell.id))
            .align(Align::Center)
            .justify(Justify::SpaceBetween)
            .gap(12)
            .accessibility_role(Role::MenuItem)
            .child(text(shell.name));
        menu = menu.child(if shell.installed {
            let (owner, program) = (workspace.clone(), shell.program);
            item.style(&menu_item_style())
                .focusable(true)
                .focus_ring(FocusRing::NONE)
                .on_click(move |event: &ClickEvent| {
                    event.stop_propagation();
                    open.set(false);
                    owner.new_tab_with_shell(&program);
                })
        } else {
            let file = std::path::Path::new(&shell.program)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            item.style(&missing_item_style())
                .tooltip(format!("没有找到 {file}，装好以后就能用"))
                .child(text("未安装").font_size(12))
        });
    }
    pop(menu)
}

/// 菜单里点不了的一项（没装的 shell）：灰字，悬停不变
fn missing_item_style() -> Style {
    Style::new()
        .height(30)
        .padding_x(10)
        .radius(5)
        .color(text_3())
}

fn menu_item_style() -> Style {
    soft(
        Style::new()
            .height(30)
            .padding_x(10)
            .radius(5)
            .color(fg())
            .hovered(|s| s.background(hover()))
            .focused(|s| s.background(hover())),
    )
}

/// 重命名框：居中的模态小面板，和命令面板一个样。回车确认，Esc、点外面取消，留空退回自动的名字
pub(super) fn rename_dialog(workspace: Rc<Workspace>) -> Element {
    let Some(id) = workspace.renaming.get_untracked() else {
        return column();
    };
    let name = signal(
        workspace
            .titles
            .get_untracked()
            .get(&id)
            .cloned()
            .unwrap_or_default(),
    );
    let renaming = workspace.renaming;
    pop(column()
        .name("rename")
        .width(360)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            text("重命名")
                .font_size(12)
                .color(text_3())
                .padding_x(16)
                .margin_top(12),
        )
        .child(
            text_field(name)
                .name("rename-field")
                .placeholder("会话名称")
                .background(Color::TRANSPARENT)
                .border(0, Color::TRANSPARENT)
                .focus_ring(FocusRing::NONE)
                .font_size(15)
                .padding_x(16)
                .padding_y(8)
                .on_submit(move |value: String| {
                    workspace.rename_tab(id, &value);
                    renaming.set(None);
                }),
        )
        .child(rect().height(1).background(line()))
        .child(
            text("回车确认 · Esc 取消 · 留空恢复自动名称")
                .font_size(11)
                .color(text_3())
                .padding_x(16)
                .padding_y(8),
        ))
}

/// 新建或重命名文件夹。回车确认，Esc、点外面取消；新建时留空就用默认名字
pub(super) fn folder_dialog(workspace: Rc<Workspace>) -> Element {
    let Some(prompt) = workspace.folder_prompt.get_untracked() else {
        return column();
    };
    let (title, initial) = match prompt {
        FolderPrompt::Create => ("新建文件夹", workspace.fresh_folder_name()),
        FolderPrompt::Rename(id) => (
            "重命名文件夹",
            workspace
                .folders
                .get_untracked()
                .iter()
                .find(|folder| folder.id == id)
                .map(|folder| folder.name.clone())
                .unwrap_or_default(),
        ),
    };
    let name = signal(initial);
    let editing = workspace.folder_prompt;
    pop(column()
        .name("folder-dialog")
        .width(360)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            text(title)
                .font_size(12)
                .color(text_3())
                .padding_x(16)
                .margin_top(12),
        )
        .child(
            text_field(name)
                .name("folder-field")
                .placeholder("文件夹名称")
                .background(Color::TRANSPARENT)
                .border(0, Color::TRANSPARENT)
                .focus_ring(FocusRing::NONE)
                .font_size(15)
                .padding_x(16)
                .padding_y(8)
                .on_submit(move |value: String| {
                    match prompt {
                        FolderPrompt::Create => {
                            workspace.create_folder(&value);
                        }
                        FolderPrompt::Rename(id) => workspace.rename_folder(id, &value),
                    }
                    editing.set(None);
                }),
        )
        .child(rect().height(1).background(line()))
        .child(
            text("回车确认 · Esc 取消")
                .font_size(11)
                .color(text_3())
                .padding_x(16)
                .padding_y(8),
        ))
}

/// 「连接到服务器」：填一个主机名（`~/.ssh/config` 里的名字，或者 `user@host`），回车就连。
/// 主机名不能用（空的、以 `-` 开头、带空白）就留在这里，写明原因
pub(super) fn connect_dialog(workspace: Rc<Workspace>) -> Element {
    let host = signal(String::new());
    let error = signal(None::<&'static str>);
    let prompt = workspace.connect_prompt;
    pop(column()
        .name("connect-dialog")
        .width(380)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            text("连接到服务器")
                .font_size(12)
                .color(text_3())
                .padding_x(16)
                .margin_top(12),
        )
        .child(
            text_field(host)
                .name("connect-field")
                .placeholder("prod-hk 或 user@host")
                .background(Color::TRANSPARENT)
                .border(0, Color::TRANSPARENT)
                .focus_ring(FocusRing::NONE)
                .font_size(15)
                .padding_x(16)
                .padding_y(8)
                .on_submit(move |value: String| {
                    if workspace.new_tab_ssh(&value) {
                        prompt.set(false);
                    } else {
                        error.set(Some(
                            "主机名不能用：只能是字母、数字和 . _ - @ :，不能以 - 开头",
                        ));
                    }
                }),
        )
        .child(rect().height(1).background(line()))
        .child(
            text(move || error.get().unwrap_or("回车连接 · Esc 取消"))
                .name("connect-hint")
                .font_size(11)
                .color(text_3())
                .padding_x(16)
                .padding_y(8),
        ))
}

/// 「连接到容器」：填一个容器名或 ID，回车就 `docker exec` 进去。
/// 名字不能用（空的、以 `-` 开头、带空白）就留在这里，写明原因
pub(super) fn docker_dialog(workspace: Rc<Workspace>) -> Element {
    let container = signal(String::new());
    let error = signal(None::<&'static str>);
    let prompt = workspace.docker_prompt;
    pop(column()
        .name("docker-dialog")
        .width(380)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            text("连接到容器")
                .font_size(12)
                .color(text_3())
                .padding_x(16)
                .margin_top(12),
        )
        .child(
            text_field(container)
                .name("docker-field")
                .placeholder("容器名或 ID")
                .background(Color::TRANSPARENT)
                .border(0, Color::TRANSPARENT)
                .focus_ring(FocusRing::NONE)
                .font_size(15)
                .padding_x(16)
                .padding_y(8)
                .on_submit(move |value: String| {
                    if workspace.new_tab_docker(&value) {
                        prompt.set(false);
                    } else {
                        error.set(Some(
                            "容器名不能用：只能是字母、数字和 . _ -，不能以 - 开头",
                        ));
                    }
                }),
        )
        .child(rect().height(1).background(line()))
        .child(
            text(move || error.get().unwrap_or("回车连接 · Esc 取消"))
                .name("docker-hint")
                .font_size(11)
                .color(text_3())
                .padding_x(16)
                .padding_y(8),
        ))
}

/// 关一个前台有程序在跑的标签之前问一下。回车或点「关闭」才关，Esc、点外面、点「取消」都不关
pub(super) fn close_dialog(workspace: Rc<Workspace>) -> Element {
    let Some(id) = workspace.closing.get_untracked() else {
        return column();
    };
    let name = workspace
        .titles
        .get_untracked()
        .get(&id)
        .cloned()
        .unwrap_or_default();
    let (confirm, cancel) = (workspace.clone(), workspace.clone());
    let button = |label: &'static str, id: &'static str| {
        row()
            .name(id)
            .style(&dialog_button_style())
            .align(Align::Center)
            .justify(Justify::Center)
            .accessibility_role(Role::Button)
            .focusable(true)
            .focus_ring(FocusRing::NONE)
            .child(text(label))
    };
    pop(column()
        .name("close-dialog")
        .width(360)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .padding(16)
        .gap(6)
        .child(text(format!("关闭「{name}」？")).font_size(15))
        .child(
            text("里面还有程序在运行，关掉会把它一起结束。")
                .font_size(12)
                .color(text_3()),
        )
        .child(
            row()
                .justify(Justify::End)
                .gap(8)
                .margin_top(10)
                .child(button("取消", "close-cancel").on_click(move || {
                    cancel.closing.set(None);
                }))
                .child(
                    button("关闭", "close-confirm")
                        .style(
                            &dialog_button_style()
                                .background(accent())
                                .color(Color::WHITE),
                        )
                        .on_click(move || confirm.confirm_close()),
                ),
        ))
}

/// 粘贴多行文字之前问一下：给出前几行、一共几行。回车或点「粘贴」才粘贴，Esc、点外面、点「取消」都不粘贴
pub(super) fn paste_dialog(workspace: Rc<Workspace>) -> Element {
    let Some(pending) = workspace.pending_paste.get_untracked() else {
        return column();
    };
    let lines: Vec<&str> = pending.text.lines().collect();
    let preview: String = lines
        .iter()
        .take(6)
        .map(|line| {
            let short: String = line.chars().take(60).collect();
            if line.chars().count() > 60 {
                format!("{short}…")
            } else {
                short
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let more = lines.len().saturating_sub(6);
    let (confirm, cancel) = (workspace.clone(), workspace.clone());
    let button = |label: &'static str, id: &'static str| {
        row()
            .name(id)
            .style(&dialog_button_style())
            .align(Align::Center)
            .justify(Justify::Center)
            .accessibility_role(Role::Button)
            .focusable(true)
            .focus_ring(FocusRing::NONE)
            .child(text(label))
    };
    pop(column()
        .name("paste-dialog")
        .width(420)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .padding(16)
        .gap(6)
        .child(text(format!("粘贴 {} 行文字？", lines.len())).font_size(15))
        .child(
            text("终端没有开括号粘贴，每一行都会当成命令立刻执行。")
                .font_size(12)
                .color(text_3()),
        )
        .child(
            column()
                .name("paste-preview")
                .margin_top(4)
                .padding(8)
                .radius(6)
                .background(hover())
                .child(text(preview).font_size(12).font_family("monospace"))
                .child_when(
                    move || more > 0,
                    move || {
                        text(format!("… 还有 {more} 行"))
                            .font_size(12)
                            .color(text_3())
                    },
                ),
        )
        .child(
            row()
                .justify(Justify::End)
                .gap(8)
                .margin_top(10)
                .child(button("取消", "paste-cancel").on_click(move || {
                    cancel.pending_paste.set(None);
                }))
                .child(
                    button("粘贴", "paste-confirm")
                        .style(
                            &dialog_button_style()
                                .background(accent())
                                .color(Color::WHITE),
                        )
                        .on_click(move || confirm.confirm_paste()),
                ),
        ))
}

fn dialog_button_style() -> Style {
    soft(
        Style::new()
            .height(30)
            .padding_x(14)
            .radius(6)
            .font_size(13)
            .color(fg())
            .background(hover())
            .hovered(|s| s.background(selected())),
    )
}
