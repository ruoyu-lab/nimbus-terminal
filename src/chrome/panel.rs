//! 右侧面板：当前会话的目标带什么面板（监控、文件）就显示什么。本机没有面板，这一块根本不存在。
//! 它是一张和主卡片一样的浮起来的卡片，左边那条空隙可以按住左右拖动改宽度。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::menu::{Action, with_actions};
use crate::chrome::widgets::{appear, icon_button, outline_button, soft};
use crate::chrome::{GUTTER, PANEL_MAX, PANEL_MIN};
use crate::files::{self, Entry, FilesState, Kind, TransferState};
use crate::icons::{Icon, icon};
use crate::monitor::{self, Monitor, Status};
use crate::tabs::RemotePath;
use crate::tabs::Workspace;
use crate::target::PanelKind;
use crate::theme::{card_bg, card_shadow, fg, hover, line, mark, selected, text_2, text_3};

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let (shown, owner) = (workspace.clone(), workspace.clone());
    let width = workspace.panel_width;
    // 条件不成立时整块都不存在（不占宽度）；每次显示都重新建，开关几次都没事
    row().min_height(0).child_when(
        move || shown.panel_shown(),
        move || {
            // 展开：挂上时宽度从 0 弹到目标宽度；收起：退场时弹回 0（`exit`），期间这一块还在，主卡片跟着慢慢变宽。
            // 拖宽度时直接跟手（拖动那边用 `without_animation` 改宽度），不被弹簧拖着
            let opened = signal(false);
            row()
                .name("panel")
                .min_height(0)
                .shrink(0)
                .clip(true)
                .width(move || px(if opened.get() { width.get() } else { 0.0 }))
                .transition(Property::Width, Spring::snappy())
                .opacity(move || if opened.get() { 1.0 } else { 0.0 })
                .transition(Property::Opacity, Tween::new(ms(160)).ease_out())
                .on_mount(move || opened.set(true))
                .exit(Property::Width, 0.0)
                .exit(Property::Opacity, 0.0)
                .child(resize_handle(width))
                // 里面的卡片宽度固定（面板宽度减去左边的拖动条和右边的留白），外层一边变宽一边裁：动画中文字不会被挤得换行
                .child(card(&owner, width))
        },
    )
}

fn card(workspace: &Rc<Workspace>, width: Signal<f32>) -> Element {
    let (kind, owner) = (workspace.panel_tab, workspace.clone());
    let (pages, monitor_owner, files_owner) =
        (workspace.clone(), workspace.clone(), workspace.clone());
    column()
        .name("panel-card")
        .shrink(0)
        .width(move || px((width.get() - 2.0 * GUTTER).max(0.0)))
        .min_width(0)
        .min_height(0)
        .margin_right(GUTTER)
        .margin_bottom(GUTTER)
        .radius(10)
        .background(card_bg())
        .shadow(card_shadow())
        .clip(true)
        .child(page_tabs(&owner))
        .child_when(
            move || page(&pages, kind) == PanelKind::Monitor,
            move || monitor_page(&monitor_owner),
        )
        .child_when(
            move || page(&owner, kind) == PanelKind::Files,
            move || files_page(&files_owner),
        )
}

/// 现在显示哪一页：用户选的，没有这一页（换了目标）就退到第一页
fn page(workspace: &Rc<Workspace>, kind: Signal<PanelKind>) -> PanelKind {
    let panels = workspace.panels();
    let wanted = kind.get();
    if panels.contains(&wanted) {
        wanted
    } else {
        panels.first().copied().unwrap_or(wanted)
    }
}

fn page_tabs(workspace: &Rc<Workspace>) -> Element {
    let (strip, workspace) = (workspace.clone(), workspace.clone());
    let kind = workspace.panel_tab;
    row()
        .height(38)
        .padding_x(10)
        .align(Align::Center)
        .gap(2)
        .each(
            move || strip.panels(),
            |panel| *panel,
            move |panel| {
                let owner = workspace.clone();
                let current = workspace.clone();
                appear(
                    row()
                        .name(format!("panel-tab-{panel:?}").to_lowercase())
                        .style_with(
                            &tab_style(),
                            &States::new().selected(move || page(&current, kind) == panel),
                        )
                        .align(Align::Center)
                        .on_click(move || owner.panel_tab.set(panel))
                        .child(text(panel.label())),
                )
            },
        )
}

fn tab_style() -> Style {
    soft(
        Style::new()
            .height(26)
            .padding_x(10)
            .radius(6)
            .font_size(12)
            .color(text_2())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover()).color(fg()))
            .selected(|style| style.background(selected()).color(fg())),
    )
}

/// 面板左边的拖动条（正好是两张卡片之间的空隙）：指针变成左右箭头，按住左右拖就改面板宽度。
/// 按下时记下指针和面板宽度，往左拖指针面板变宽，不会在按下的一瞬间跳
fn resize_handle(width: Signal<f32>) -> Element {
    let grab = signal(None::<(f32, f32)>);
    let resize = move |event: &PointerEvent| {
        if let Some((start, width_then)) = grab.get_untracked() {
            aureole::without_animation(|| {
                width.set(
                    (width_then - (event.window_position[0] - start)).clamp(PANEL_MIN, PANEL_MAX),
                )
            });
        }
    };
    rect()
        .name("panel-resize")
        .width(GUTTER)
        .shrink(0)
        .cursor(Cursor::ResizeHorizontal)
        .on_pointer_down(move |event: &PointerEvent| {
            event.capture_pointer();
            event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
            grab.set(Some((event.window_position[0], width.get_untracked())));
        })
        .on_pointer_move(resize)
        .on_pointer_up(move || grab.set(None))
        .on_pointer_cancel(move || grab.set(None))
}

// ───────────────────────── 监控页 ─────────────────────────

/// 取数的间隔
const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// 监控页：挂上就取一次，之后每 2 秒取一次，页面卸掉（换页、收起面板、换成本机标签）时定时器跟着停，不白白占连接
fn monitor_page(workspace: &Rc<Workspace>) -> Element {
    let (poller, first) = (workspace.clone(), workspace.clone());
    let (monitors, active) = (workspace.monitors, workspace.active);
    let current = move || {
        monitors
            .get()
            .get(&active.get())
            .cloned()
            .unwrap_or_default()
    };
    let (status, body) = (current, current);
    column()
        .name("monitor")
        .grow(1)
        .min_height(0)
        .padding(14)
        .gap(14)
        .on_mount(move || {
            first.poll_monitor();
            // 换了标签（还在监控页上）：立刻取一次，不等下一个 2 秒
            let _ = aureole::every(POLL_EVERY, move || poller.poll_monitor());
        })
        .child(status_line(status))
        .child_when(move || body().latest.is_some(), move || stats(body))
}

/// 顶上一行：连着时写运行时间（这个目标有运行时间才写），没连着、不支持、出错时写明白
fn status_line(current: impl Fn() -> Monitor + Clone + 'static) -> Element {
    let message = move || -> (String, bool) {
        let monitor = current();
        match monitor.status {
            Status::Waiting => ("正在读取…".to_string(), false),
            Status::Live => (
                monitor
                    .latest
                    .and_then(|snapshot| snapshot.uptime)
                    .map_or_else(
                        || "已连接".to_string(),
                        |seconds| format!("运行了 {}", monitor::uptime(seconds)),
                    ),
                true,
            ),
            Status::NotConnected => ("未连接：终端连上之后这里才有数据。".to_string(), false),
            Status::Unsupported => ("暂不支持监控（需要 Linux）。".to_string(), false),
            Status::Failed(reason) => (format!("读取失败：{reason}"), false),
        }
    };
    let (text_message, dot_message) = (message.clone(), message);
    row()
        .align(Align::Center)
        .gap(8)
        .child_when(
            move || dot_message().1,
            || rect().size(7, 7).radius(4).background(mark()).shrink(0),
        )
        .child(
            text(move || text_message().0)
                .name("monitor-status")
                .font_size(12)
                .color(text_2()),
        )
}

/// 几块数字：CPU（带曲线）、内存；磁盘、负载这个目标取到才有（容器没有按容器算的负载，不显示错的数字）
fn stats(current: impl Fn() -> Monitor + Clone + 'static) -> Element {
    let (cpu, memory, disk, load) = (
        current.clone(),
        current.clone(),
        current.clone(),
        current.clone(),
    );
    let (disk_shown, load_shown) = (current.clone(), current.clone());
    let history = current;
    column()
        .gap(16)
        .child(block(
            "CPU",
            move || {
                cpu()
                    .latest
                    .and_then(|snapshot| snapshot.cpu)
                    .map_or("…".to_string(), |percent| format!("{percent:.0}%"))
            },
            column().gap(8).child(sparkline(history)),
        ))
        .child(block(
            "内存",
            {
                let memory = memory.clone();
                move || {
                    usage_label(&memory(), |snapshot| {
                        (snapshot.mem_used, snapshot.mem_total)
                    })
                }
            },
            bar(memory, |snapshot| (snapshot.mem_used, snapshot.mem_total))
                .name("monitor-memory-bar"),
        ))
        .child_when(
            move || {
                disk_shown()
                    .latest
                    .and_then(|snapshot| snapshot.disk)
                    .is_some()
            },
            move || {
                let (value_disk, bar_disk) = (disk.clone(), disk.clone());
                block(
                    "磁盘 /",
                    move || {
                        usage_label(&value_disk(), |snapshot| {
                            snapshot.disk.map_or((0, 0), |(total, used)| (used, total))
                        })
                    },
                    bar(bar_disk, |snapshot| {
                        snapshot.disk.map_or((0, 0), |(total, used)| (used, total))
                    })
                    .name("monitor-disk-bar"),
                )
            },
        )
        .child_when(
            move || {
                load_shown()
                    .latest
                    .and_then(|snapshot| snapshot.load)
                    .is_some()
            },
            move || {
                let load = load.clone();
                block(
                    "负载",
                    move || {
                        load()
                            .latest
                            .and_then(|snapshot| snapshot.load)
                            .map_or("…".to_string(), |load| {
                                format!("{:.2}  {:.2}  {:.2}", load[0], load[1], load[2])
                            })
                    },
                    column().child(text("1、5、15 分钟").font_size(11).color(text_3())),
                )
            },
        )
}

fn usage_label(monitor: &Monitor, pick: impl Fn(&monitor::Snapshot) -> (u64, u64)) -> String {
    monitor.latest.as_ref().map_or("…".to_string(), |snapshot| {
        let (used, total) = pick(snapshot);
        let percent = if total == 0 {
            0.0
        } else {
            used as f64 / total as f64 * 100.0
        };
        format!(
            "{} / {}  {percent:.0}%",
            monitor::size(used),
            monitor::size(total)
        )
    })
}

/// 一块：左上小标题，右上数字，下面是图
fn block(title: &'static str, value: impl Fn() -> String + 'static, graphic: Element) -> Element {
    column()
        .gap(8)
        .child(
            row()
                .align(Align::Center)
                .child(text(title).font_size(12).color(text_2()).grow(1))
                .child(text(value).font_size(12).color(fg()).tabular_numbers(true)),
        )
        .child(graphic)
}

/// 进度条：灰色的槽，里面一段深灰，宽度跟着比例。超过九成变成强调的深色，一眼看得出快满了
fn bar(
    current: impl Fn() -> Monitor + 'static,
    pick: impl Fn(&monitor::Snapshot) -> (u64, u64) + 'static,
) -> Element {
    let share = Rc::new(move || -> f32 {
        current().latest.map_or(0.0, |snapshot| {
            let (used, total) = pick(&snapshot);
            if total == 0 {
                0.0
            } else {
                (used as f32 / total as f32).clamp(0.0, 1.0)
            }
        })
    });
    let (fill, tint) = (share.clone(), share);
    row()
        .height(6)
        .radius(3)
        .background(hover())
        .clip(true)
        .child(
            rect()
                .height(6)
                .radius(3)
                .width(move || pct(fill() * 100.0))
                .transition(Property::Width, Tween::new(ms(300)).ease_out())
                .background(move || if tint() > 0.9 { fg() } else { text_2() }),
        )
}

/// 曲线的高度
const SPARK_HEIGHT: f32 = 56.0;

/// 曲线上的点：从左边开始往右长，每个采样占固定的宽度，攒满两分钟（`HISTORY` 个）之后整条往左滚。
/// 以前是靠右放的：刚连上的头一分钟，只有最右边一小截线，看起来就像没有。
/// 一个点画不成线，补成一条平线；0% 在底、100% 在顶，上下各留一点边不被裁掉
fn spark_points(history: &[f32], width: f32, height: f32) -> Vec<[f32; 2]> {
    let step = width / (monitor::HISTORY - 1) as f32;
    let y = |percent: f32| height - 2.0 - percent.clamp(0.0, 100.0) / 100.0 * (height - 4.0);
    let mut points: Vec<[f32; 2]> = history
        .iter()
        .enumerate()
        .map(|(index, percent)| [index as f32 * step, y(*percent)])
        .collect();
    match points.len() {
        0 => vec![[0.0, y(0.0)], [width, y(0.0)]],
        1 => {
            let level = points[0][1];
            vec![[0.0, level], [step, level]]
        }
        _ => {
            points.dedup_by(|a, b| a[0] == b[0]);
            points
        }
    }
}

/// 曲线下面的填充：沿着曲线走一遍，再从最后一点落到底边、回到起点
fn spark_area(points: &[[f32; 2]], height: f32) -> VectorPath {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return VectorPath::polyline([]);
    };
    let mut builder = VectorPath::builder().move_to(first[0], first[1]);
    for point in &points[1..] {
        builder = builder.line_to(point[0], point[1]);
    }
    builder
        .line_to(last[0], height)
        .line_to(first[0], height)
        .close()
        .build()
}

/// CPU 的曲线：最近两分钟，0 到 100% 铺满高度，曲线下面淡淡地填一层。宽度跟着面板走
fn sparkline(current: impl Fn() -> Monitor + Clone + 'static) -> Element {
    let bounds = signal(Rect::default());
    let width = move || {
        let measured = bounds.get().width;
        if measured > 1.0 { measured } else { 240.0 }
    };
    let (line_history, area_history) = (current.clone(), current);
    let (line_width, area_width) = (width, width);
    row()
        .height(SPARK_HEIGHT)
        .radius(6)
        .background(hover())
        .clip(true)
        .report_bounds(bounds)
        // 填充和线叠在同一块地方：线在上面
        .child(
            path(move || {
                spark_area(
                    &spark_points(&area_history().cpu_history, area_width(), SPARK_HEIGHT),
                    SPARK_HEIGHT,
                )
            })
            .name("monitor-cpu-area")
            .position(0, 0)
            .size(move || px(width()), SPARK_HEIGHT)
            .color(text_2().with_alpha(0.14)),
        )
        .child(
            path(move || {
                VectorPath::polyline(spark_points(
                    &line_history().cpu_history,
                    line_width(),
                    SPARK_HEIGHT,
                ))
            })
            .name("monitor-cpu-line")
            .position(0, 0)
            .size(move || px(width()), SPARK_HEIGHT)
            .stroke(Stroke::new(1.5).round())
            .color(text_2()),
        )
}

// ───────────────────────── 文件页 ─────────────────────────

/// 文件页：路径栏（上一级、路径、刷新、显示隐藏）、目录里的条目（点文件夹进去，点右边的箭头下载）、
/// 传输记录、上传。文件从访达拖进来也能上传。挂上时读一次目录，没连上时每 2 秒再试（终端连上后自己出来）
fn files_page(workspace: &Rc<Workspace>) -> Element {
    let (files, active) = (workspace.files, workspace.active);
    let current = move || files.get().get(&active.get()).cloned().unwrap_or_default();
    // 拖着文件在面板上时整块淡淡地亮一下，提示可以放
    let over = signal(false);
    let (ensure, retry, drop_owner) = (workspace.clone(), workspace.clone(), workspace.clone());
    // 换了标签（还在文件页上）也要读：副作用跟着当前标签
    Effect::new(move || {
        let _ = active.get();
        ensure.files_ensure_loaded();
    });
    let (header_state, body_state, transfers_state) = (current, current, current);
    column()
        .name("files")
        .grow(1)
        .min_height(0)
        .background(move || {
            if over.get() {
                hover()
            } else {
                Color::TRANSPARENT
            }
        })
        .transition(Property::Background, Tween::new(ms(120)).ease_out())
        .on_mount(move || {
            let _ = aureole::every(std::time::Duration::from_secs(2), move || {
                retry.files_ensure_loaded()
            });
        })
        .on_file_hover(move |hovering: bool| over.set(hovering))
        .on_file_drop(move |event: &FileDropEvent| {
            over.set(false);
            drop_owner.files_upload(event.paths.clone());
        })
        .child(files_header(workspace, header_state))
        .child(rect().height(1).background(line()))
        .child(files_body(workspace, body_state))
        .child(files_footer(workspace, transfers_state))
}

fn files_header(
    workspace: &Rc<Workspace>,
    current: impl Fn() -> FilesState + Clone + 'static,
) -> Element {
    let (up, refresh, hidden) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (path, toggled) = (current.clone(), current);
    // 点路径变成输入框：回车去那个路径，Esc、点别处取消
    let (editing, draft) = (signal(false), signal(String::new()));
    let (shown, go) = (path.clone(), workspace.clone());
    row()
        .height(38)
        .padding_x(10)
        .align(Align::Center)
        .gap(2)
        .child(
            icon_button(Icon::ArrowUp, 15.0)
                .name("files-up")
                .tooltip("上一级")
                .on_click(move || up.files_up()),
        )
        .child_when(
            move || !editing.get(),
            move || {
                let shown = shown.clone();
                let current = shown.clone();
                row()
                    .name("files-path-button")
                    .style(&path_style())
                    .margin_x(2)
                    .grow(1)
                    .min_width(0)
                    .align(Align::Center)
                    .cursor(Cursor::Text)
                    .tooltip("点击输入路径")
                    .on_click(move || {
                        // 没读到目录（还在读、没连上）时没有路径可改
                        if let Some(path) = current().path {
                            draft.set(path);
                            editing.set(true);
                        }
                    })
                    .child(
                        text(move || shown().path.unwrap_or_else(|| "…".to_string()))
                            .name("files-path")
                            .font_size(12)
                            .max_lines(1)
                            .min_width(0),
                    )
            },
        )
        .child_when(
            move || editing.get(),
            move || {
                let focus = FocusHandle::new();
                let go = go.clone();
                text_field(draft)
                    .name("files-path-field")
                    .focus_handle(focus)
                    // 没有边框：一块淡灰的底就够看出是输入框（焦点边框是框架默认的蓝色，不要）
                    .background(hover())
                    .border(0, Color::TRANSPARENT)
                    .focus_ring(FocusRing::NONE)
                    .placeholder("/ 开头的路径，或者 ~")
                    .grow(1)
                    .min_width(0)
                    .height(28)
                    .font_size(12)
                    .radius(6)
                    .margin_x(2)
                    .on_mount(move || focus.focus())
                    // 点别处（失去焦点）就算取消
                    .on_focus(move |focused: bool| {
                        if !focused {
                            editing.set(false);
                        }
                    })
                    .on_key_down(move |event: &KeyEvent| {
                        if matches!(event.key, Key::Named(NamedKey::Escape)) {
                            editing.set(false);
                            event.stop_propagation();
                        }
                    })
                    .on_submit(move |value: String| {
                        editing.set(false);
                        go.files_go(&value);
                    })
            },
        )
        .child(
            icon_button(Icon::Refresh, 15.0)
                .name("files-refresh")
                .tooltip("刷新")
                .on_click(move || refresh.files_refresh()),
        )
        .child(
            row()
                .name("files-hidden")
                .style_with(
                    &chip_style(),
                    &States::new().selected(move || toggled().show_hidden),
                )
                .shrink(0)
                .align(Align::Center)
                .tooltip("显示或隐藏以点开头的文件")
                .on_click(move || hidden.files_toggle_hidden())
                .child(text("显示隐藏").font_size(11).max_lines(1)),
        )
}

/// 路径栏：平时是一行字，悬停时淡淡一层底色，提示可以点
fn path_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .padding_x(6)
            .radius(6)
            .color(fg())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover())),
    )
}

/// 切换隐藏文件的小按钮：打开时（显示隐藏文件）用选中的底色
fn chip_style() -> Style {
    soft(
        Style::new()
            .height(24)
            .padding_x(8)
            .radius(6)
            .color(text_2())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover()).color(fg()))
            .selected(|style| style.background(selected()).color(fg())),
    )
}

fn files_body(
    workspace: &Rc<Workspace>,
    current: impl Fn() -> FilesState + Clone + 'static,
) -> Element {
    let (message, list, banner) = (current.clone(), current.clone(), current);
    let owner = workspace.clone();
    column()
        .grow(1)
        .min_height(0)
        // 已经读到目录之后又断线、出错：列表留着，上面一行小字说明（不然看着像没事）
        .child_when(
            {
                let banner = banner.clone();
                move || stale_note(&banner()).is_some()
            },
            move || {
                let banner = banner.clone();
                text(move || stale_note(&banner()).unwrap_or_default())
                    .name("files-banner")
                    .font_size(11)
                    .color(text_2())
                    .padding_x(14)
                    .padding_y(6)
            },
        )
        .child_when(
            {
                let message = message.clone();
                move || notice(&message()).is_some()
            },
            move || {
                let message = message.clone();
                column()
                    .grow(1)
                    .align(Align::Center)
                    .justify(Justify::Center)
                    .padding(16)
                    .child(
                        text(move || notice(&message()).unwrap_or_default())
                            .name("files-notice")
                            .font_size(12)
                            .color(text_2())
                            .text_align(TextAlign::Center),
                    )
            },
        )
        .child_when(
            {
                let list = list.clone();
                move || notice(&list()).is_none()
            },
            move || {
                let (rows, owner) = (list.clone(), owner.clone());
                let blank = blank_actions(&owner);
                with_actions(
                    scroll_y(column().padding_x(6).padding_y(4).gap(1).each(
                        move || rows().visible(),
                        |entry: &Entry| entry.name.clone(),
                        move |entry| entry_row(&owner, entry),
                    ))
                    .name("files-list")
                    .grow(1)
                    .min_height(0),
                    "menu-files-blank".to_string(),
                    blank,
                )
            },
        )
}

/// 列表还在、但最近一次读失败了：写一句
fn stale_note(state: &FilesState) -> Option<String> {
    use files::Status;
    state.path.as_ref()?;
    match &state.status {
        Status::NotConnected => Some("终端的连接断了，列表可能不是最新的。".to_string()),
        Status::Failed(reason) => Some(format!("读取失败：{reason}")),
        _ => None,
    }
}

/// 没有条目可显示时写什么：读取中、没连上、不支持、出错、空目录。有条目时是 `None`
fn notice(state: &FilesState) -> Option<String> {
    use files::Status;
    match &state.status {
        Status::Idle | Status::Loading if state.path.is_none() => Some("正在读取…".to_string()),
        Status::NotConnected if state.path.is_none() => {
            Some("未连接：终端连上之后这里才有文件。".to_string())
        }
        Status::Unsupported => {
            Some("暂不支持文件页（需要 GNU 的 find，一般的 Linux 都有）。".to_string())
        }
        Status::Failed(reason) if state.path.is_none() => Some(format!("读取失败：{reason}")),
        _ if state.path.is_some() && state.visible().is_empty() => Some("空目录".to_string()),
        _ => None,
    }
}

fn entry_row(workspace: &Rc<Workspace>, entry: Entry) -> Element {
    let (open, download, dragged) = (workspace.clone(), workspace.clone(), workspace.clone());
    let (name, download_name, drag_name) =
        (entry.name.clone(), entry.name.clone(), entry.name.clone());
    let directory = entry.kind == Kind::Dir;
    let glyph = if directory { Icon::Folder } else { Icon::File };
    let mut line = row()
        .name(format!("file-{}", entry.name))
        .style(&entry_style())
        .align(Align::Center)
        .gap(8)
        // 拖到左边的终端里，就是把这个条目的绝对路径写进去（带控制字符的名字写不进，终端那边会拒绝）
        .draggable(move || RemotePath(dragged.files_full_path(&drag_name).unwrap_or_default()))
        .child(icon(glyph, 15.0).color(text_2()).shrink(0))
        .child(
            text(entry.name.clone())
                .font_size(12)
                .grow(1)
                .min_width(0)
                .max_lines(1),
        );
    if entry.kind == Kind::File {
        line = line.child(
            text(files::size(entry.size))
                .font_size(11)
                .color(text_3())
                .tabular_numbers(true)
                .shrink(0),
        );
    }
    if entry.kind != Kind::Other {
        line = line.child(
            icon_button(Icon::Download, 14.0)
                .name(format!("download-{}", entry.name))
                .tooltip("下载到「下载」文件夹")
                .on_click(move |event: &ClickEvent| {
                    // 点下载不能同时算点了这一行（文件夹会被打开）
                    event.stop_propagation();
                    download.files_download(&download_name);
                }),
        );
    }
    if directory {
        line = line.on_click(move || open.files_open(&name));
    }
    appear(with_actions(
        line,
        format!("menu-file-{}", entry.name),
        entry_actions(workspace, &entry),
    ))
}

/// 一行的右键菜单：文件夹可以「在这里新建终端」（进那个目录），文件是在它所在的目录；
/// 能下载的有「下载」；还有「复制路径」
fn entry_actions(workspace: &Rc<Workspace>, entry: &Entry) -> Vec<Action> {
    let name = entry.name.clone();
    let terminal = Rc::clone(workspace);
    let terminal_target = (entry.kind == Kind::Dir).then(|| name.clone());
    let mut actions = vec![Action {
        id: format!("file-terminal-{name}"),
        label: if entry.kind == Kind::Dir {
            "在这里新建终端".to_string()
        } else {
            "在所在目录新建终端".to_string()
        },
        run: later(move || terminal.files_new_terminal(terminal_target.as_deref())),
    }];
    if entry.kind != Kind::Other {
        let (download, download_name) = (Rc::clone(workspace), name.clone());
        actions.push(Action {
            id: format!("file-download-{name}"),
            label: "下载".to_string(),
            run: Rc::new(move || download.files_download(&download_name)),
        });
    }
    let (copy, copy_name) = (Rc::clone(workspace), name.clone());
    actions.push(Action {
        id: format!("file-copy-{name}"),
        label: "复制路径".to_string(),
        run: Rc::new(move || {
            if let Some(path) = copy.files_full_path(&copy_name) {
                aureole::clipboard::write_text(path);
            }
        }),
    });
    actions
}

/// 等菜单卸掉再做：在菜单的点击里直接开新标签，会话的读任务会记在菜单的作用域上，菜单一关就没了
fn later(run: impl Fn() + 'static) -> Rc<dyn Fn()> {
    let run = Rc::new(run);
    Rc::new(move || {
        let run = run.clone();
        aureole::detached(|| {
            aureole::spawn_local(async move {
                aureole::sleep(ms(1)).await;
                run();
            })
        });
    })
}

/// 列表空白处的右键菜单：在现在这个目录新建终端、刷新
fn blank_actions(workspace: &Rc<Workspace>) -> Vec<Action> {
    let (terminal, refresh) = (Rc::clone(workspace), Rc::clone(workspace));
    vec![
        Action {
            id: "files-blank-terminal".to_string(),
            label: "在此目录新建终端".to_string(),
            run: later(move || terminal.files_new_terminal(None)),
        },
        Action {
            id: "files-blank-refresh".to_string(),
            label: "刷新".to_string(),
            run: Rc::new(move || refresh.files_refresh()),
        },
    ]
}

fn entry_style() -> Style {
    soft(
        Style::new()
            .height(28)
            .padding_x(8)
            .radius(6)
            .color(fg())
            .background(Color::TRANSPARENT)
            .hovered(|style| style.background(hover())),
    )
}

/// 底部：传输记录（有才显示）和上传按钮
fn files_footer(
    workspace: &Rc<Workspace>,
    current: impl Fn() -> FilesState + Clone + 'static,
) -> Element {
    let (upload, clear) = (workspace.clone(), workspace.clone());
    let (any, list) = (current.clone(), current);
    column()
        .shrink(0)
        .child(rect().height(1).background(line()))
        .child_when(
            move || !any().transfers.is_empty(),
            move || {
                let (rows, clear) = (list.clone(), clear.clone());
                column()
                    .name("files-transfers")
                    .padding_x(10)
                    .padding_y(6)
                    .gap(4)
                    .child(column().gap(4).each(
                        move || rows().transfers,
                        |transfer: &files::Transfer| (transfer.id, transfer.state.clone()),
                        move |transfer| transfer_row(&transfer),
                    ))
                    .child(
                        text("清除记录")
                            .name("files-clear")
                            .font_size(11)
                            .color(text_3())
                            .on_click(move || clear.files_clear_transfers()),
                    )
            },
        )
        .child(
            row()
                .padding(10)
                .align(Align::Center)
                .gap(8)
                .child(
                    outline_button(Some(Icon::Upload), "上传文件")
                        .name("files-upload")
                        .on_click(move || upload.files_choose_upload()),
                )
                .child(text("也可以把文件拖到这里").font_size(11).color(text_3())),
        )
}

fn transfer_row(transfer: &files::Transfer) -> Element {
    let (glyph, status, tint) = match &transfer.state {
        TransferState::Running => (
            if transfer.upload {
                Icon::Upload
            } else {
                Icon::Download
            },
            "传输中…".to_string(),
            text_2(),
        ),
        TransferState::Done(note) => (
            if transfer.upload {
                Icon::Upload
            } else {
                Icon::Download
            },
            note.clone(),
            text_2(),
        ),
        TransferState::Failed(reason) => (Icon::Close, reason.clone(), fg()),
    };
    column()
        .name(format!("transfer-{}", transfer.id))
        .gap(2)
        .child(
            row()
                .align(Align::Center)
                .gap(6)
                .child(icon(glyph, 13.0).color(tint).shrink(0))
                .child(
                    text(transfer.name.clone())
                        .font_size(12)
                        .grow(1)
                        .min_width(0)
                        .max_lines(1),
                ),
        )
        .child(
            text(status)
                .name(format!("transfer-state-{}", transfer.id))
                .font_size(11)
                .color(tint)
                .margin_left(19)
                .max_lines(2),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有数据：一条底线；一个点：补成一小段平线，不是空的
    #[test]
    fn too_little_data_still_draws_something() {
        let empty = spark_points(&[], 240.0, 56.0);
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[0][1], empty[1][1], "平的");
        let one = spark_points(&[30.0], 240.0, 56.0);
        assert_eq!(one.len(), 2);
        assert_eq!(one[0][0], 0.0, "从最左边开始");
        assert_eq!(one[0][1], one[1][1]);
    }

    /// 从左边开始往右长：刚连上的头一分钟线在左边，不是只有最右边一小截
    #[test]
    fn a_short_history_starts_at_the_left_edge() {
        let points = spark_points(&[10.0, 20.0, 30.0], 240.0, 56.0);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0][0], 0.0);
        let step = 240.0 / (monitor::HISTORY - 1) as f32;
        assert!((points[2][0] - 2.0 * step).abs() < 0.001);
    }

    /// 攒满之后最后一个点正好在最右边；0% 在底、100% 在顶，都在框里
    #[test]
    fn a_full_history_spans_the_width_and_values_stay_inside() {
        let history: Vec<f32> = (0..monitor::HISTORY)
            .map(|index| (index * 2) as f32 % 130.0 - 10.0)
            .collect();
        let points = spark_points(&history, 300.0, 56.0);
        assert_eq!(points.len(), monitor::HISTORY);
        assert!((points.last().unwrap()[0] - 300.0).abs() < 0.001);
        assert!(
            points.iter().all(|point| (0.0..=56.0).contains(&point[1])),
            "超出 0..100 的值也画在框里"
        );
        let (low, high) = (
            spark_points(&[0.0, 0.0], 100.0, 56.0),
            spark_points(&[100.0, 100.0], 100.0, 56.0),
        );
        assert!(low[0][1] > high[0][1], "数值越大越靠上");
    }

    /// 填充是闭合的：沿着曲线，再落到底边回到起点
    #[test]
    fn the_area_closes_along_the_bottom() {
        let points = spark_points(&[10.0, 90.0, 50.0], 240.0, 56.0);
        let area = spark_area(&points, 56.0);
        let [_, top, _, bottom] = area.bounds();
        assert!(
            bottom >= 56.0 - 0.01
                && top <= points.iter().map(|point| point[1]).fold(f32::MAX, f32::min) + 0.01
        );
    }
}
