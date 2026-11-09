//! 提醒和确认：窗口在后台时的系统通知（命令跑完、程序要求通知），和多行粘贴的确认。

use super::*;

/// 等用户确认的一次粘贴
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPaste {
    pub tab: u64,
    pub pane: u64,
    pub text: String,
}

/// 「2 分 10 秒」「45 秒」「1 小时 3 分」
pub(super) fn elapsed_text(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds} 秒"),
        60..=3599 => format!("{} 分 {} 秒", seconds / 60, seconds % 60),
        _ => format!("{} 小时 {} 分", seconds / 3600, seconds % 3600 / 60),
    }
}

impl Workspace {
    /// 确认框里点了「粘贴」：照原样粘贴进它问的那个终端
    pub fn confirm_paste(&self) {
        let Some(pending) = self.pending_paste.get_untracked() else {
            return;
        };
        self.pending_paste.set(None);
        let session = self
            .tabs
            .get_untracked()
            .into_iter()
            .find(|tab| tab.id == pending.tab)
            .and_then(|tab| tab.layout.get_untracked().session(pending.pane));
        if let Some(session) = session {
            session.paste_text(&pending.text);
            session.request_focus();
        }
    }

    /// 用户这会儿正看着这个终端吗：窗口在前台、标签是当前的、拆分过的话它是聚焦的那个
    pub(super) fn watching(&self, tab: u64, pane: u64) -> bool {
        let window_front = self.window.is_none_or(|window| window.is_focused());
        window_front
            && !self.settings_open.get_untracked()
            && self.active.get_untracked() == tab
            && self
                .tabs
                .get_untracked()
                .iter()
                .find(|candidate| candidate.id == tab)
                .is_none_or(|candidate| candidate.focus.get_untracked() == pane)
    }

    /// 会话要提醒用户：设置里开着、用户又不在看这个终端时发系统通知，点通知回到这个终端
    pub fn alert(&self, tab: u64, pane: u64, alert: crate::app::Alert) {
        use crate::app::Alert;
        if !self.form.notifications.get_untracked() || self.watching(tab, pane) {
            return;
        }
        let name = self
            .titles
            .get_untracked()
            .get(&tab)
            .cloned()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "终端".to_string());
        let (title, body) = match alert {
            Alert::Finished {
                seconds,
                exit,
                command,
            } => {
                let title = match exit {
                    Some(code) if code != 0 => format!("命令失败（退出码 {code}）"),
                    _ => "命令已完成".to_string(),
                };
                let what = if command.is_empty() { name } else { command };
                (title, format!("{what} · 用时 {}", elapsed_text(seconds)))
            }
            Alert::Message { title, body } if title.is_empty() => (name, body),
            Alert::Message { title, body } => (format!("{name} · {title}"), body),
        };
        let (this, window) = (self.this.clone(), self.window);
        let shown = aureole::Notification::new(title)
            .body(body)
            .on_click(move || {
                if let Some(workspace) = this.upgrade() {
                    workspace.select(tab);
                    workspace.focus_pane(tab, pane);
                }
                if let Some(window) = window {
                    window.focus();
                }
            });
        // 发不了（macOS 没打包成 .app、系统没开通知）只是没有提醒，不是错误
        let _ = shown.show();
    }
}
