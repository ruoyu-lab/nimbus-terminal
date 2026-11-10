//! 「目标」下面的文件夹，以及还没归进文件夹的会话。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::menu::{Action, with_actions, with_context_menu};
use crate::chrome::sidebar::nav::{nav_named, section_label};
use crate::chrome::widgets::{appear, soft};
use crate::icons::{Icon, icon};
use crate::tabs::{Folder, FolderDrag, FolderId, FolderPrompt, Tab, Workspace};
use crate::theme::{fg, hover, mark, selected, text_2, text_3};

/// 文件夹里面的会话，图标缩进到文件夹名字的那一列（没归进文件夹的会话的图标和文件夹的图标对齐，不缩进）
const FOLDER_SESSION_INDENT: f32 = 23.0;
/// 拖的时候所有文件夹都收成一行（28 高加 2 的间距），相邻两个文件夹的间隔就是这个
const FOLDER_STRIDE: f32 = 30.0;

/// 中间那段滚动的句柄和内容的位置：切到的会话在视口外面时靠它们滚进来
#[derive(Clone, Copy)]
pub(super) struct Scrolling {
    pub list: ScrollHandle,
    pub content: Signal<Rect>,
}

/// 文件夹，以及直接列在「目标」下的会话
pub(super) fn list(workspace: &Rc<Workspace>, scrolling: Scrolling) -> Element {
    let folders = workspace.folders;
    let (rows, loose) = (workspace.clone(), workspace.clone());
    column()
        .gap(2)
        // 没归进文件夹的会话：图标和文件夹的图标对齐，收起侧栏时也在，只剩图标。
        // WSL 的会话列在「WSL」里它的发行版下面
        .child(sessions(
            &loose,
            scrolling,
            |tab| {
                tab.folder.is_none()
                    && tab.target.remote_name().is_none()
                    && tab.wsl_distribution().is_none()
            },
            None,
        ))
        .child(column().gap(2).each(
            move || folders.get(),
            |folder: &Folder| folder.id,
            move |folder| folder_block(&rows, folder.id, scrolling),
        ))
}

/// 「WSL」（Windows）：这台机器上装了的发行版，和「容器」一个样子。一个都没装时整块不存在；
/// 侧栏挂上时（启动、从设置页回来）重新列一遍，新装的发行版也出来。
/// 点一个发行版：它已经有会话就切过去，没有就开一个；右键可以再开一个会话。发行版下面列它的会话
pub(super) fn wsl(workspace: &Rc<Workspace>, scrolling: Scrolling) -> Element {
    let (distributions, collapsed) = (workspace.distributions, workspace.collapsed);
    let (owner, refresh) = (workspace.clone(), workspace.clone());
    column()
        .on_mount(move || refresh.refresh_distributions())
        .child_when(
            move || !distributions.get().is_empty(),
            move || {
                let owner = owner.clone();
                column()
                    .name("wsl")
                    .gap(2)
                    .child(section_label(collapsed, "WSL"))
                    .child(column().gap(2).each(
                        move || distributions.get(),
                        |distribution: &String| distribution.clone(),
                        move |distribution| distribution_block(&owner, distribution, scrolling),
                    ))
            },
        )
}

fn distribution_block(
    workspace: &Rc<Workspace>,
    distribution: String,
    scrolling: Scrolling,
) -> Element {
    let (collapsed, tabs) = (workspace.collapsed, workspace.tabs);
    let (open, again, owner) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (name, menu_name) = (distribution.clone(), distribution.clone());
    let (online, belongs) = (distribution.clone(), distribution.clone());
    let row = nav_named(collapsed, Icon::Laptop, distribution.clone())
        .name(format!("wsl-{distribution}"))
        .child_when(
            move || {
                !collapsed.get()
                    && tabs
                        .get()
                        .iter()
                        .any(|tab| tab.wsl_distribution() == Some(online.as_str()))
            },
            || rect().size(7, 7).radius(4).background(mark()),
        )
        .on_click(move || {
            let existing = open
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.wsl_distribution() == Some(name.as_str()))
                .map(|tab| tab.id);
            match existing {
                Some(id) => open.select(id),
                None => open.new_tab_wsl(&name),
            }
        });
    let row = with_actions(
        row,
        format!("menu-wsl-{distribution}"),
        vec![Action {
            id: format!("new-session-wsl-{distribution}"),
            label: "新建会话".to_string(),
            // 等菜单卸掉再开，原因同文件夹菜单里的「创建会话」
            run: Rc::new(move || {
                let (workspace, distribution) = (again.clone(), menu_name.clone());
                aureole::detached(|| {
                    aureole::spawn_local(async move {
                        aureole::sleep(ms(1)).await;
                        workspace.new_tab_wsl(&distribution);
                    })
                });
            }),
        }],
    );
    column().gap(1).child(row).child_when(
        move || !collapsed.get(),
        move || {
            let belongs = belongs.clone();
            sessions(
                &owner,
                scrolling,
                move |tab| tab.wsl_distribution() == Some(belongs.as_str()),
                Some(FOLDER_SESSION_INDENT),
            )
        },
    )
}

/// 「远程」：`~/.ssh/config` 里的和手动连过的服务器。没有服务器、或者设置里关了「远程功能」时整块不存在。
/// 点一个主机：它已经有会话就切过去，没有就连上去；右键可以再开一个会话。主机下面列它的会话
pub(super) fn remote(workspace: &Rc<Workspace>, scrolling: Scrolling) -> Element {
    let (hosts, form, collapsed) = (workspace.hosts, workspace.form, workspace.collapsed);
    let owner = workspace.clone();
    column().child_when(
        move || form.remote.get() && !hosts.get().is_empty(),
        move || {
            let owner = owner.clone();
            column()
                .name("remote")
                .gap(2)
                .child(section_label(collapsed, "远程"))
                .child(column().gap(2).each(
                    move || hosts.get(),
                    |host: &String| host.clone(),
                    move |host| host_block(&owner, host, scrolling),
                ))
        },
    )
}

fn host_block(workspace: &Rc<Workspace>, host: String, scrolling: Scrolling) -> Element {
    let (collapsed, tabs) = (workspace.collapsed, workspace.tabs);
    let (open, connect, again) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (name, menu_host, owner) = (host.clone(), host.clone(), workspace.clone());
    let (online, belongs) = (host.clone(), host.clone());
    let row = nav_named(collapsed, Icon::Server, host.clone())
        .name(format!("host-{host}"))
        .child_when(
            move || {
                !collapsed.get()
                    && tabs
                        .get()
                        .iter()
                        .any(|tab| tab.target.host() == Some(online.as_str()))
            },
            || rect().size(7, 7).radius(4).background(mark()),
        )
        .on_click(move || {
            let existing = open
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.target.host() == Some(name.as_str()))
                .map(|tab| tab.id);
            match existing {
                Some(id) => open.select(id),
                None => {
                    connect.new_tab_ssh(&name);
                }
            }
        });
    let row = with_actions(
        row,
        format!("menu-host-{host}"),
        vec![Action {
            id: format!("new-session-host-{host}"),
            label: "新建会话".to_string(),
            // 等菜单卸掉再开，原因同文件夹菜单里的「创建会话」
            run: Rc::new(move || {
                let (workspace, host) = (again.clone(), menu_host.clone());
                aureole::detached(|| {
                    aureole::spawn_local(async move {
                        aureole::sleep(ms(1)).await;
                        workspace.new_tab_ssh(&host);
                    })
                });
            }),
        }],
    );
    column().gap(1).child(row).child_when(
        move || !collapsed.get(),
        move || {
            let belongs = belongs.clone();
            sessions(
                &owner,
                scrolling,
                move |tab| tab.target.host() == Some(belongs.as_str()),
                Some(FOLDER_SESSION_INDENT),
            )
        },
    )
}

/// 「容器」：运行中的和手动连过的容器。没有容器、或者设置里关了「远程功能」时整块不存在。
/// 点一个容器：它已经有会话就切过去，没有就连上去；右键可以再开一个会话。容器下面列它的会话
pub(super) fn containers(workspace: &Rc<Workspace>, scrolling: Scrolling) -> Element {
    let (containers, form, collapsed) = (workspace.containers, workspace.form, workspace.collapsed);
    let (owner, refresh) = (workspace.clone(), workspace.clone());
    column().child_when(
        move || form.remote.get() && !containers.get().is_empty(),
        move || {
            let (owner, refresh) = (owner.clone(), refresh.clone());
            column()
                .name("containers")
                .gap(2)
                // 挂上时重新列一遍：命令行里新起的容器也出来
                .on_mount(move || refresh.refresh_containers())
                .child(section_label(collapsed, "容器"))
                .child(column().gap(2).each(
                    move || containers.get(),
                    |container: &String| container.clone(),
                    move |container| container_block(&owner, container, scrolling),
                ))
        },
    )
}

fn container_block(workspace: &Rc<Workspace>, container: String, scrolling: Scrolling) -> Element {
    let (collapsed, tabs) = (workspace.collapsed, workspace.tabs);
    let (open, connect, again) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (name, menu_container, owner) = (container.clone(), container.clone(), workspace.clone());
    let (online, belongs) = (container.clone(), container.clone());
    let row = nav_named(collapsed, Icon::Box, container.clone())
        .name(format!("container-{container}"))
        .child_when(
            move || {
                !collapsed.get()
                    && tabs
                        .get()
                        .iter()
                        .any(|tab| tab.target.container() == Some(online.as_str()))
            },
            || rect().size(7, 7).radius(4).background(mark()),
        )
        .on_click(move || {
            let existing = open
                .tabs
                .get_untracked()
                .iter()
                .find(|tab| tab.target.container() == Some(name.as_str()))
                .map(|tab| tab.id);
            match existing {
                Some(id) => open.select(id),
                None => {
                    connect.new_tab_docker(&name);
                }
            }
        });
    let row = with_actions(
        row,
        format!("menu-container-{container}"),
        vec![Action {
            id: format!("new-session-container-{container}"),
            label: "新建会话".to_string(),
            // 等菜单卸掉再开，原因同主机菜单里的「新建会话」
            run: Rc::new(move || {
                let (workspace, container) = (again.clone(), menu_container.clone());
                aureole::detached(|| {
                    aureole::spawn_local(async move {
                        aureole::sleep(ms(1)).await;
                        workspace.new_tab_docker(&container);
                    })
                });
            }),
        }],
    );
    column().gap(1).child(row).child_when(
        move || !collapsed.get(),
        move || {
            let belongs = belongs.clone();
            sessions(
                &owner,
                scrolling,
                move |tab| tab.target.container() == Some(belongs.as_str()),
                Some(FOLDER_SESSION_INDENT),
            )
        },
    )
}

fn folder_block(workspace: &Rc<Workspace>, id: u64, scrolling: Scrolling) -> Element {
    let (collapsed, toggle) = (workspace.collapsed, workspace.clone());
    let (label, expanded, collapsed_mark) =
        (workspace.folders, workspace.folders, workspace.folders);
    let (drag, reorder) = (workspace.folder_drag, workspace.clone());
    let (order, expanded_drag) = (workspace.folders, workspace.folder_drag);
    // 拖的时候所有文件夹收成一行，才好按固定的间隔让位；松手后各自恢复原来的展开
    let open_now = move |folders: Signal<Vec<Folder>>| {
        expanded_drag.get().is_none() && folder_open(folders, id)
    };
    // 这个文件夹该往哪边挪一格：被拖的从上往下越过它，它往上让；从下往上越过，它往下让。
    // 被拖的这个自己不挪：它在合成层上被框架拖着，松手后框架从指针处把它落进新的格子
    let shift = move || -> f32 {
        let Some(dragged) = drag.get() else {
            return 0.0;
        };
        if dragged.id == id {
            return 0.0;
        }
        let Some(index) = order.get().iter().position(|folder| folder.id == id) else {
            return 0.0;
        };
        if dragged.from < dragged.to && index > dragged.from && index <= dragged.to {
            -FOLDER_STRIDE
        } else if dragged.to < dragged.from && index >= dragged.to && index < dragged.from {
            FOLDER_STRIDE
        } else {
            0.0
        }
    };
    // 对应磁盘目录的文件夹，悬停时看得到路径
    let path = workspace
        .folders
        .get_untracked()
        .iter()
        .find(|folder| folder.id == id)
        .and_then(|folder| folder.path.clone());
    let mut row = row()
        .name(format!("folder-{id}"))
        .style(&folder_style())
        .align(Align::Center)
        .gap(8)
        .translation_y(shift)
        .transition(Property::TranslationY, Spring::snappy())
        .on_click(move || {
            if collapsed.get_untracked() {
                toggle.collapsed.set(false);
            } else {
                toggle.toggle_folder(id);
            }
        })
        .draggable(move || FolderId(id))
        // 被拖的文件夹由框架在合成层上跟着指针走：只沿纵向
        .drag_preview(DragPreview::Lift)
        .drag_preview_axis(Direction::Column)
        .on_drag(move |event: &DragEvent| match event.phase {
            GesturePhase::Start => {
                let from = reorder.folder_index(id).unwrap_or(0);
                drag.set(Some(FolderDrag { id, from, to: from }));
            }
            // 放下时顺序已经换好、状态已经清掉；取消（Esc、拖出窗口）时落回原来的格子
            GesturePhase::End | GesturePhase::Cancel => {
                if drag.get_untracked().is_some() {
                    drag.set(None);
                }
            }
            GesturePhase::Move => {
                let Some(dragged) = drag.get_untracked() else {
                    return;
                };
                let last = reorder.folders.get_untracked().len().saturating_sub(1) as f32;
                let moved = dragged.from as f32 * FOLDER_STRIDE + event.translation[1];
                let to = (moved.clamp(0.0, last * FOLDER_STRIDE) / FOLDER_STRIDE).round() as usize;
                if dragged.to != to {
                    drag.set(Some(FolderDrag { to, ..dragged }));
                }
            }
        })
        .child(
            icon(Icon::Folder, 15.0)
                .name(format!("folder-icon-{id}"))
                .color(text_2()),
        )
        .child_when(
            move || !collapsed.get(),
            move || text(move || folder_name(label, id)).grow(1).max_lines(1),
        )
        .child_when(
            move || !collapsed.get() && open_now(expanded),
            || icon(Icon::ChevronDown, 14.0).color(text_3()),
        )
        .child_when(
            move || !collapsed.get() && !open_now(collapsed_mark),
            || icon(Icon::ChevronRight, 14.0).color(text_3()),
        );
    if let Some(path) = path {
        row = row.tooltip(path.display().to_string());
    }
    let owner = workspace.clone();
    let shown = workspace.folders;
    column()
        .gap(1)
        .child(folder_menu(row, workspace, id))
        .child_when(
            move || !collapsed.get() && open_now(shown),
            move || {
                sessions(
                    &owner,
                    scrolling,
                    move |tab| tab.folder == Some(id),
                    Some(FOLDER_SESSION_INDENT),
                )
            },
        )
}

fn folder_name(folders: Signal<Vec<Folder>>, id: u64) -> String {
    folders
        .get()
        .into_iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.name)
        .unwrap_or_default()
}

fn folder_open(folders: Signal<Vec<Folder>>, id: u64) -> bool {
    folders
        .get()
        .into_iter()
        .find(|folder| folder.id == id)
        .is_some_and(|folder| folder.open)
}

fn folder_menu(row: Element, workspace: &Rc<Workspace>, id: u64) -> Element {
    let (create, rename, delete) = (workspace.clone(), workspace.clone(), workspace.clone());
    with_actions(
        row,
        format!("menu-folder-{id}"),
        vec![
            Action {
                id: format!("new-session-folder-{id}"),
                label: "创建会话".to_string(),
                // 等菜单卸掉再开。在菜单的点击里直接开，读 shell 的任务会记在菜单上，菜单一关任务就没了
                run: Rc::new(move || {
                    let workspace = create.clone();
                    aureole::detached(|| {
                        aureole::spawn_local(async move {
                            aureole::sleep(ms(1)).await;
                            workspace.new_tab_in(id);
                        })
                    });
                }),
            },
            Action {
                id: format!("rename-folder-{id}"),
                label: "重命名".to_string(),
                run: Rc::new(move || rename.folder_prompt.set(Some(FolderPrompt::Rename(id)))),
            },
            Action {
                id: format!("delete-folder-{id}"),
                label: "删除".to_string(),
                run: Rc::new(move || delete.delete_folder(id)),
            },
        ],
    )
}

/// 「目标」下面还没被文件夹占掉的空白：右键可以创建会话（不归任何文件夹）、新建文件夹，或者添加一个磁盘目录
pub(super) fn blank(workspace: &Rc<Workspace>) -> Element {
    let prompt = workspace.folder_prompt;
    let (choose, create) = (workspace.clone(), workspace.clone());
    with_actions(
        rect().name("targets-blank").grow(1).min_height(48),
        "menu-targets".to_string(),
        vec![
            Action {
                id: "new-session".to_string(),
                label: "创建会话".to_string(),
                // 等菜单卸掉再开，原因同文件夹菜单里的「创建会话」
                run: Rc::new(move || {
                    let workspace = create.clone();
                    aureole::detached(|| {
                        aureole::spawn_local(async move {
                            aureole::sleep(ms(1)).await;
                            workspace.new_tab_loose();
                        })
                    });
                }),
            },
            Action {
                id: "new-folder".to_string(),
                label: "新建文件夹".to_string(),
                run: Rc::new(move || prompt.set(Some(FolderPrompt::Create))),
            },
            Action {
                id: "add-directory".to_string(),
                label: "添加目录…".to_string(),
                run: Rc::new(move || choose.choose_directory()),
            },
        ],
    )
}

fn folder_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .padding_x(10)
            .radius(6)
            .font_size(13)
            .color(fg())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover())),
    )
}

/// 符合条件的会话行，按开的先后。`indent` 是文件夹里的会话文字缩进多少；`None` 是直接列在「目标」下的，带图标
fn sessions(
    workspace: &Rc<Workspace>,
    scrolling: Scrolling,
    belongs: impl Fn(&Tab) -> bool + 'static,
    indent: Option<f32>,
) -> Element {
    let (tabs, owner) = (workspace.tabs, workspace.clone());
    column().gap(1).each(
        move || {
            tabs.get()
                .into_iter()
                .filter(|tab| belongs(tab))
                .collect::<Vec<_>>()
        },
        |tab| tab.id,
        move |tab| session_row(&owner, tab, scrolling, indent),
    )
}

fn session_row(
    workspace: &Rc<Workspace>,
    tab: Tab,
    scrolling: Scrolling,
    indent: Option<f32>,
) -> Element {
    let (active, titles, unseen, collapsed) = (
        workspace.active,
        workspace.titles,
        workspace.unseen,
        workspace.collapsed,
    );
    let id = tab.id;
    let select = workspace.clone();
    let Scrolling { list, content } = scrolling;
    // 这一行成了当前会话时（新开的、点标签栏切的、快捷键切的）滚进视口。
    // 等一帧再滚：新行刚挂上，位置还没量到
    let bounds = signal(Rect::default());
    Effect::new(move || {
        if active.get() == id {
            aureole::spawn_local(async move {
                aureole::sleep(ms(20)).await;
                let (row, content) = (bounds.get_untracked(), content.get_untracked());
                let (top, offset, shown) =
                    (row.y - content.y, list.offset_y(), list.viewport_size()[1]);
                let bottom = top + row.height;
                if top < offset {
                    list.scroll_to(0.0, top);
                } else if bottom > offset + shown {
                    list.scroll_to(0.0, bottom - shown);
                }
            });
        }
    });
    let title = move || titles.get().get(&id).cloned().unwrap_or_default();
    let mut session = row()
        .name(format!("session-{id}"))
        .report_bounds(bounds)
        .style_with(
            &session_style(),
            &States::new().selected(move || active.get() == id),
        )
        .align(Align::Center)
        .on_click(move || select.select(id));
    session = match indent {
        // 文件夹里的：图标缩进到文件夹名字的那一列，看得出是它的子项；行和别的侧栏行一样宽
        Some(indent) => session
            .padding_x(10)
            .gap(8)
            .child(
                icon(Icon::Terminal, 15.0)
                    .name(format!("session-icon-{id}"))
                    .color(text_2())
                    .margin_left(indent),
            )
            .child(text(title).grow(1).max_lines(1)),
        // 直接列在「目标」下的：图标和文件夹的图标同一列、同样大，名字和文件夹的名字对齐；
        // 收起侧栏时只剩图标（名字放进提示里）
        None => session
            .padding_x(move || if collapsed.get() { 0.0 } else { 10.0 })
            .gap(8)
            .justify(move || {
                if collapsed.get() {
                    Justify::Center
                } else {
                    Justify::Start
                }
            })
            .tooltip(title)
            .child(
                icon(Icon::Terminal, 15.0)
                    .name(format!("session-icon-{id}"))
                    .color(text_2()),
            )
            .child_when(
                move || !collapsed.get(),
                move || text(title).grow(1).max_lines(1),
            ),
    };
    let session = session.child_when(
        move || !collapsed.get() && unseen.get().contains(&id),
        || rect().size(6, 6).radius(3).background(mark()),
    );
    appear(with_context_menu(session, workspace, "session", id))
}

fn session_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .radius(6)
            .font_size(12)
            .color(text_2())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover()).color(fg()))
            .selected(|style| style.background(selected()).color(fg())),
    )
}
