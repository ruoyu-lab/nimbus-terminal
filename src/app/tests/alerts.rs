//! 提醒：命令跑完、程序要求通知。

use super::*;

/// 前台命令跑够 10 秒才算「跑完了」：一闪而过的不提醒，跑得久的结束时提醒一次，带上用了多久
#[test]
fn finished_alert_needs_a_long_enough_run() {
    let (session, alerts) = session_with_alerts();
    let start = std::time::Instant::now();
    let after = |seconds: u64| start + std::time::Duration::from_secs(seconds);
    session.watch_busy(true, start);
    session.watch_busy(false, after(3));
    assert!(alerts.borrow().is_empty(), "3 秒的不提醒");
    session.watch_busy(true, after(10));
    session.watch_busy(true, after(15));
    assert!(alerts.borrow().is_empty(), "还在跑");
    session.watch_busy(false, after(22));
    assert_eq!(
        *alerts.borrow(),
        [Alert::Finished {
            seconds: 12,
            exit: None,
            command: String::new()
        }]
    );
    session.watch_busy(false, after(30));
    assert_eq!(alerts.borrow().len(), 1, "没有命令在跑时不重复提醒");
}

/// 程序自己要求的通知（OSC 9 / 777）转给外面；刷屏时两条之间要隔一会儿；前台有程序时响铃也算，shell 提示符前的不算
#[test]
fn programs_can_ask_for_notifications() {
    let (session, alerts) = session_with_alerts();
    session.feed_for_test("\x1b]9;build done\x07\x1b]9;second\x07");
    assert_eq!(
        *alerts.borrow(),
        [Alert::Message {
            title: String::new(),
            body: "build done".into()
        }],
        "紧挨着的第二条被压住"
    );
    let (session, alerts) = session_with_alerts();
    session.feed_for_test("\x07");
    assert!(alerts.borrow().is_empty(), "没有前台程序时响铃不提醒");
    session.watch_busy(true, std::time::Instant::now());
    session.feed_for_test("\x07");
    assert_eq!(
        *alerts.borrow(),
        [Alert::Message {
            title: String::new(),
            body: "终端响铃".into()
        }]
    );
    let (session, alerts) = session_with_alerts();
    session.feed_for_test("\x1b]777;notify;Claude;需要你确认\x07");
    assert_eq!(
        *alerts.borrow(),
        [Alert::Message {
            title: "Claude".into(),
            body: "需要你确认".into()
        }]
    );
}
