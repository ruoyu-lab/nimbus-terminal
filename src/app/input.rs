//! 键盘和输入：按键编码、复制、粘贴（含多行确认）、输入法提交、把路径或命令写在提示符处。

use super::*;

/// Windows、Linux 的那一套剪贴板键（Ctrl+Shift+C、Ctrl+Insert、Shift+Insert）。
/// macOS 上不认：⌘C、⌘A 由网格自己处理（终端不收 ⌘ 的组合键），Ctrl+Shift+C 照旧交给终端
const OTHER_CLIPBOARD_KEYS: bool = !cfg!(target_os = "macos");

/// 按的是不是这个字母（大小写都算：按着 Shift 时报上来的是大写）
fn is_letter(event: &KeyEvent, letter: &str) -> bool {
    matches!(&event.key, Key::Character(text) if text.as_str().eq_ignore_ascii_case(letter))
}

/// 按的是不是只带 `modifiers` 这几个修饰键的 Insert
fn is_insert(event: &KeyEvent, modifiers: Modifiers) -> bool {
    matches!(event.key, Key::Named(NamedKey::Insert)) && event.modifiers == modifiers
}

/// 粘贴的快捷键：macOS 是 ⌘V，Windows、Linux 是 Ctrl+Shift+V（Ctrl+V 要留给 shell），还有老习惯的 Shift+Insert
fn is_paste(event: &KeyEvent) -> bool {
    let modifiers = event.modifiers;
    let chord = modifiers.has_super() || (modifiers.has_control() && modifiers.has_shift());
    (is_letter(event, "v") && chord) || (OTHER_CLIPBOARD_KEYS && is_insert(event, Modifiers::SHIFT))
}

/// 复制的快捷键（Windows、Linux）：Ctrl+Shift+C，老习惯的 Ctrl+Insert。单独的 Ctrl+C 是给 shell 的中断
fn is_copy(event: &KeyEvent) -> bool {
    OTHER_CLIPBOARD_KEYS
        && ((is_letter(event, "c") && event.modifiers == Modifiers::CONTROL | Modifiers::SHIFT)
            || is_insert(event, Modifiers::CONTROL))
}

/// 全选的快捷键（Windows、Linux）：Ctrl+Shift+A。单独的 Ctrl+A 是 shell 里的回到行首
fn is_select_all(event: &KeyEvent) -> bool {
    OTHER_CLIPBOARD_KEYS
        && is_letter(event, "a")
        && event.modifiers == Modifiers::CONTROL | Modifiers::SHIFT
}

/// 文字里除了结尾的那个换行之外，还有换行
pub fn has_inner_newline(text: &str) -> bool {
    text.trim_end_matches(['\n', '\r']).contains(['\n', '\r'])
}

impl Session {
    /// 网格有焦点时的按键。返回 `true` 表示终端收下了（不再往外传）
    pub(super) fn key(self: &Rc<Self>, event: &KeyEvent) -> bool {
        // 复制、全选不碰 pty：连接断了也能复制屏幕上的字（比如出错的原因）
        if is_copy(event) {
            self.copy_selection();
            return true;
        }
        if is_select_all(event) {
            self.select_all();
            return true;
        }
        // 连接断了，等着重连：回车重连，别的键都不往已经没有人听的 pty 里写
        if self.waiting.get() && self.reconnect.borrow().is_some() {
            if matches!(event.key, Key::Named(NamedKey::Enter)) {
                self.reconnect_now();
            }
            return true;
        }
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
        // 写给 shell 的键，网格自己不再处理：它把主修饰键加 C、A 当复制、全选，Windows、Linux 上主修饰键是 Ctrl，
        // 而 Ctrl+C、Ctrl+A 是给 shell 的（不拦的话按 Ctrl+A 回行首，整屏也跟着被选中）
        event.prevent_builtin(Builtin::EDIT);
        // 打字、按方向键都是「继续用 shell 了」：选区作废，回看的话回到最新的输出
        self.grid.set_selection(None);
        self.write(&bytes);
        self.snap_to_bottom();
        true
    }

    /// 把剪贴板里的字粘贴进去
    pub fn paste(&self) {
        let Some(text) = aureole::clipboard::read_text() else {
            return;
        };
        let bracketed = self.terminal.borrow().modes.bracketed_paste;
        // 没开括号粘贴时，shell 会把每一行都当成命令立刻执行：多行的先问一下
        if !bracketed
            && self.form.confirm_paste.get_untracked()
            && has_inner_newline(&text)
            && let Some(confirm) = &self.hooks.confirm_paste
        {
            confirm(text);
            return;
        }
        self.paste_text(&text);
    }

    /// 把 `text` 当粘贴的内容写给程序（括号粘贴、换行、控制字符都照粘贴的规矩处理）
    pub fn paste_text(&self, text: &str) {
        let bracketed = self.terminal.borrow().modes.bracketed_paste;
        self.write(&keys::paste_bytes(text, bracketed));
        self.snap_to_bottom();
    }

    /// 把一条命令写在提示符处（命令历史里选中的）：不按回车，由用户决定要不要改、要不要执行
    pub fn insert_command(&self, command: &str) {
        let bracketed = self.terminal.borrow().modes.bracketed_paste;
        // 换行换成空格：历史里的多行命令别一粘贴就执行
        let single: String = command
            .chars()
            .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
            .collect();
        self.grid.set_selection(None);
        self.write(&keys::paste_bytes(&single, bracketed));
        self.snap_to_bottom();
    }

    /// 把一个路径写在光标处（从文件页拖进来的）：按 shell 的规矩引好，后面跟一个空格，不按回车。
    /// 路径带控制字符（换行会变成回车，远端的文件名能让你执行任何命令）就一个字也不写，返回 `false`
    pub fn insert_path(&self, path: &str) -> bool {
        let Some(word) = crate::files::shell_word(path) else {
            return false;
        };
        self.grid.set_selection(None);
        self.write(format!("{word} ").as_bytes());
        self.snap_to_bottom();
        true
    }

    /// 从访达（资源管理器）拖来的文件落在这个终端上：把它们的路径写在光标处，按本机 shell 的规矩引好，空格隔开，不按回车。
    /// 只有本机的终端写（服务器、容器里没有这些路径）；带控制字符或者不是 UTF-8 的路径跳过。返回写了没有。
    ///
    /// macOS、Linux 上是 `sh` 的规矩。Windows 上 cmd、PowerShell 的写法不一样，会话又不记得自己起的是哪个程序，
    /// 就按设置里新终端用的那个算（设置后来改过、shell 里又开了另一个 shell 时会猜错；
    /// [`crate::files::local_word`] 尽量写成两边都对的样子，猜错了也只有带 `$`、`%` 这类字的名字受影响）
    pub fn drop_files(&self, paths: &[std::path::PathBuf]) -> bool {
        if !self.local.get() {
            return false;
        }
        let shell = if cfg!(windows) {
            let program = (self.form.shell.get_untracked() == crate::settings::ShellKind::Program)
                .then(|| self.form.shell_program.get_untracked().trim().to_string())
                .filter(|program| !program.is_empty())
                .unwrap_or_else(crate::pty::default_shell);
            crate::files::LocalShell::of_program(&program)
        } else {
            crate::files::LocalShell::Sh
        };
        let words: Vec<String> = paths
            .iter()
            .filter_map(|path| path.to_str())
            .filter_map(|path| crate::files::local_word(path, shell))
            .collect();
        if words.is_empty() {
            return false;
        }
        self.grid.set_selection(None);
        self.write(format!("{} ", words.join(" ")).as_bytes());
        self.snap_to_bottom();
        true
    }

    /// 输入法提交的文字
    pub(super) fn commit_text(&self, text: &str) {
        self.grid.set_selection(None);
        self.write(text.as_bytes());
        self.snap_to_bottom();
    }

    /// 要这个终端拿回键盘焦点（从文件页拖了东西进来之后）
    pub fn request_focus(&self) {
        self.focus_requests.update(|requests| *requests += 1);
    }
}
