//! 键盘和输入：按键编码、粘贴（含多行确认）、输入法提交、把路径或命令写在提示符处。

use super::*;

/// 粘贴的快捷键：macOS 是 ⌘V，Windows、Linux 是 Ctrl+Shift+V（Ctrl+V 要留给 shell）
fn is_paste(event: &KeyEvent) -> bool {
    let is_v =
        matches!(&event.key, Key::Character(text) if text.as_str().eq_ignore_ascii_case("v"));
    let modifiers = event.modifiers;
    is_v && (modifiers.has_super() || (modifiers.has_control() && modifiers.has_shift()))
}

/// 文字里除了结尾的那个换行之外，还有换行
pub fn has_inner_newline(text: &str) -> bool {
    text.trim_end_matches(['\n', '\r']).contains(['\n', '\r'])
}

impl Session {
    /// 网格有焦点时的按键。返回 `true` 表示终端收下了（不再往外传）
    pub(super) fn key(self: &Rc<Self>, event: &KeyEvent) -> bool {
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

    /// 从访达拖来的文件落在这个终端上：把它们的路径写在光标处，按 shell 的规矩引好，空格隔开，不按回车。
    /// 只有本机的终端写（服务器、容器里没有这些路径）；带控制字符或者不是 UTF-8 的路径跳过。返回写了没有
    pub fn drop_files(&self, paths: &[std::path::PathBuf]) -> bool {
        if !self.local.get() {
            return false;
        }
        let words: Vec<String> = paths
            .iter()
            .filter_map(|path| path.to_str())
            .filter_map(crate::files::shell_word)
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
