//! 命令面板（⌘K）：搜索会话和命令。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::widgets::{pop, soft};
use crate::icons::{Icon, icon};
use crate::tabs::Workspace;
use crate::theme::{card_bg, fg, hover, line, popup_shadow, text_2, text_3};

/// 面板里的一条命令
#[derive(Clone)]
struct Command {
    id: String,
    label: String,
    hint: &'static str,
    run: Rc<dyn Fn()>,
}

fn commands(workspace: &Rc<Workspace>) -> Vec<Command> {
    let command = |id: &str, label: &str, hint: &'static str, run: Rc<dyn Fn()>| Command {
        id: id.to_string(),
        label: label.to_string(),
        hint,
        run,
    };
    let find = workspace.clone();
    let (previous, next, copy_output) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (new_tab, close, toggle, settings, choose, columns, rows) = (
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
    );
    let mut list = vec![
        command(
            "cmd-new-tab",
            "新建会话",
            "⌘T",
            Rc::new(move || new_tab.new_tab()),
        ),
        command(
            "cmd-close-tab",
            "关闭当前会话",
            "⌘W",
            Rc::new(move || close.close_active()),
        ),
        command(
            "cmd-find",
            "在终端中查找",
            "⌘F",
            Rc::new(move || find.find_in_terminal()),
        ),
        command(
            "cmd-previous-command",
            "跳到上一条命令",
            "⌘↑",
            Rc::new(move || {
                if let Some(session) = previous.focused_session() {
                    session.jump_prompt(true);
                }
            }),
        ),
        command(
            "cmd-next-command",
            "跳到下一条命令",
            "⌘↓",
            Rc::new(move || {
                if let Some(session) = next.focused_session() {
                    session.jump_prompt(false);
                }
            }),
        ),
        command(
            "cmd-copy-output",
            "复制上一条命令的输出",
            "",
            Rc::new(move || {
                if let Some(session) = copy_output.focused_session() {
                    session.copy_last_output();
                }
            }),
        ),
        command(
            "cmd-toggle-sidebar",
            "收起或展开侧栏",
            "⌘B",
            Rc::new(move || toggle.toggle_sidebar()),
        ),
        command(
            "cmd-split-columns",
            "左右分屏",
            "⌘D",
            Rc::new(move || columns.split(crate::pane::Axis::Columns)),
        ),
        command(
            "cmd-split-rows",
            "上下分屏",
            "⌘⇧D",
            Rc::new(move || rows.split(crate::pane::Axis::Rows)),
        ),
        command(
            "cmd-choose-directory",
            "选择目录…",
            "",
            Rc::new(move || choose.choose_directory()),
        ),
        command(
            "cmd-settings",
            "打开设置",
            "⌘,",
            Rc::new(move || settings.open_settings()),
        ),
    ];
    if workspace.form.remote.get_untracked() {
        let connect = workspace.clone();
        list.push(command(
            "cmd-connect",
            "连接到服务器…",
            "",
            Rc::new(move || connect.connect_prompt.set(true)),
        ));
        let docker = workspace.clone();
        list.push(command(
            "cmd-docker",
            "连接到容器…",
            "",
            Rc::new(move || docker.docker_prompt.set(true)),
        ));
    }
    // 只有当前会话的目标带面板（连到服务器）时才有这一条
    if workspace.has_panels() {
        let panel = workspace.clone();
        list.push(command(
            "cmd-toggle-panel",
            "显示或隐藏右侧面板",
            "",
            Rc::new(move || panel.toggle_panel()),
        ));
    }
    let titles = workspace.titles.get_untracked();
    for tab in workspace.tabs.get_untracked() {
        let select = workspace.clone();
        let title = titles.get(&tab.id).cloned().unwrap_or_default();
        list.push(command(
            &format!("cmd-session-{}", tab.id),
            &format!("切换到会话：{title}"),
            "",
            Rc::new(move || select.select(tab.id)),
        ));
    }
    list
}

/// 命令面板：搜索框加一列命令，打字过滤，回车执行第一条，点哪条执行哪条
pub(super) fn view(workspace: Rc<Workspace>) -> Element {
    let query = signal(String::new());
    let all = commands(&workspace);
    let filter = {
        let all = all.clone();
        let history_owner = workspace.clone();
        move || {
            let typed = query.get();
            let needle = typed.to_lowercase();
            let mut found: Vec<Command> = all
                .iter()
                .filter(|command| {
                    needle.is_empty() || command.label.to_lowercase().contains(&needle)
                })
                .cloned()
                .collect();
            // 打了两个字以上：再从跑过的命令里找，选中就写在当前终端的提示符处（不按回车）
            if typed.trim().chars().count() >= 2 {
                for (index, command) in history_owner
                    .history_matches(typed.trim(), 8)
                    .into_iter()
                    .enumerate()
                {
                    let target = history_owner.clone();
                    let text = command.clone();
                    found.push(Command {
                        id: format!("cmd-history-{index}"),
                        label: format!("运行过：{command}"),
                        hint: "",
                        run: Rc::new(move || {
                            if let Some(session) = target.focused_session() {
                                session.insert_command(&text);
                                session.request_focus();
                            }
                        }),
                    });
                }
            }
            found
        }
    };
    let (first, owner) = (filter.clone(), workspace.clone());
    let open = workspace.palette_open;
    pop(column()
        .name("palette")
        .width(560)
        .radius(10)
        .background(card_bg())
        .shadow(popup_shadow())
        .child(
            row()
                .align(Align::Center)
                .gap(10)
                .padding_x(16)
                .padding_y(10)
                .color(text_2())
                .child(icon(Icon::Search, 17.0))
                .child(
                    text_field(query)
                        .placeholder("搜索命令、会话和跑过的命令…")
                        .grow(1)
                        // 框和焦点环都去掉：整块面板就是输入区，不要再套一层框
                        .background(Color::TRANSPARENT)
                        .border(0, Color::TRANSPARENT)
                        .focus_ring(FocusRing::NONE)
                        .font_size(15)
                        .on_submit(move |_| {
                            if let Some(command) = first().first() {
                                (command.run)();
                            }
                            open.set(false);
                        }),
                ),
        )
        .child(rect().height(1).background(line()))
        .child(column().padding(6).gap(2).each(
            filter,
            |command| format!("{}|{}", command.id, command.label),
            move |command| {
                let owner = owner.clone();
                row()
                    .name(command.id.clone())
                    .style(&palette_row_style())
                    .align(Align::Center)
                    .on_click(move || {
                        (command.run)();
                        owner.palette_open.set(false);
                    })
                    .child(text(command.label.clone()).grow(1).max_lines(1))
                    .child(text(command.hint).font_size(11).color(text_3()))
            },
        )))
}

fn palette_row_style() -> Style {
    soft(
        Style::new()
            .height(34)
            .padding_x(12)
            .radius(6)
            .color(fg())
            .hovered(|s| s.background(hover())),
    )
}
