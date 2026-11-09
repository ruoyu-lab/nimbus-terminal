//! 右侧面板：当前会话的目标带什么面板（监控、文件）就显示什么。本机没有面板，这一块根本不存在。
//! 它是一张和主卡片一样的浮起来的卡片，左边那条空隙可以按住左右拖动改宽度。

use std::rc::Rc;

use aureole::prelude::*;

use crate::chrome::widgets::{appear, soft};
use crate::monitor::{self, Monitor, Status};
use crate::chrome::{GUTTER, PANEL_MAX, PANEL_MIN};
use crate::icons::{Icon, icon};
use crate::tabs::Workspace;
use crate::target::PanelKind;
use crate::theme::{card_bg, card_shadow, fg, hover, mark, selected, text_2, text_3};

pub(super) fn view(workspace: &Rc<Workspace>) -> Element {
    let (shown, owner) = (workspace.clone(), workspace.clone());
    let width = workspace.panel_width;
    // 条件不成立时整块都不存在（不占宽度）；每次显示都重新建，开关几次都没事
    row().min_height(0).child_when(move || shown.panel_shown(), move || {
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
    })
}

fn card(workspace: &Rc<Workspace>, width: Signal<f32>) -> Element {
    let (kind, owner) = (workspace.panel_tab, workspace.clone());
    let (pages, monitor_owner) = (workspace.clone(), workspace.clone());
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
        .child_when(move || page(&owner, kind) == PanelKind::Files, || {
            column()
                .grow(1)
                .min_height(0)
                .align(Align::Center)
                .justify(Justify::Center)
                .gap(8)
                .color(text_3())
                .padding(16)
                .child(placeholder(
                    Icon::Folder,
                    "文件",
                    "连上服务器后，这里显示远端的文件，可以拖拽上传和下载。",
                ))
        })
        .into()
}

/// 现在显示哪一页：用户选的，没有这一页（换了目标）就退到第一页
fn page(workspace: &Rc<Workspace>, kind: Signal<PanelKind>) -> PanelKind {
    let panels = workspace.panels();
    let wanted = kind.get();
    if panels.contains(&wanted) { wanted } else { panels.first().copied().unwrap_or(wanted) }
}

fn page_tabs(workspace: &Rc<Workspace>) -> Element {
    let (strip, workspace) = (workspace.clone(), workspace.clone());
    let kind = workspace.panel_tab;
    row().height(38).padding_x(10).align(Align::Center).gap(2).each(
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

fn placeholder(glyph: Icon, title: &'static str, hint: &'static str) -> Element {
    column()
        .align(Align::Center)
        .gap(8)
        .child(icon(glyph, 26.0))
        .child(text(title).font_size(15).color(fg()))
        .child(text(hint).font_size(12).color(text_2()).text_align(TextAlign::Center))
}

/// 面板左边的拖动条（正好是两张卡片之间的空隙）：指针变成左右箭头，按住左右拖就改面板宽度。
/// 按下时记下指针和面板宽度，往左拖指针面板变宽，不会在按下的一瞬间跳
fn resize_handle(width: Signal<f32>) -> Element {
    let grab = signal(None::<(f32, f32)>);
    let resize = move |event: &PointerEvent| {
        if let Some((start, width_then)) = grab.get_untracked() {
            aureole::without_animation(|| {
                width.set((width_then - (event.window_position[0] - start)).clamp(PANEL_MIN, PANEL_MAX))
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
    let current = move || monitors.get().get(&active.get()).cloned().unwrap_or_default();
    let (status, body) = (current.clone(), current);
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
        .child_when(
            {
                let body = body.clone();
                move || body().latest.is_some()
            },
            move || stats(body.clone()),
        )
}

/// 顶上一行：连着时写主机的运行时间，没连着、不支持、出错时写明白
fn status_line(current: impl Fn() -> Monitor + Clone + 'static) -> Element {
    let message = move || -> (String, bool) {
        let monitor = current();
        match monitor.status {
            Status::Waiting => ("正在读取…".to_string(), false),
            Status::Live => (
                format!("运行了 {}", monitor::uptime(monitor.latest.map_or(0, |snapshot| snapshot.uptime))),
                true,
            ),
            Status::NotConnected => ("未连接：终端连上之后这里才有数据。".to_string(), false),
            Status::Unsupported => ("这台服务器暂不支持监控（需要 Linux）。".to_string(), false),
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

/// 四块数字：CPU（带曲线）、内存、磁盘、负载
fn stats(current: impl Fn() -> Monitor + Clone + 'static) -> Element {
    let (cpu, memory, disk, load) = (current.clone(), current.clone(), current.clone(), current.clone());
    let history = current;
    column()
        .gap(16)
        .child(block(
            "CPU",
            move || cpu().latest.and_then(|snapshot| snapshot.cpu).map_or("…".to_string(), |percent| format!("{percent:.0}%")),
            column().gap(8).child(sparkline(history)),
        ))
        .child(block(
            "内存",
            {
                let memory = memory.clone();
                move || usage_label(&memory(), |snapshot| (snapshot.mem_used, snapshot.mem_total))
            },
            bar(memory, |snapshot| (snapshot.mem_used, snapshot.mem_total)).name("monitor-memory-bar"),
        ))
        .child(block(
            "磁盘 /",
            {
                let disk = disk.clone();
                move || usage_label(&disk(), |snapshot| (snapshot.disk_used, snapshot.disk_total))
            },
            bar(disk, |snapshot| (snapshot.disk_used, snapshot.disk_total)).name("monitor-disk-bar"),
        ))
        .child(block(
            "负载",
            move || {
                load()
                    .latest
                    .map_or("…".to_string(), |snapshot| format!("{:.2}  {:.2}  {:.2}", snapshot.load[0], snapshot.load[1], snapshot.load[2]))
            },
            column().child(text("1、5、15 分钟").font_size(11).color(text_3())),
        ))
        .into()
}

fn usage_label(monitor: &Monitor, pick: impl Fn(&monitor::Snapshot) -> (u64, u64)) -> String {
    monitor.latest.as_ref().map_or("…".to_string(), |snapshot| {
        let (used, total) = pick(snapshot);
        let percent = if total == 0 { 0.0 } else { used as f64 / total as f64 * 100.0 };
        format!("{} / {}  {percent:.0}%", monitor::size(used), monitor::size(total))
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
fn bar(current: impl Fn() -> Monitor + 'static, pick: impl Fn(&monitor::Snapshot) -> (u64, u64) + 'static) -> Element {
    let share = Rc::new(move || -> f32 {
        current().latest.map_or(0.0, |snapshot| {
            let (used, total) = pick(&snapshot);
            if total == 0 { 0.0 } else { (used as f32 / total as f32).clamp(0.0, 1.0) }
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
    let mut points: Vec<[f32; 2]> =
        history.iter().enumerate().map(|(index, percent)| [index as f32 * step, y(*percent)]).collect();
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
    builder.line_to(last[0], height).line_to(first[0], height).close().build()
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
            path(move || spark_area(&spark_points(&area_history().cpu_history, area_width(), SPARK_HEIGHT), SPARK_HEIGHT))
                .name("monitor-cpu-area")
                .position(0, 0)
                .size(move || px(width()), SPARK_HEIGHT)
                .color(text_2().with_alpha(0.14)),
        )
        .child(
            path(move || VectorPath::polyline(spark_points(&line_history().cpu_history, line_width(), SPARK_HEIGHT)))
                .name("monitor-cpu-line")
                .position(0, 0)
                .size(move || px(width()), SPARK_HEIGHT)
                .stroke(Stroke::new(1.5).round())
                .color(text_2()),
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
        let history: Vec<f32> = (0..monitor::HISTORY).map(|index| (index * 2) as f32 % 130.0 - 10.0).collect();
        let points = spark_points(&history, 300.0, 56.0);
        assert_eq!(points.len(), monitor::HISTORY);
        assert!((points.last().unwrap()[0] - 300.0).abs() < 0.001);
        assert!(points.iter().all(|point| (0.0..=56.0).contains(&point[1])), "超出 0..100 的值也画在框里");
        let (low, high) = (spark_points(&[0.0, 0.0], 100.0, 56.0), spark_points(&[100.0, 100.0], 100.0, 56.0));
        assert!(low[0][1] > high[0][1], "数值越大越靠上");
    }

    /// 填充是闭合的：沿着曲线，再落到底边回到起点
    #[test]
    fn the_area_closes_along_the_bottom() {
        let points = spark_points(&[10.0, 90.0, 50.0], 240.0, 56.0);
        let area = spark_area(&points, 56.0);
        let [_, top, _, bottom] = area.bounds();
        assert!(bottom >= 56.0 - 0.01 && top <= points.iter().map(|point| point[1]).fold(f32::MAX, f32::min) + 0.01);
    }
}
