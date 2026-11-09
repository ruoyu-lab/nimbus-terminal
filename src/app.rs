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

use crate::links;
use crate::pty::{self, ByteQueue, Chunk, Pty};
use crate::settings::{Bell, Form};
use crate::target::Target;
use crate::view;
use crate::vt::image::Pixels;
use crate::vt::mouse::{Action, Button, Report};
use crate::vt::search::{self, Match};
use crate::vt::{MouseMode, Terminal, keys, mouse};

mod alerts;
mod commands;
mod element;
mod find;
mod hyperlinks;
mod images;
mod input;
mod launch;
mod mouse_input;
mod scroll;
mod selection;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use alerts::{Alert, FINISHED_AFTER};
pub use element::view;
use find::Find;
pub use images::VisibleImage;
use images::terminal_image;
pub use launch::{Hooks, Launch, Spawned, spawn};

/// 终端区域边缘留的空白（逻辑像素）
pub const PADDING: f32 = 12.0;
/// 一轮最多处理这么多字节就先让出主线程去画（`yes`、`cat 大文件` 时界面不卡）
const BATCH_BUDGET: usize = 256 * 1024;

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
    /// 外面（拖放、菜单）要这个终端拿键盘焦点的次数：视图盯着它，加一就聚焦。
    /// 终端该不该聚焦（`wanted`）只在「聚焦的是哪个终端」变的时候重新算；拖放这类动作把焦点弄丢了，
    /// 而聚焦的终端没变，所以需要另外一条路明确地要回来
    focus_requests: Signal<u64>,
    /// 下一次画要把整屏的颜色重算（配色换了）
    repaint_all: Cell<bool>,
    /// 字体、字号变了、格子大小要重新量：记着变的时候网格的边界，边界变了才说明新的格子大小已经布局出来
    remeasure_from: Cell<Option<Rect>>,
    /// 屏幕内容每变一次加一：存终端内容时靠它知道这个会话自上次存以后有没有新输出
    output_version: Cell<u64>,
    /// 连接断了以后怎么重连（服务器的会话才有）。本机的会话没有：shell 退出了就是退出了
    reconnect: RefCell<Option<Rc<dyn Fn() -> Spawned>>>,
    /// 现在没有进程在跑，等用户按回车重连
    waiting: Cell<bool>,
    /// 终端内查找（⌘F）
    find: Find,
    /// 在命令之间跳到了哪一条（提示符的行编号）；打字、回到最新输出时清掉
    jump_mark: Cell<Option<u64>>,
    /// 这是本机的终端（⌘点击文件路径才打得开：服务器上的路径在本机找不到）
    local: Cell<bool>,
    /// 看得见的图片（界面叠在网格上画）
    images: Signal<Vec<VisibleImage>>,
    /// 每张图的解码来源：同一张图只建一次，换位置不重新解码
    image_sources: RefCell<std::collections::HashMap<u64, ImageSource>>,
    /// 指针现在在哪一格（不在终端上是 `None`）
    pointer_cell: Cell<Option<(usize, usize)>>,
    /// 右键点在哪个链接上（菜单里的「打开链接」「复制链接」用）：点的时候记下，没点在链接上是 `None`
    context_link: RefCell<Option<links::Link>>,
    /// 指针停着的链接：它在显示的第几行、哪几列
    hover: Cell<Option<view::Hover>>,
    /// 指针停着的链接变了：下一次画要整屏重画一遍
    hover_dirty: Cell<bool>,
    /// 指针是不是停在链接上（光标变成手指）
    hovering_link: Signal<bool>,
    /// 前台命令从什么时候开始跑（没有就是 shell 在等输入）
    busy_since: Cell<Option<std::time::Instant>>,
    /// 上一条程序自己发的通知是什么时候
    last_alert: Cell<Option<std::time::Instant>>,
    /// 看前台命令跑完没有的定时任务；会话关了就取消
    watcher: RefCell<Option<LocalTask>>,
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
            terminal.finish_restore();
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
            session.start_pump(queue);
        }
        session.start_watch();
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
            focus_requests: aureole::detached(|| signal(0)),
            repaint_all: Cell::new(false),
            remeasure_from: Cell::new(None),
            output_version: Cell::new(0),
            reconnect: RefCell::new(None),
            waiting: Cell::new(false),
            local: Cell::new(false),
            jump_mark: Cell::new(None),
            images: aureole::detached(|| signal(Vec::new())),
            image_sources: RefCell::new(std::collections::HashMap::new()),
            context_link: RefCell::new(None),
            pointer_cell: Cell::new(None),
            hover: Cell::new(None),
            hover_dirty: Cell::new(false),
            hovering_link: aureole::detached(|| signal(false)),
            busy_since: Cell::new(None),
            last_alert: Cell::new(None),
            watcher: RefCell::new(None),
            find: Find {
                open: aureole::detached(|| signal(false)),
                query: aureole::detached(|| signal(String::new())),
                status: aureole::detached(|| signal(String::new())),
                matches: RefCell::new(Vec::new()),
                current: Cell::new(None),
                anchor: Cell::new(None),
                dirty: Cell::new(false),
                seen_version: Cell::new(0),
            },
        })
    }

    /// 让这个会话能在断线后重连：`respawn` 每次返回一个新的连接
    pub fn set_reconnect(&self, respawn: Rc<dyn Fn() -> Spawned>) {
        *self.reconnect.borrow_mut() = Some(respawn);
    }

    /// 不连，先停在「已断开」：重启后恢复的服务器会话是这样的，连不连由用户按回车决定
    /// （悄悄连上去会在启动时就弹密码、host key 的提示）
    pub fn wait_for_reconnect(&self, message: &str) {
        self.terminal
            .borrow_mut()
            .feed(format!("\x1b[0m{message}\r\n").as_bytes());
        self.output_version.set(self.output_version.get() + 1);
        self.waiting.set(true);
        self.flush();
    }

    /// 按回车重连：起一个新的连接接到同一个屏幕上，之前的输出留在上面
    fn reconnect_now(self: &Rc<Self>) {
        let Some(respawn) = self.reconnect.borrow().clone() else {
            return;
        };
        let spawned = respawn();
        self.waiting.set(false);
        self.terminal.borrow_mut().feed(b"\x1b[0m\r\n");
        if let Some(error) = &spawned.error {
            self.terminal
                .borrow_mut()
                .feed(format!("起不了会话：{error}\r\n").as_bytes());
            self.waiting.set(true);
        }
        *self.pty.borrow_mut() = spawned.pty;
        if let Some(queue) = spawned.queue {
            self.start_pump(queue);
        }
        self.output_version.set(self.output_version.get() + 1);
        self.flush();
    }

    fn start_pump(self: &Rc<Self>, queue: ByteQueue) {
        let reading = self.clone();
        let task = session_scope().run(|| aureole::spawn_local(pump(reading, queue)));
        *self.pump.borrow_mut() = Some(task);
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

    /// shell 现在所在的目录（存下来下次启动在这里开；点相对路径时从这里算）。
    /// 本机的会话先看 shell 集成报的（OSC 7）：PowerShell 的 `cd` 不改进程的当前目录，问进程问不准。
    /// 这个提示符没报、报的是别的机器（在里面 ssh 到了服务器），再问 pty
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        if self.local.get()
            && let Some(reported) = self.terminal.borrow().reported_directory()
            && is_this_machine(&reported.host)
        {
            return Some(reported.path.clone());
        }
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
            Bell::Sound => {
                aureole::beep();
            }
            Bell::Flash => self.flashes.update(|count| *count += 1),
        }
    }

    /// 关掉会话（标签关了）：停掉读的任务、放掉 pty（`Drop` 会给 shell 发 `SIGHUP`）
    pub fn close(&self) {
        self.closed.set(true);
        if let Some(task) = self.pump.borrow_mut().take() {
            task.cancel();
        }
        if let Some(task) = self.watcher.borrow_mut().take() {
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
        let (reply, title, history, total, alternate, bell, alerts, finished) = {
            let mut terminal = self.terminal.borrow_mut();
            (
                terminal.take_output(),
                terminal.title.clone(),
                terminal.scrollback_len(),
                terminal.scrollback_total(),
                terminal.on_alternate,
                terminal.bell,
                terminal.take_alerts(),
                terminal.take_finished(),
            )
        };
        for done in finished {
            if let Some(record) = &self.hooks.record
                && !done.text.is_empty()
            {
                record(done.text.clone());
            }
            if done.seconds >= FINISHED_AFTER.as_secs() {
                (self.hooks.alert)(Alert::Finished {
                    seconds: done.seconds,
                    exit: done.exit,
                    command: done.text,
                });
            }
        }
        let now = std::time::Instant::now();
        for (title, body) in alerts {
            self.alert_message(title, body, now);
        }
        if bell != self.bell_seen.replace(bell) {
            self.ring();
            // 前台有程序在跑时响铃，多半是它在喊人（TUI 干完了活）；shell 提示符前的响铃不算
            if self.busy_since.get().is_some() {
                self.alert_message(String::new(), "终端响铃".to_string(), now);
            }
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
        // 查找开着、屏幕内容又变了：重新找一遍，高亮才不会留在已经不是那行字的地方
        if self.find.open.get_untracked()
            && self.find.seen_version.get() != self.output_version.get()
        {
            self.search(false);
        }
        self.update_hover(offset);
        let matches = self.find.matches.borrow();
        let highlights =
            (self.find.open.get_untracked() && !matches.is_empty()).then(|| view::Highlights {
                matches: &matches,
                current: self.find.current.get(),
            });
        view::paint(
            &mut self.terminal.borrow_mut(),
            &self.grid,
            self.look(),
            view::Frame {
                offset,
                shown_offset: self.shown_offset.get(),
                full: self.repaint_all.take() || self.find.dirty.take() || self.hover_dirty.take(),
                highlights,
                hover: self.hover.get(),
            },
        );
        drop(matches);
        self.refresh_images(offset);
        self.shown_offset.set(offset);
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
        // 图片按一格多大换算成占几行几列
        self.terminal.borrow_mut().set_cell_pixels(cell.0, cell.1);
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

    /// 清屏：连滚动历史一起清掉，再让 shell 重画提示符（Ctrl-L）
    pub fn clear_screen(&self) {
        self.grid.set_selection(None);
        self.terminal.borrow_mut().feed(b"\x1b[H\x1b[2J\x1b[3J");
        self.output_version.set(self.output_version.get() + 1);
        self.offset.set(0);
        self.flush();
        self.write(b"\x0c");
    }

    /// shell 退出了。出错退出（退出码不是 0）把原因写在屏幕上，让人看得到最后的输出；
    /// 然后交给外面决定：正常退出（`exit`、Ctrl-D）标签就关掉
    fn exited(&self, code: i32) {
        if self.closed.get() {
            return;
        }
        if code != 0 {
            let message = if self.reconnect.borrow().is_some() {
                self.waiting.set(true);
                format!("\r\n[连接已断开，退出码 {code}。按回车重新连接]\r\n")
            } else {
                format!("\r\n[进程已退出，退出码 {code}]\r\n")
            };
            self.terminal.borrow_mut().feed(message.as_bytes());
            self.output_version.set(self.output_version.get() + 1);
            self.flush();
        }
        (self.hooks.exited)(code);
    }
}

/// shell 报目录时写的主机是不是这台机器：没写、`localhost`、本机的名字（不分大小写，只比第一段：
/// `mac.local` 和 `mac` 算同一台）
fn is_this_machine(host: &str) -> bool {
    let first = |name: &str| name.split('.').next().unwrap_or("").to_string();
    host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || host_name().is_some_and(|name| first(&name).eq_ignore_ascii_case(&first(host)))
}

/// 本机的名字：Windows 是 `COMPUTERNAME`（和 PowerShell 的 `[Environment]::MachineName` 一样）
#[cfg(windows)]
fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
}

/// 本机的名字（`gethostname`）
#[cfg(unix)]
fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: 往栈上的缓冲里写，长度如实给；名字放不下时可能不以 0 结尾，下面找不到 0 就当没有
    let failed = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0;
    if failed {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    String::from_utf8(buffer[..end].to_vec())
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(not(any(unix, windows)))]
fn host_name() -> Option<String> {
    None
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
