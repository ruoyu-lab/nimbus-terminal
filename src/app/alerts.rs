//! 会话想提醒用户的事：前台命令跑完了、程序要求弹通知、前台程序响铃。弹不弹、弹给谁由外面（`tabs`）决定。

use super::*;

/// 会话想提醒用户的事
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alert {
    /// 前台的命令（任何 shell 之外的程序）跑了这么久，现在结束了
    Finished {
        seconds: u64,
        /// 退出码（有命令标记才知道）
        exit: Option<i32>,
        /// 敲的命令（有命令标记才知道，没有是空的）
        command: String,
    },
    /// 程序自己要求的通知（OSC 9、OSC 777），或者前台程序响了铃（`title` 是空的）
    Message { title: String, body: String },
}

/// 前台命令至少跑这么久，结束时才算「跑完了」：`ls` 之类一闪而过的不提醒
pub const FINISHED_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// 程序自己发的通知，两条之间至少隔这么久：它刷屏时不跟着刷
const ALERT_SPACING: std::time::Duration = std::time::Duration::from_secs(3);

impl Session {
    /// 挂在一个不随界面卸载的作用域上。从文件夹右键菜单里创建时，菜单的作用域马上就释放，
    /// 读任务要是记在菜单上，会被一起取消：shell 已经起来（光标在闪），提示符却永远到不了屏幕
    /// 每秒看一眼前台有没有命令在跑。手里只拿弱引用：会话释放了任务自己结束
    pub(super) fn start_watch(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let task = session_scope().run(|| {
            aureole::spawn_local(async move {
                loop {
                    aureole::sleep(ms(1000)).await;
                    let Some(session) = weak.upgrade() else { break };
                    if session.closed.get() {
                        break;
                    }
                    session.watch_busy(session.is_busy(), std::time::Instant::now());
                }
            })
        });
        *self.watcher.borrow_mut() = Some(task);
    }

    /// 前台命令的状态变了：从闲到忙记下时间，从忙到闲时跑得够久就报「跑完了」
    pub(super) fn watch_busy(&self, busy: bool, now: std::time::Instant) {
        // 有命令标记时靠标记报（知道退出码和命令），这里不再重复
        if self.terminal.borrow().has_command_marks() {
            self.busy_since.set(None);
            return;
        }
        match (busy, self.busy_since.get()) {
            (true, None) => self.busy_since.set(Some(now)),
            (false, Some(since)) => {
                self.busy_since.set(None);
                let ran = now.saturating_duration_since(since);
                if ran >= FINISHED_AFTER {
                    (self.hooks.alert)(Alert::Finished {
                        seconds: ran.as_secs(),
                        exit: None,
                        command: String::new(),
                    });
                }
            }
            _ => {}
        }
    }

    /// 程序自己要求的通知、前台程序响铃：交给外面决定弹不弹。两条之间隔一会儿
    pub(super) fn alert_message(&self, title: String, body: String, now: std::time::Instant) {
        if self
            .last_alert
            .get()
            .is_some_and(|last| now.saturating_duration_since(last) < ALERT_SPACING)
        {
            return;
        }
        self.last_alert.set(Some(now));
        (self.hooks.alert)(Alert::Message { title, body });
    }
}
