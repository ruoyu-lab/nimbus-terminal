//! 终端内查找（⌘F）的状态和动作：命中怎么找、怎么高亮、怎么跳。界面是 `chrome::find`。

use super::*;

/// 终端内查找的状态：查找条是界面，这里是它背后的东西
pub(super) struct Find {
    /// 查找条开着没有
    pub(super) open: Signal<bool>,
    /// 输入框里的字
    pub(super) query: Signal<String>,
    /// 「3/12」「无结果」，查找条显示它
    pub(super) status: Signal<String>,
    /// 命中，从老到新
    pub(super) matches: RefCell<Vec<Match>>,
    /// 当前是第几处（`matches` 的下标）
    pub(super) current: Cell<Option<usize>>,
    /// 当前那一处在哪：（行的编号，起列）。行的编号不随历史增长变：新输出把行往上顶时，
    /// 重新找一遍之后靠它认出还是原来那一处
    pub(super) anchor: Cell<Option<(u64, usize)>>,
    /// 命中变了，下一次画要整屏重画（不然只重画变了的行，高亮留在原地）
    pub(super) dirty: Cell<bool>,
    /// 上一次找的时候的 `output_version`：屏幕内容变了才需要再找
    pub(super) seen_version: Cell<u64>,
}

impl Session {
    /// 查找条开着没有（界面绑它）
    pub fn find_open(&self) -> Signal<bool> {
        self.find.open
    }

    pub fn find_query(&self) -> Signal<String> {
        self.find.query
    }

    pub fn find_status(&self) -> Signal<String> {
        self.find.status
    }

    /// 打开查找条（⌘F）。屏幕上选着字（单行）就拿它当要找的，没有就接着上次的
    pub fn open_find(&self) {
        if self.waiting.get() {
            return;
        }
        if let Some(text) = self.selected_text()
            && !text.contains(['\n', '\r'])
        {
            self.find.query.set(text);
        }
        self.find.open.set(true);
        self.search(true);
        self.flush();
    }

    /// 关掉查找条：高亮撤掉，位置留在原地
    pub fn close_find(&self) {
        if !self.find.open.get_untracked() {
            return;
        }
        self.find.open.set(false);
        self.find.matches.borrow_mut().clear();
        self.find.current.set(None);
        self.find.anchor.set(None);
        self.find.dirty.set(true);
        self.flush();
    }

    /// 输入框里的字变了：重新找，跳到最近（最新）的一处
    pub fn find_changed(&self) {
        if self.find.open.get_untracked() {
            self.search(true);
            self.flush();
        }
    }

    /// 上一处（往更老的方向）或者下一处（往更新的方向），到头了绕回另一端
    pub fn find_step(&self, older: bool) {
        let count = self.find.matches.borrow().len();
        if count == 0 {
            return;
        }
        let next = match self.find.current.get() {
            None => count - 1,
            Some(current) if older => (current + count - 1) % count,
            Some(current) => (current + 1) % count,
        };
        self.jump_to_match(next);
        self.flush();
    }

    /// 行的编号：历史里每一行有个不变的号（这一行是历史上第几个被挤出屏幕的），屏幕上的行接在后面
    pub(super) fn line_id(terminal: &Terminal, line: usize) -> u64 {
        let dropped = terminal.scrollback_total() - terminal.scrollback_len() as u64;
        dropped + line as u64
    }

    /// 按输入框里的字重新找一遍。`jump` 为真跳到最新的一处；否则尽量留在原来那一处
    pub(super) fn search(&self, jump: bool) {
        let query = self.find.query.get_untracked();
        let found = {
            let terminal = self.terminal.borrow();
            search::find(terminal.screen(), &query, search::is_case_sensitive(&query))
        };
        let capped = found.len() >= search::LIMIT;
        self.find.seen_version.set(self.output_version.get());
        self.find.dirty.set(true);
        let keep = if jump { None } else { self.find.anchor.get() };
        let current = keep.and_then(|(id, column)| {
            let terminal = self.terminal.borrow();
            found
                .iter()
                .position(|hit| (Self::line_id(&terminal, hit.line), hit.start) == (id, column))
        });
        *self.find.matches.borrow_mut() = found;
        let count = self.find.matches.borrow().len();
        match (current, count) {
            (_, 0) => {
                self.find.current.set(None);
                self.find.anchor.set(None);
                self.find.status.set(if query.is_empty() {
                    String::new()
                } else {
                    "无结果".into()
                });
            }
            (Some(index), _) => self.set_current(index, capped),
            (None, _) => {
                self.set_current(count - 1, capped);
                if jump {
                    self.scroll_to_match(count - 1);
                }
            }
        }
    }

    pub(super) fn set_current(&self, index: usize, capped: bool) {
        let hit = self.find.matches.borrow()[index];
        let id = Self::line_id(&self.terminal.borrow(), hit.line);
        self.find.current.set(Some(index));
        self.find.anchor.set(Some((id, hit.start)));
        let count = self.find.matches.borrow().len();
        self.find.status.set(format!(
            "{}/{count}{}",
            index + 1,
            if capped { "+" } else { "" }
        ));
    }

    pub(super) fn jump_to_match(&self, index: usize) {
        let capped = self.find.matches.borrow().len() >= search::LIMIT;
        self.set_current(index, capped);
        self.find.dirty.set(true);
        self.scroll_to_match(index);
    }

    /// 翻到能看见第 `index` 处：在屏幕上的不用翻，在历史里的让它落在中间
    pub(super) fn scroll_to_match(&self, index: usize) {
        let Some(hit) = self.find.matches.borrow().get(index).copied() else {
            return;
        };
        let (history, rows) = {
            let terminal = self.terminal.borrow();
            (terminal.scrollback_len(), terminal.size().1)
        };
        let offset = if hit.line >= history {
            0
        } else {
            let top = hit.line.saturating_sub(rows / 2);
            history.saturating_sub(top)
        };
        self.offset.set(offset);
        self.grid.set_selection(None);
    }
}
