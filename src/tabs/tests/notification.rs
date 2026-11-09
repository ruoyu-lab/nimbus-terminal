//! 后台通知。

use super::*;

/// 后台通知：用户在看的那个终端不弹；别的标签里命令跑完了弹，点通知回到那个标签；设置里关了就都不弹
#[test]
fn background_notifications_go_to_the_tab_you_are_not_looking_at() {
    use crate::app::Alert;
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.frame();
    let [first, second] = [ids(&workspace)[0], ids(&workspace)[1]];
    assert_eq!(workspace.active.get_untracked(), second);
    let pane = |tab: u64| {
        workspace
            .tabs
            .get_untracked()
            .iter()
            .find(|candidate| candidate.id == tab)
            .expect("有标签")
            .layout
            .get_untracked()
            .first_id()
    };
    workspace.alert(
        second,
        pane(second),
        Alert::Finished {
            seconds: 75,
            exit: None,
            command: String::new(),
        },
    );
    assert!(harness.notifications().is_empty(), "正看着的不弹");
    workspace.alert(
        first,
        pane(first),
        Alert::Finished {
            seconds: 75,
            exit: None,
            command: String::new(),
        },
    );
    let shown = harness.notifications();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title, "命令已完成");
    assert!(shown[0].body.contains("1 分 15 秒"), "{}", shown[0].body);
    // 点通知：切回那个标签
    assert!(harness.click_notification(0));
    harness.frame();
    assert_eq!(workspace.active.get_untracked(), first);
    // 程序自己要求的通知带上标题
    workspace.select(second);
    workspace.alert(
        first,
        pane(first),
        Alert::Message {
            title: "Claude".into(),
            body: "需要你确认".into(),
        },
    );
    let shown = harness.notifications();
    assert_eq!(shown.last().map(|n| n.body.as_str()), Some("需要你确认"));
    assert!(shown.last().is_some_and(|n| n.title.ends_with("Claude")));
    // 设置里关了：不弹
    let before = harness.notifications().len();
    workspace.form.notifications.set(false);
    workspace.alert(
        first,
        pane(first),
        Alert::Finished {
            seconds: 99,
            exit: None,
            command: String::new(),
        },
    );
    assert_eq!(harness.notifications().len(), before);
}

/// 通知里的用时写法
#[test]
fn elapsed_text_reads_naturally() {
    assert_eq!(crate::tabs::alerts::elapsed_text(45), "45 秒");
    assert_eq!(crate::tabs::alerts::elapsed_text(130), "2 分 10 秒");
    assert_eq!(crate::tabs::alerts::elapsed_text(3780), "1 小时 3 分");
}
