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

use aureole::prelude::*;
use aureole::{LocalTask, WheelDelta};

use crate::pty::{self, ByteQueue, Chunk, Pty};
use crate::settings::{Bell, Form};
use crate::target::Target;
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
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            title: Rc::new(|_| {}),
            resized: Rc::new(|_, _| {}),
            exited: Rc::new(|_| {}),
            activity: Rc::new(|| {}),
        }
    }
}

/// 起一个会话要的东西：shell 的 pty 和读队列，或者起不来的原因
pub struct Spawned {
    pub pty: Option<Pty>,
    pub queue: Option<ByteQueue>,
    pub error: Option<String>,
}

/// 怎么起一个会话
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub target: Target,
    pub columns: usize,
    pub rows: usize,
    /// 起始目录；`None` 是主目录
    pub directory: Option<std::path::PathBuf>,
    /// 指定的 shell 程序；`None` 是系统默认的登录 shell
    pub shell: Option<String>,
    /// 终端底色是深色
    pub dark_background: bool,
}

/// 起一个本机会话
pub fn spawn(launch: &Launch) -> Spawned {
    let Target::Local = launch.target;
    let (columns, rows) = (launch.columns as u16, launch.rows as u16);
    let mut config = match &launch.shell {
        Some(program) => pty::Config::shell(program, columns, rows),
        None => pty::Config::login_shell(columns, rows),
    };
    if let Some(directory) = &launch.directory {
        config.working_directory = Some(directory.clone());
    }
    config.dark_background = launch.dark_background;
    match pty::spawn(config).map_err(|error| error.to_string()) {
        Ok((pty, queue)) => Spawned {
            pty: Some(pty),
            queue: Some(queue),
            error: None,
        },
        Err(error) => Spawned {
            pty: None,
            queue: None,
            error: Some(error),
        },
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
    /// 设置：配色、字体、光标、滚轮、响铃都读它
    form: Form,
    /// 响铃已经处理到第几次
    bell_seen: Cell<u64>,
    /// 画面闪过几次（响铃设成「画面闪一下」时加一），视图盯着它
    flashes: Signal<u64>,
    /// 下一次画要把整屏的颜色重算（配色换了）
    repaint_all: Cell<bool>,
    /// 字体、字号变了、格子大小要重新量：记着变的时候网格的边界，边界变了才说明新的格子大小已经布局出来
    remeasure_from: Cell<Option<Rect>>,
    /// 屏幕内容每变一次加一：存终端内容时靠它知道这个会话自上次存以后有没有新输出
    output_version: Cell<u64>,
}

impl Session {
    /// 起一个会话：建仿真和网格、接上 pty、开始读。起不来 shell 时把原因写在屏幕上
    pub fn start(
        spawned: Spawned,
        columns: usize,
        rows: usize,
        hooks: Hooks,
        form: Form,
        history: Option<&str>,
    ) -> Rc<Self> {
        let mut terminal = Terminal::new(columns, rows);
        // 上次存下的内容先放上去，新 shell 的提示符接在它后面
        if let Some(history) = history.filter(|history| !history.is_empty()) {
            terminal.set_scrollback_limit(form.scrollback_lines());
            terminal.feed(history.as_bytes());
            terminal.feed(b"\x1b[0m\r\n");
        }
        if let Some(error) = &spawned.error {
            terminal.feed(format!("起不了会话：{error}\r\n").as_bytes());
        }
        let session = Self::new(
            terminal,
            CharGrid::new(columns, rows),
            spawned.pty,
            hooks,
            form,
        );
        session.apply_scrollback();
        session.flush();
        if let Some(queue) = spawned.queue {
            // 挂在一个不随界面卸载的作用域上。从文件夹右键菜单里创建时，菜单的作用域马上就释放，
            // 读任务要是记在菜单上，会被一起取消：shell 已经起来（光标在闪），提示符却永远到不了屏幕
            let reading = session.clone();
            let task = session_scope().run(|| aureole::spawn_local(pump(reading, queue)));
            *session.pump.borrow_mut() = Some(task);
        }
        session
    }

    fn new(
        terminal: Terminal,
        grid: CharGrid,
        pty: Option<Pty>,
        hooks: Hooks,
        form: Form,
    ) -> Rc<Self> {
        let bell_seen = terminal.bell;
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
            form,
            bell_seen: Cell::new(bell_seen),
            // 信号归当前的作用域，从命令面板、右键菜单里点出来的新标签，作用域一会儿就释放了，
            // 之后再读它就 panic（在鼠标事件的回调里就是直接 abort）：会话比任何界面都活得久，信号放在不挂界面的作用域里
            flashes: aureole::detached(|| signal(0)),
            repaint_all: Cell::new(false),
            remeasure_from: Cell::new(None),
            output_version: Cell::new(0),
        })
    }

    /// 现在画屏幕用的配色和光标
    fn look(&self) -> view::Look {
        view::Look {
            colors: view::colors(crate::theme::dark().get_untracked()),
            cursor: self.form.cursor.get_untracked(),
            blink: self.form.cursor_blink.get_untracked(),
        }
    }

    /// 历史行数的设置变了（或刚起来）：照设置截
    pub fn apply_scrollback(&self) {
        self.terminal
            .borrow_mut()
            .set_scrollback_limit(self.form.scrollback_lines());
    }

    /// 配色、光标变了：整屏按新的重画
    fn restyle(&self) {
        self.repaint_all.set(true);
        self.flush();
    }

    /// 字体、字号、行距变了：格子大小要重新量
    fn remeasure(&self, current: Rect) {
        self.cell.set(None);
        self.remeasure_from.set(Some(current));
    }

    /// shell 现在所在的目录（存下来下次启动在这里开）
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        self.pty.borrow().as_ref()?.working_directory()
    }

    /// 屏幕内容的版本：有新输出就变
    pub fn output_version(&self) -> u64 {
        self.output_version.get()
    }

    /// 终端里现在的内容（历史加屏幕），写成能喂回终端的文字，最多 `max_lines` 行
    pub fn transcript(&self, max_lines: usize) -> String {
        self.terminal.borrow().transcript(max_lines)
    }

    /// 前台有没有 shell 之外的程序在跑
    pub fn is_busy(&self) -> bool {
        self.pty.borrow().as_ref().is_some_and(Pty::is_busy)
    }

    /// 程序输出 BEL：按设置出声或者闪一下（一批输出里响多次也只算一次）
    fn ring(&self) {
        match self.form.bell.get_untracked() {
            Bell::Off => {}
            Bell::Sound => crate::native::beep(),
            Bell::Flash => self.flashes.update(|count| *count += 1),
        }
    }

    /// 关掉会话（标签关了）：停掉读的任务、放掉 pty（`Drop` 会给 shell 发 `SIGHUP`）
    pub fn close(&self) {
        self.closed.set(true);
        if let Some(task) = self.pump.borrow_mut().take() {
            task.cancel();
        }
        self.pty.borrow_mut().take();
    }

    fn write(&self, bytes: &[u8]) {
        if let Some(pty) = self.pty.borrow().as_ref() {
            // 写不进去只有一种可能：shell 已经退出了，读线程会报结束，这里不用管
            let _ = pty.write(bytes);
        }
    }

    /// 仿真层攒下的东西送出去：给 shell 的应答、窗口标题、屏幕的改动
    fn flush(&self) {
        let (reply, title, history, total, alternate, bell) = {
            let mut terminal = self.terminal.borrow_mut();
            (
                terminal.take_output(),
                terminal.title.clone(),
                terminal.scrollback_len(),
                terminal.scrollback_total(),
                terminal.on_alternate,
                terminal.bell,
            )
        };
        if bell != self.bell_seen.replace(bell) {
            self.ring();
        }
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
        view::paint(
            &mut self.terminal.borrow_mut(),
            &self.grid,
            self.look(),
            offset,
            self.shown_offset.get(),
            self.repaint_all.take(),
        );
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
    fn report_mouse(
        &self,
        button: Option<Button>,
        action: Action,
        cell: (usize, usize),
        modifiers: Modifiers,
    ) {
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
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
        // 这次按下归程序：不选字、不开始拖动；抓住指针，拖出窗口也照样收到移动和松开
        event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
        event.capture_pointer();
        self.grid.set_selection(None);
        self.pressed.set(Some(button));
        self.last_cell.set(Some(cell));
        self.report_mouse(Some(button), Action::Press, cell, event.modifiers);
    }

    fn pointer_up(&self, event: &PointerEvent) {
        // 选中后自动复制：这次选字归终端自己（程序没抓着鼠标）时，松手就把选中的字放进剪贴板
        if self.form.copy_on_select.get_untracked()
            && self.pressed.get().is_none()
            && !self.mouse_reporting(event.modifiers)
            && let Some(text) = self.grid.selected_text()
            && !text.is_empty()
        {
            aureole::clipboard::write_text(text);
        }
        // 按下时归了程序，松开就一定要报，不管这时候 Shift 按没按、模式变没变
        let Some(button) = self.pressed.take() else {
            return;
        };
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
        self.report_mouse(Some(button), Action::Release, cell, event.modifiers);
    }

    fn pointer_move(&self, event: &PointerEvent) {
        if !self.mouse_reporting(event.modifiers) {
            return;
        }
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
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
        let cell_height = self
            .cell
            .get()
            .map_or(view::FONT_SIZE * view::LINE_HEIGHT, |cell| cell.1);
        let lines = match event.delta {
            // 滚轮一格滚几行听设置的
            WheelDelta::Lines(delta) => delta[1] * self.form.wheel_lines(),
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
                let button = if steps > 0 {
                    Button::WheelUp
                } else {
                    Button::WheelDown
                };
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
            // 字体刚换：网格的边界还是旧字体的，要等它布局成新的再量
            if self.remeasure_from.get() == Some(shown) {
                return;
            }
            self.remeasure_from.set(None);
            self.cell.set(view::measure(shown, columns, rows));
        }
        let Some(cell) = self.cell.get() else { return };
        let (new_columns, new_rows) = view::fit(
            viewport.width - 2.0 * PADDING,
            viewport.height - 2.0 * PADDING,
            cell,
        );
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
        let Some(bytes) = keys::encode(event, keymap) else {
            return false;
        };
        // 打字、按方向键都是「继续用 shell 了」：选区作废，回看的话回到最新的输出
        self.grid.set_selection(None);
        self.write(&bytes);
        self.snap_to_bottom();
        true
    }

    /// Shift + PageUp / PageDown / Home / End：翻历史（和 xterm、iTerm、GNOME 终端一样）
    fn scroll_key(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if !modifiers.has_shift()
            || modifiers.has_control()
            || modifiers.has_alt()
            || modifiers.has_super()
        {
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

    /// 右键。程序抓着鼠标时（按着 Shift 除外）右键归程序；设置里选了「右键粘贴」就直接粘贴；
    /// 都不是就该弹右键菜单，返回 `true`
    fn secondary_click(&self, modifiers: Modifiers) -> bool {
        if self.mouse_reporting(modifiers) {
            return false;
        }
        if self.form.right_click_paste.get_untracked() {
            self.paste();
            return false;
        }
        true
    }

    /// 选中的字（没有选中是 `None`）
    pub fn selected_text(&self) -> Option<String> {
        self.grid.selected_text().filter(|text| !text.is_empty())
    }

    /// 把选中的字放进剪贴板；没有选中什么也不做
    pub fn copy_selection(&self) {
        if let Some(text) = self.selected_text() {
            aureole::clipboard::write_text(text);
        }
    }

    /// 选中屏幕上的全部内容
    pub fn select_all(&self) {
        let [columns, rows] = self.grid.size();
        self.grid.set_selection(Some((
            CharGridPosition::new(0, 0),
            CharGridPosition::new(rows.saturating_sub(1), columns),
        )));
    }

    /// 清屏：连滚动历史一起清掉，再让 shell 重画提示符（Ctrl-L）
    pub fn clear_screen(&self) {
        self.grid.set_selection(None);
        self.terminal.borrow_mut().feed(b"\x1b[H\x1b[2J\x1b[3J");
        self.output_version.set(self.output_version.get() + 1);
        self.offset.set(0);
        self.flush();
        self.write(b"\x0c");
    }

    /// 把剪贴板里的字粘贴进去
    pub fn paste(&self) {
        let Some(text) = aureole::clipboard::read_text() else {
            return;
        };
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
            self.terminal
                .borrow_mut()
                .feed(format!("\r\n[进程已退出，退出码 {code}]\r\n").as_bytes());
            self.output_version.set(self.output_version.get() + 1);
            self.flush();
        }
        (self.hooks.exited)(code);
    }
}

/// 会话读 shell 的任务记在这里。这个作用域没有父级，菜单、页面卸掉都不会释放它；
/// 会话自己关掉时再取消任务
fn session_scope() -> aureole::Scope {
    thread_local! {
        static SCOPE: std::cell::Cell<Option<aureole::Scope>> = const { std::cell::Cell::new(None) };
    }
    SCOPE.with(|slot| {
        if let Some(scope) = slot.get() {
            return scope;
        }
        let scope = aureole::detached(aureole::Scope::new);
        slot.set(Some(scope));
        scope
    })
}

/// 粘贴的快捷键：macOS 是 ⌘V，Windows、Linux 是 Ctrl+Shift+V（Ctrl+V 要留给 shell）
fn is_paste(event: &KeyEvent) -> bool {
    let is_v =
        matches!(&event.key, Key::Character(text) if text.as_str().eq_ignore_ascii_case("v"));
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
            session.output_version.set(session.output_version.get() + 1);
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
    let (keys_session, ime_session, wheel_session) =
        (session.clone(), session.clone(), session.clone());
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
            char_grid(&grid)
                .name("grid")
                .align_self(Align::Start)
                .focus_handle(focus)
                // 键盘切标签后终端拿到焦点：整块终端就是输入区，不在外面画框
                .focus_ring(FocusRing::NONE)
                .report_bounds(shown)
                .on_secondary_click(move |event: &ClickEvent| {
                    if click_session.secondary_click(event.modifiers) {
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
}

#[cfg(test)]
impl Session {
    pub(crate) fn flashes_for_test(&self) -> u64 {
        self.flashes.get_untracked()
    }

    pub(crate) fn hooks_for_test(&self) -> Hooks {
        self.hooks.clone()
    }

    pub(crate) fn exited_for_test(&self, code: i32) {
        self.exited(code);
    }

    /// 屏幕上现在的文字，测试用来看 shell 有没有回话
    pub(crate) fn screen_text_for_test(&self) -> String {
        let terminal = self.terminal.borrow();
        let screen = terminal.screen();
        screen
            .lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|cell| if cell.width == 0 { '\0' } else { cell.ch })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 当作 shell 输出了这段文字
    pub(crate) fn feed_for_test(&self, text: &str) {
        self.terminal.borrow_mut().feed(text.as_bytes());
        self.output_version.set(self.output_version.get() + 1);
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
        let (harness, recorder, grid, cell, _session) =
            mounted_with(setup, crate::settings::Settings::default());
        (harness, recorder, grid, cell)
    }

    /// 同上，用指定的设置，并把会话也交出来
    fn mounted_with(
        setup: &str,
        settings: crate::settings::Settings,
    ) -> (Harness, Arc<Recorder>, Rect, (f32, f32), Rc<Session>) {
        let recorder = Arc::new(Recorder::default());
        let (columns, rows) = (80, 24);
        let mut terminal = Terminal::new(columns, rows);
        terminal.feed(setup.as_bytes());
        let session = Session::new(
            terminal,
            CharGrid::new(columns, rows),
            Some(Pty::from_backend(recorder.clone())),
            Hooks::none(),
            Form::new(&settings),
        );
        session.apply_scrollback();
        let harness = Harness::new(960.0, 600.0, view(session.clone(), || true, |_| {}));
        harness.frame();
        let grid = harness.bounds("grid").expect("网格要在元素树里");
        let [columns, rows] = session.grid.size();
        let cell = (grid.width / columns as f32, grid.height / rows as f32);
        (harness, recorder, grid, cell, session)
    }

    fn lines(count: usize) -> String {
        (0..count).map(|n| format!("line {n}\r\n")).collect()
    }

    /// 第 `row` 行第 `column` 列（0 起算）那一格的中心，窗口坐标
    fn center(grid: Rect, cell: (f32, f32), row: usize, column: usize) -> (f32, f32) {
        (
            grid.x + (column as f32 + 0.5) * cell.0,
            grid.y + (row as f32 + 0.5) * cell.1,
        )
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

    use crate::settings::{Bell, CursorStyle, Settings};

    /// 滚轮一格滚几行听设置的（默认 3）
    #[test]
    fn the_wheel_scrolls_as_many_lines_as_the_setting_says() {
        for (setting, expected) in [(3.0, 3), (10.0, 10), (1.0, 1)] {
            let (harness, _recorder, grid, _cell, session) = mounted_with(
                "",
                Settings {
                    wheel: setting,
                    ..Settings::default()
                },
            );
            session.feed_for_test(&lines(120));
            let [x, y] = grid.center();
            // 滚轮往回拨一格：往更早的历史翻
            harness.wheel(x, y, 0.0, -1.0);
            assert_eq!(session.offset.get(), expected, "滚轮设成 {setting}");
        }
    }

    /// 历史行数听设置的：写得再多也只留设定的行数；调小马上丢掉多出来的
    #[test]
    fn the_scrollback_setting_limits_the_history() {
        let (_harness, _recorder, _grid, _cell, session) = mounted_with(
            "",
            Settings {
                scrollback: 200.0,
                ..Settings::default()
            },
        );
        session.feed_for_test(&lines(1000));
        assert_eq!(session.terminal.borrow().scrollback_len(), 200);
        session.form.scrollback.set(100.0);
        session.apply_scrollback();
        assert_eq!(session.terminal.borrow().scrollback_len(), 100);
        // 整台终端复位以后上限还在
        session.feed_for_test("\x1bc");
        session.feed_for_test(&lines(500));
        assert_eq!(session.terminal.borrow().scrollback_len(), 100);
    }

    /// 选中后自动复制：开了才复制；程序抓着鼠标时（Shift 之外）不碰剪贴板
    #[test]
    fn copy_on_select_copies_when_enabled_only() {
        let select = |copy: bool| {
            let (harness, _recorder, grid, cell, session) = mounted_with(
                "hello world",
                Settings {
                    copy_on_select: copy,
                    ..Settings::default()
                },
            );
            session.feed_for_test("");
            aureole::clipboard::write_text("旧的");
            let (from, to) = (center(grid, cell, 0, 0), center(grid, cell, 0, 4));
            harness.drag([from.0 - cell.0 * 0.4, from.1], [to.0 + cell.0 * 0.4, to.1]);
            aureole::clipboard::read_text()
        };
        assert_eq!(select(true).as_deref(), Some("hello"));
        assert_eq!(select(false).as_deref(), Some("旧的"));
    }

    /// 右键粘贴：开了才粘贴
    #[test]
    fn right_click_pastes_when_enabled_only() {
        for (enabled, expected) in [(true, "from clipboard"), (false, "")] {
            let (harness, recorder, grid, cell, _session) = mounted_with(
                "",
                Settings {
                    right_click_paste: enabled,
                    ..Settings::default()
                },
            );
            aureole::clipboard::write_text("from clipboard");
            let (x, y) = center(grid, cell, 3, 3);
            harness.right_click(x, y);
            assert_eq!(recorder.take(), expected, "右键粘贴 {enabled}");
        }
    }

    /// 程序响铃：设成画面闪一下时闪（一批输出里响几次也只闪一次），关闭时什么也不做；
    /// 设置马上生效，不用重开会话
    #[test]
    fn the_bell_follows_the_setting() {
        let (_harness, _recorder, _grid, _cell, session) = mounted_with(
            "",
            Settings {
                bell: Bell::Flash,
                ..Settings::default()
            },
        );
        session.feed_for_test("\x07\x07\x07");
        assert_eq!(session.flashes.get_untracked(), 1);
        session.feed_for_test("no bell");
        assert_eq!(session.flashes.get_untracked(), 1);
        session.feed_for_test("\x07");
        assert_eq!(session.flashes.get_untracked(), 2);
        session.form.bell.set(Bell::Off);
        session.feed_for_test("\x07");
        assert_eq!(session.flashes.get_untracked(), 2, "关了就不闪");
        session.form.bell.set(Bell::Flash);
        session.feed_for_test("\x07");
        assert_eq!(session.flashes.get_untracked(), 3);
    }

    /// 响铃闪一下：底色亮一下，再恢复
    #[test]
    fn a_flash_lights_the_background_briefly() {
        let (harness, _recorder, grid, _cell, session) = mounted_with(
            "",
            Settings {
                bell: Bell::Flash,
                ..Settings::default()
            },
        );
        let corner = |harness: &Harness| {
            harness
                .screenshot()
                .pixel((grid.x + grid.width + 6.0) as u32, (grid.y + 6.0) as u32)
                .0
        };
        harness.advance(secs(1.0));
        let normal = corner(&harness);
        session.feed_for_test("\x07");
        harness.advance(ms(60));
        let flashing = corner(&harness);
        assert_ne!(normal, flashing, "闪的时候底色变了");
        harness.advance(secs(1.0));
        assert_eq!(corner(&harness), normal, "闪完恢复");
    }

    /// 默认光标形状、闪烁听设置的；程序自己指定了形状就听程序的，程序说「回到默认」又交还给设置
    #[test]
    fn the_cursor_shape_defaults_to_the_setting_and_programs_can_override_it() {
        let (_harness, _recorder, _grid, _cell, session) = mounted_with(
            "",
            Settings {
                cursor: CursorStyle::Bar,
                cursor_blink: false,
                ..Settings::default()
            },
        );
        let caret = || session.grid.caret();
        assert_eq!(caret().shape, aureole::CaretShape::Bar);
        assert!(!caret().blinking, "设置里关了闪烁");
        session.feed_for_test("\x1b[4 q");
        assert_eq!(
            caret().shape,
            aureole::CaretShape::Underline,
            "程序指定的下划线"
        );
        session.feed_for_test("\x1b[0 q");
        assert_eq!(
            caret().shape,
            aureole::CaretShape::Bar,
            "回到默认：设置的竖线"
        );
        session.form.cursor.set(CursorStyle::Block);
        session.form.cursor_blink.set(true);
        session.restyle();
        assert_eq!(caret().shape, aureole::CaretShape::Block, "改设置马上换");
        assert!(caret().blinking);
    }

    /// 配色：同一个 ANSI 红色，浅色和深色配色画出不同的颜色，换配色时屏幕上已有的字也跟着重画
    #[test]
    fn changing_the_palette_repaints_what_is_already_on_screen() {
        let (_harness, _recorder, _grid, _cell, session) = mounted_with("", Settings::default());
        session.feed_for_test("\x1b[31mred\x1b[0m");
        let red = || session.grid.cell(0, 0).expect("有字").style.foreground;
        let light = red();
        assert!(light.is_some());
        crate::theme::dark().set(true);
        session.restyle();
        assert_ne!(red(), light, "深色配色里红色不一样");
        crate::theme::dark().set(false);
        session.restyle();
        assert_eq!(red(), light);
    }

    /// 右键：没抓鼠标、没开「右键粘贴」时弹菜单（回调拿到点的位置）；开了右键粘贴就粘贴、不弹；
    /// 程序抓着鼠标时归程序、不弹（按着 Shift 例外）
    #[test]
    fn right_click_opens_the_menu_unless_pasting_or_the_program_wants_the_mouse() {
        let menus = |setup: &str, paste: bool, modifiers: Modifiers| {
            let recorder = Arc::new(Recorder::default());
            let session = Session::new(
                {
                    let mut terminal = Terminal::new(80, 24);
                    terminal.feed(setup.as_bytes());
                    terminal
                },
                CharGrid::new(80, 24),
                Some(Pty::from_backend(recorder)),
                Hooks::none(),
                Form::new(&Settings {
                    right_click_paste: paste,
                    ..Settings::default()
                }),
            );
            let opened = Rc::new(Cell::new(None));
            let seen = opened.clone();
            let harness = Harness::new(
                960.0,
                600.0,
                view(session, || true, move |at| seen.set(Some(at))),
            );
            harness.frame();
            let grid = harness.bounds("grid").expect("网格在");
            let [x, y] = grid.center();
            harness.set_modifiers(modifiers);
            harness.mouse_down(x, y, MouseButton::Right);
            harness.mouse_up(MouseButton::Right);
            opened
                .get()
                .map(|at| (at[0] - x).abs() < 1.0 && (at[1] - y).abs() < 1.0)
        };
        assert_eq!(
            menus("", false, Modifiers::default()),
            Some(true),
            "默认弹菜单，位置是点的地方"
        );
        assert_eq!(
            menus("", true, Modifiers::default()),
            None,
            "右键粘贴时不弹"
        );
        assert_eq!(
            menus("\x1b[?1000h", false, Modifiers::default()),
            None,
            "程序抓着鼠标不弹"
        );
        assert_eq!(
            menus("\x1b[?1000h", false, Modifiers::SHIFT),
            Some(true),
            "按着 Shift 还是弹"
        );
    }

    /// 复制只复制选中的；全选选满屏幕；清屏连历史一起清并让 shell 重画
    #[test]
    fn copy_select_all_and_clear() {
        let (_harness, recorder, _grid, _cell, session) = mounted_with("", Settings::default());
        assert_eq!(session.selected_text(), None);
        session.feed_for_test("hello\r\nworld");
        aureole::clipboard::write_text("旧的");
        session.copy_selection();
        assert_eq!(
            aureole::clipboard::read_text().as_deref(),
            Some("旧的"),
            "没选中不动剪贴板"
        );
        session.select_all();
        let selected = session.selected_text().expect("全选了");
        assert!(
            selected.contains("hello") && selected.contains("world"),
            "{selected:?}"
        );
        session.copy_selection();
        assert!(
            aureole::clipboard::read_text()
                .expect("有")
                .contains("hello")
        );
        session.feed_for_test(&lines(100));
        assert!(session.terminal.borrow().scrollback_len() > 0);
        session.clear_screen();
        assert_eq!(session.terminal.borrow().scrollback_len(), 0, "历史清了");
        assert_eq!(
            session
                .screen_text_for_test()
                .replace(['\0', ' ', '\n'], ""),
            ""
        );
        assert_eq!(recorder.take(), "\x0c", "让 shell 重画提示符");
    }
}
