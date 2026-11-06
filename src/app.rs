//! 把 pty、仿真、界面接到窗口上。
//!
//! 一条数据的来回：
//!
//! ```text
//! 按键 ─ vt::keys::encode ─▶ pty 写线程 ─▶ shell
//! shell ─▶ pty 读线程 ─▶ ByteQueue ─▶ pump（主线程）─▶ Terminal::feed ─▶ view::paint ─▶ CharGrid
//!                                                    └▶ 应答（DSR 等）写回 pty
//! ```
//!
//! 窗口大小变了：算出放得下几列几行 → 仿真层、网格、pty（`SIGWINCH`）一起改。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aureole::{LocalTask, WheelDelta};
use aureole::prelude::*;

use crate::pty::{self, ByteQueue, Chunk, Pty};
use crate::view;
use crate::vt::mouse::{Action, Button, Report};
use crate::vt::{MouseMode, Terminal, keys, mouse};

/// 终端区域边缘留的空白（逻辑像素）
pub const PADDING: f32 = 12.0;
/// 一轮最多处理这么多字节就先让出主线程去画（`yes`、`cat 大文件` 时界面不卡）
const BATCH_BUDGET: usize = 256 * 1024;

/// 会话向外报告的几件事，外面（标签栏）决定怎么办。会话自己不认识窗口和标签
#[derive(Clone)]
pub struct Hooks {
    /// 程序设了新标题（OSC 0 / 2），可能是空的
    pub title: Rc<dyn Fn(&str)>,
    /// 行列数变了
    pub resized: Rc<dyn Fn(usize, usize)>,
    /// shell 退出了，带退出码
    pub exited: Rc<dyn Fn(i32)>,
    /// 读到了新的输出（一批一次）。标签不在前台时，侧栏靠它提示「有新输出」
    pub activity: Rc<dyn Fn()>,
}

impl Hooks {
    /// 什么都不管（测试里）
    pub fn none() -> Self {
        Self { title: Rc::new(|_| {}), resized: Rc::new(|_, _| {}), exited: Rc::new(|_| {}), activity: Rc::new(|| {}) }
    }
}

/// 起一个会话要的东西：shell 的 pty 和读队列，或者起不来的原因
pub struct Spawned {
    pub pty: Option<Pty>,
    pub queue: Option<ByteQueue>,
    pub error: Option<String>,
    /// 没有标题时标签上写什么（shell 的名字）
    pub name: String,
}

/// 起登录 shell
pub fn spawn_login_shell(columns: usize, rows: usize) -> Spawned {
    let config = pty::Config::login_shell(columns as u16, rows as u16);
    let name = std::path::Path::new(&config.program).file_name().map_or_else(|| "shell".to_string(), |name| name.to_string_lossy().into_owned());
    match pty::spawn(config) {
        Ok((pty, queue)) => Spawned { pty: Some(pty), queue: Some(queue), error: None, name },
        Err(error) => Spawned { pty: None, queue: None, error: Some(error.to_string()), name },
    }
}

/// 一个终端会话在主线程上的全部状态
pub struct Session {
    terminal: RefCell<Terminal>,
    grid: CharGrid,
    /// 起不来 shell 或者会话关了以后是 `None`（窗口里会写明原因）
    pty: RefCell<Option<Pty>>,
    hooks: Hooks,
    /// 读 pty 的任务；会话关了就取消，不然它手里的 `Rc` 让会话永远不释放
    pump: RefCell<Option<LocalTask>>,
    closed: Cell<bool>,
    /// 一格的真实大小（逻辑像素）：第一次布局之后从网格的边界反算出来
    cell: Cell<Option<(f32, f32)>>,
    shown_title: RefCell<String>,
    /// 往回翻了多少行（0 是跟着最新的输出走）
    offset: Cell<usize>,
    /// 网格上现在画的是哪个 `offset`
    shown_offset: Cell<usize>,
    /// 上次看到主屏一共往历史里放过多少行：回看时新输出让历史变长，靠它多翻几行，看着的那一屏才不动
    archived_seen: Cell<u64>,
    /// 滚轮、触控板滚不满一行的零头，攒到下一次
    wheel_rest: Cell<f32>,
    /// 鼠标上报时现在按着哪个键（松开时要报是哪个键）
    pressed: Cell<Option<Button>>,
    /// 上一次上报的鼠标在哪一格：移动只在换了格子时报
    last_cell: Cell<Option<(usize, usize)>>,
}

impl Session {
    /// 起一个会话：建仿真和网格、接上 pty、开始读。起不来 shell 时把原因写在屏幕上
    pub fn start(spawned: Spawned, columns: usize, rows: usize, hooks: Hooks) -> Rc<Self> {
        let mut terminal = Terminal::new(columns, rows);
        if let Some(error) = &spawned.error {
            terminal.feed(format!("起不了 shell：{error}\r\n").as_bytes());
        }
        let session = Self::new(terminal, CharGrid::new(columns, rows), spawned.pty, hooks);
        session.flush();
        if let Some(queue) = spawned.queue {
            *session.pump.borrow_mut() = Some(aureole::spawn_local(pump(session.clone(), queue)));
        }
        session
    }

    fn new(terminal: Terminal, grid: CharGrid, pty: Option<Pty>, hooks: Hooks) -> Rc<Self> {
        Rc::new(Self {
            terminal: RefCell::new(terminal),
            grid,
            pty: RefCell::new(pty),
            hooks,
            pump: RefCell::new(None),
            closed: Cell::new(false),
            cell: Cell::new(None),
            shown_title: RefCell::new(String::new()),
            offset: Cell::new(0),
            shown_offset: Cell::new(0),
            archived_seen: Cell::new(0),
            wheel_rest: Cell::new(0.0),
            pressed: Cell::new(None),
            last_cell: Cell::new(None),
        })
    }

    /// 关掉会话（标签关了）：停掉读的任务、放掉 pty（`Drop` 会给 shell 发 `SIGHUP`）
    pub fn close(&self) {
        self.closed.set(true);
        if let Some(task) = self.pump.borrow_mut().take() {
            task.cancel();
        }
        self.pty.borrow_mut().take();
    }

    /// 屏幕现在的行列数
    pub fn size(&self) -> (usize, usize) {
        self.terminal.borrow().size()
    }

    fn write(&self, bytes: &[u8]) {
        if let Some(pty) = self.pty.borrow().as_ref() {
            // 写不进去只有一种可能：shell 已经退出了，读线程会报结束，这里不用管
            let _ = pty.write(bytes);
        }
    }

    /// 仿真层攒下的东西送出去：给 shell 的应答、窗口标题、屏幕的改动
    fn flush(&self) {
        let (reply, title, history, total, alternate) = {
            let mut terminal = self.terminal.borrow_mut();
            (terminal.take_output(), terminal.title.clone(), terminal.scrollback_len(), terminal.scrollback_total(), terminal.on_alternate)
        };
        if !reply.is_empty() {
            self.write(&reply);
        }
        if *self.shown_title.borrow() != title {
            (self.hooks.title)(&title);
            *self.shown_title.borrow_mut() = title;
        }
        // 回看时来了新输出：历史每多一行，就多翻一行，看着的那一屏原地不动。
        // 副屏（vim、less）没有历史，历史被清掉或者缩短了也要截回来
        let mut offset = self.offset.get();
        if offset > 0 {
            offset += total.saturating_sub(self.archived_seen.get()) as usize;
        }
        self.archived_seen.set(total);
        if alternate {
            offset = 0;
        }
        offset = offset.min(history);
        self.offset.set(offset);
        view::paint(&mut self.terminal.borrow_mut(), &self.grid, offset, self.shown_offset.get());
        self.shown_offset.set(offset);
    }

    /// 往历史里翻：`lines` 为正是往更早的方向，为负是往最新的方向。到头了就停
    fn scroll_back(&self, lines: isize) {
        let history = self.terminal.borrow().scrollback_len() as isize;
        let offset = self.offset.get();
        let target = (offset as isize).saturating_add(lines).clamp(0, history) as usize;
        if target != offset {
            self.offset.set(target);
            // 选区记的是网格上的位置，内容换了它就指错地方了
            self.grid.set_selection(None);
            self.flush();
        }
    }

    /// 回到最新的输出
    fn snap_to_bottom(&self) {
        self.scroll_back(isize::MIN / 2);
    }

    /// 现在鼠标归程序（要上报）吗？按着 Shift 时永远归终端：这是和 xterm 一样的约定，
    /// 程序抓着鼠标的时候，还能用 Shift+拖动选字、复制
    fn mouse_reporting(&self, modifiers: Modifiers) -> bool {
        self.terminal.borrow().modes.mouse != MouseMode::Off && !modifiers.has_shift()
    }

    /// 元素坐标（逻辑像素）落在第几行第几列，限制在屏内。格子大小还没量到时是 `None`
    fn cell_at(&self, position: [f32; 2]) -> Option<(usize, usize)> {
        let (width, height) = self.cell.get()?;
        let (columns, rows) = self.terminal.borrow().size();
        let column = ((position[0] / width).floor().max(0.0) as usize).min(columns - 1);
        let row = ((position[1] / height).floor().max(0.0) as usize).min(rows - 1);
        Some((row, column))
    }

    /// 编码一次鼠标动作写给程序
    fn report_mouse(&self, button: Option<Button>, action: Action, cell: (usize, usize), modifiers: Modifiers) {
        let (mode, sgr) = {
            let terminal = self.terminal.borrow();
            (terminal.modes.mouse, terminal.modes.mouse_sgr)
        };
        let report = Report {
            button,
            action,
            column: cell.1,
            row: cell.0,
            shift: modifiers.has_shift(),
            alt: modifiers.has_alt(),
            control: modifiers.has_control(),
        };
        if let Some(bytes) = mouse::encode(&report, mode, sgr) {
            self.write(&bytes);
        }
    }

    fn pointer_down(&self, event: &PointerEvent) {
        if !self.mouse_reporting(event.modifiers) {
            return;
        }
        let button = match event.button {
            Some(MouseButton::Left) => Button::Left,
            Some(MouseButton::Middle) => Button::Middle,
            Some(MouseButton::Right) => Button::Right,
            _ => return,
        };
        let Some(cell) = self.cell_at(event.position) else { return };
        // 这次按下归程序：不选字、不开始拖动；抓住指针，拖出窗口也照样收到移动和松开
        event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
        event.capture_pointer();
        self.grid.set_selection(None);
        self.pressed.set(Some(button));
        self.last_cell.set(Some(cell));
        self.report_mouse(Some(button), Action::Press, cell, event.modifiers);
    }

    fn pointer_up(&self, event: &PointerEvent) {
        // 按下时归了程序，松开就一定要报，不管这时候 Shift 按没按、模式变没变
        let Some(button) = self.pressed.take() else { return };
        let Some(cell) = self.cell_at(event.position) else { return };
        self.report_mouse(Some(button), Action::Release, cell, event.modifiers);
    }

    fn pointer_move(&self, event: &PointerEvent) {
        if !self.mouse_reporting(event.modifiers) {
            return;
        }
        let Some(cell) = self.cell_at(event.position) else { return };
        if self.last_cell.get() == Some(cell) {
            return;
        }
        self.last_cell.set(Some(cell));
        self.report_mouse(self.pressed.get(), Action::Motion, cell, event.modifiers);
    }

    /// 窗口得到、失去键盘焦点：程序要的话告诉它（vim 靠它在切回来时重读文件）
    fn focus_changed(&self, focused: bool) {
        if self.terminal.borrow().modes.focus_reporting {
            self.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    /// 滚轮、触控板。程序要鼠标就上报成滚轮键；否则主屏是翻历史；
    /// 副屏（less、man、vim）没有历史，翻成方向键，和别的终端一样
    fn wheel(&self, event: &WheelEvent) {
        let cell_height = self.cell.get().map_or(view::FONT_SIZE * view::LINE_HEIGHT, |cell| cell.1);
        let lines = match event.delta {
            // 滚轮一格滚三行
            WheelDelta::Lines(delta) => delta[1] * 3.0,
            WheelDelta::Pixels(delta) => delta[1] / cell_height,
        };
        // 事件里正数是「内容往上走」（往最新的方向），翻历史的方向正好相反
        let total = self.wheel_rest.get() - lines;
        let whole = total.trunc();
        self.wheel_rest.set(total - whole);
        let steps = whole as isize;
        if steps == 0 {
            return;
        }
        if self.mouse_reporting(event.modifiers) {
            if let Some(cell) = self.cell_at(event.position) {
                let button = if steps > 0 { Button::WheelUp } else { Button::WheelDown };
                for _ in 0..steps.unsigned_abs().min(12) {
                    self.report_mouse(Some(button), Action::Press, cell, event.modifiers);
                }
            }
            return;
        }
        let (alternate, application) = {
            let terminal = self.terminal.borrow();
            (terminal.on_alternate, terminal.modes.application_cursor)
        };
        if alternate {
            let arrow = match (steps > 0, application) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..steps.unsigned_abs().min(12) {
                self.write(arrow);
            }
        } else {
            self.scroll_back(steps);
        }
    }

    /// 窗口变了：放得下几列几行就改成几列几行
    fn fit(&self, viewport: Rect, shown: Rect) {
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return;
        }
        let (columns, rows) = self.terminal.borrow().size();
        // 格子大小只在第一次布局时量：那时网格还是打开时的行列数，除出来是准的。
        // 之后行列数在变，网格的边界落后一帧，再量会把旧边界除以新行列数
        if self.cell.get().is_none() {
            self.cell.set(view::measure(shown, columns, rows));
        }
        let Some(cell) = self.cell.get() else { return };
        let (new_columns, new_rows) = view::fit(viewport.width - 2.0 * PADDING, viewport.height - 2.0 * PADDING, cell);
        if (new_columns, new_rows) == (columns, rows) {
            return;
        }
        self.terminal.borrow_mut().resize(new_columns, new_rows);
        if let Some(pty) = self.pty.borrow().as_ref() {
            pty.resize(new_columns as u16, new_rows as u16);
        }
        self.flush();
        (self.hooks.resized)(new_columns, new_rows);
    }

    /// 网格有焦点时的按键。返回 `true` 表示终端收下了（不再往外传）
    fn key(&self, event: &KeyEvent) -> bool {
        if is_paste(event) {
            self.paste();
            return true;
        }
        if self.scroll_key(event) {
            return true;
        }
        let keymap = self.terminal.borrow().keymap();
        let Some(bytes) = keys::encode(event, keymap) else { return false };
        // 打字、按方向键都是「继续用 shell 了」：选区作废，回看的话回到最新的输出
        self.grid.set_selection(None);
        self.write(&bytes);
        self.snap_to_bottom();
        true
    }

    /// Shift + PageUp / PageDown / Home / End：翻历史（和 xterm、iTerm、GNOME 终端一样）
    fn scroll_key(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if !modifiers.has_shift() || modifiers.has_control() || modifiers.has_alt() || modifiers.has_super() {
            return false;
        }
        let page = self.terminal.borrow().size().1.saturating_sub(1).max(1) as isize;
        match &event.key {
            Key::Named(NamedKey::PageUp) => self.scroll_back(page),
            Key::Named(NamedKey::PageDown) => self.scroll_back(-page),
            Key::Named(NamedKey::Home) => self.scroll_back(isize::MAX / 2),
            Key::Named(NamedKey::End) => self.snap_to_bottom(),
            _ => return false,
        }
        true
    }

    fn paste(&self) {
        let Some(text) = aureole::clipboard::read_text() else { return };
        let bracketed = self.terminal.borrow().modes.bracketed_paste;
        self.write(&keys::paste_bytes(&text, bracketed));
        self.snap_to_bottom();
    }

    /// 输入法提交的文字
    fn commit_text(&self, text: &str) {
        self.grid.set_selection(None);
        self.write(text.as_bytes());
        self.snap_to_bottom();
    }

    /// shell 退出了。出错退出（退出码不是 0）把原因写在屏幕上，让人看得到最后的输出；
    /// 然后交给外面决定：正常退出（`exit`、Ctrl-D）标签就关掉
    fn exited(&self, code: i32) {
        if self.closed.get() {
            return;
        }
        if code != 0 {
            self.terminal.borrow_mut().feed(format!("\r\n[进程已退出，退出码 {code}]\r\n").as_bytes());
            self.flush();
        }
        (self.hooks.exited)(code);
    }
}

/// 粘贴的快捷键：macOS 是 ⌘V，Windows、Linux 是 Ctrl+Shift+V（Ctrl+V 要留给 shell）
fn is_paste(event: &KeyEvent) -> bool {
    let is_v = matches!(&event.key, Key::Character(text) if text.as_str().eq_ignore_ascii_case("v"));
    let modifiers = event.modifiers;
    is_v && (modifiers.has_super() || (modifiers.has_control() && modifiers.has_shift()))
}

/// 后台读到的字节搬到仿真层：一批一批处理，处理完画一次
async fn pump(session: Rc<Session>, queue: ByteQueue) {
    while let Some(first) = queue.next().await {
        let mut next = Some(first);
        let mut spent = 0;
        let mut exit = None;
        while let Some(chunk) = next.take() {
            match chunk {
                Chunk::Bytes(bytes) => {
                    spent += bytes.len();
                    session.terminal.borrow_mut().feed(&bytes);
                }
                Chunk::Closed(code) => {
                    exit = Some(code);
                    break;
                }
            }
            if spent >= BATCH_BUDGET {
                break;
            }
            next = queue.try_next();
        }
        session.flush();
        if spent > 0 {
            (session.hooks.activity)();
        }
        if let Some(code) = exit {
            // 任务马上就结束了：先把自己的句柄放掉，免得外面关会话时「取消」到正在跑的自己
            let _ = session.pump.borrow_mut().take();
            session.exited(code);
            return;
        }
        if spent >= BATCH_BUDGET {
            // 让出主线程：这一帧该画、该响应输入了
            aureole::sleep(ms(1)).await;
        }
    }
}

/// 会话的视图：网格加上把按键、指针、滚轮、输入法接到会话上的处理器。
/// 背景和文字颜色由外面（窗口）统一给，这里只管终端自己
pub fn view(session: Rc<Session>) -> Element {
    let grid = session.grid.clone();
    // 窗口的内容区（算放得下几列几行）和网格自己的边界（反算格子大小）
    let viewport = signal(Rect::default());
    let shown = signal(Rect::default());
    let fitter = session.clone();
    Effect::new(move || fitter.fit(viewport.get(), shown.get()));

    let focus = FocusHandle::new();
    let (keys_session, ime_session, wheel_session) = (session.clone(), session.clone(), session.clone());
    let (down_session, up_session, move_session, focus_session) = (session.clone(), session.clone(), session.clone(), session);
    column()
        .grow(1)
        // 终端区域的大小由窗口决定、不由里面的网格决定：网格比窗口宽时要被压住（再由 `fit` 缩小行列），
        // 不能把整块卡片撑出窗口
        .min_width(0)
        .min_height(0)
        .clip(true)
        .padding(PADDING)
        .report_bounds(viewport)
        .on_mount({
            let focus = focus.clone();
            move || focus.focus()
        })
        .child(
            char_grid(&grid)
                .name("grid")
                .align_self(Align::Start)
                .focus_handle(focus)
                .report_bounds(shown)
                .font_size(view::FONT_SIZE)
                .line_height(view::LINE_HEIGHT)
                .on_key_down(move |event: &KeyEvent| {
                    if keys_session.key(event) {
                        event.stop_propagation();
                    }
                })
                // 程序要鼠标时（vim、htop、tmux）把指针事件编码了写给它；不要时这几个处理器什么都不做，
                // 选字、复制照旧由网格自己处理
                .on_pointer_down(move |event: &PointerEvent| down_session.pointer_down(event))
                .on_pointer_up(move |event: &PointerEvent| up_session.pointer_up(event))
                .on_pointer_move(move |event: &PointerEvent| move_session.pointer_move(event))
                .on_keyboard_focus(move |focused: bool| focus_session.focus_changed(focused))
                .on_wheel(move |event: &WheelEvent| {
                    wheel_session.wheel(event);
                    event.stop_propagation();
                })
                // 输入法：预编辑画在插入点处，提交的字原样写给 shell
                .on_text_insert(move |text: String| ime_session.commit_text(&text)),
        )
        .into()
}

#[cfg(test)]
impl Session {
    pub(crate) fn hooks_for_test(&self) -> Hooks {
        self.hooks.clone()
    }

    pub(crate) fn exited_for_test(&self, code: i32) {
        self.exited(code);
    }

    /// 当作 shell 输出了这段文字
    pub(crate) fn feed_for_test(&self, text: &str) {
        self.terminal.borrow_mut().feed(text.as_bytes());
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aureole::testing::Harness;

    use super::*;
    use crate::pty::Recorder;

    /// 离屏挂一个会话（假 pty），先喂 `setup`（让程序开鼠标模式），布局一帧，返回网格的位置和一格的大小
    fn mounted(setup: &str) -> (Harness, Arc<Recorder>, Rect, (f32, f32)) {
        let recorder = Arc::new(Recorder::default());
        let (columns, rows) = (80, 24);
        let mut terminal = Terminal::new(columns, rows);
        terminal.feed(setup.as_bytes());
        let session = Session::new(terminal, CharGrid::new(columns, rows), Some(Pty::from_backend(recorder.clone())), Hooks::none());
        let harness = Harness::new(960.0, 600.0, view(session.clone()));
        harness.frame();
        let grid = harness.bounds("grid").expect("网格要在元素树里");
        let [columns, rows] = session.grid.size();
        let cell = (grid.width / columns as f32, grid.height / rows as f32);
        (harness, recorder, grid, cell)
    }

    /// 第 `row` 行第 `column` 列（0 起算）那一格的中心，窗口坐标
    fn center(grid: Rect, cell: (f32, f32), row: usize, column: usize) -> (f32, f32) {
        (grid.x + (column as f32 + 0.5) * cell.0, grid.y + (row as f32 + 0.5) * cell.1)
    }

    /// 程序开了鼠标上报（1000 + SGR）：点一下，按下和松开都要报，坐标是点中的那一格
    #[test]
    fn click_is_reported_with_the_cell_under_the_pointer() {
        let (harness, recorder, grid, cell) = mounted("\x1b[?1000h\x1b[?1006h");
        let (x, y) = center(grid, cell, 4, 9);
        harness.click(x, y);
        assert_eq!(recorder.take(), "\x1b[<0;10;5M\x1b[<0;10;5m");
    }

    /// 没开鼠标上报时，点击不能漏给 shell（会在提示符里打出乱码）
    #[test]
    fn click_is_not_reported_when_the_program_did_not_ask() {
        let (harness, recorder, grid, cell) = mounted("");
        let (x, y) = center(grid, cell, 4, 9);
        harness.click(x, y);
        assert_eq!(recorder.take(), "");
    }

    /// 按着 Shift 点：永远归终端（选字），不上报
    #[test]
    fn shift_click_bypasses_mouse_reporting() {
        let (harness, recorder, grid, cell) = mounted("\x1b[?1000h\x1b[?1006h");
        let (x, y) = center(grid, cell, 1, 1);
        harness.click_with(x, y, Modifiers::SHIFT);
        assert_eq!(recorder.take(), "");
    }

    /// 1003 下没按键的移动也报，而且同一格里动不重复报
    #[test]
    fn motion_is_reported_once_per_cell_in_any_motion_mode() {
        let (harness, recorder, grid, cell) = mounted("\x1b[?1003h\x1b[?1006h");
        let (x, y) = center(grid, cell, 2, 3);
        harness.mouse_move(x, y);
        harness.mouse_move(x + 1.0, y);
        assert_eq!(recorder.take(), "\x1b[<35;4;3M");
        let (x, y) = center(grid, cell, 2, 4);
        harness.mouse_move(x, y);
        assert_eq!(recorder.take(), "\x1b[<35;5;3M");
    }
}
