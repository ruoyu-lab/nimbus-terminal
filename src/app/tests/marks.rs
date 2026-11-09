//! 命令标记：跳转、复制输出、恢复。

use super::*;

/// 重启后恢复的内容带着命令标记：能在旧命令之间跳、复制输出；恢复的命令不会重新记进命令历史
#[test]
fn restored_sessions_keep_their_command_marks() {
    let old = session_with_commands(4);
    let saved = old.transcript(1000);
    let recorded = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut hooks = Hooks::none();
    let sink = recorded.clone();
    hooks.record = Some(Rc::new(move |command| sink.borrow_mut().push(command)));
    let session = Session::start(
        Spawned {
            pty: None,
            queue: None,
            error: None,
        },
        40,
        5,
        hooks,
        Form::new(&crate::settings::Settings::default()),
        Some(&saved),
    );
    assert!(recorded.borrow().is_empty(), "恢复的命令不进历史");
    assert!(session.jump_prompt(true));
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd3") && text.contains("out3"))
    );
    assert!(session.jump_prompt(true));
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd2"))
    );
    assert_eq!(session.last_command_output().as_deref(), Some("out3"));
}

/// 在命令之间跳：每次选中一条命令（提示符加输出），一路往上到最老，再往下回到最新；命令都在一屏里时也看得见
#[test]
fn jumping_between_commands() {
    // 命令都在一屏里：不滚动，但选中的是那条命令
    let session = session_with_commands(2);
    assert!(session.jump_prompt(true));
    assert_eq!(session.offset.get(), 0);
    let selected = session.selected_text().expect("选中了命令");
    assert!(
        selected.contains("cmd1") && selected.contains("out1"),
        "{selected:?}"
    );
    assert!(!selected.contains("cmd0"));
    assert!(session.jump_prompt(true));
    let selected = session.selected_text().expect("选中了上一条");
    assert!(
        selected.contains("cmd0") && selected.contains("out0"),
        "{selected:?}"
    );
    assert!(!session.jump_prompt(true), "最老的那条之前没有了");
    // 往下：回到后面一条，再往下就回到最新并取消选中
    assert!(session.jump_prompt(false));
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd1"))
    );
    assert!(session.jump_prompt(false));
    assert!(session.selected_text().is_none());
    assert!(!session.jump_prompt(false), "已经在最新的了");

    // 命令多得滚进了历史：要翻回去
    let session = session_with_commands(8);
    assert!(session.jump_prompt(true));
    let mut jumps = 1;
    while session.jump_prompt(true) {
        jumps += 1;
        assert!(jumps < 20, "停得下来");
    }
    assert_eq!(jumps, 8, "八条命令都跳到过");
    assert!(session.offset.get() > 0, "翻回历史了");
    assert!(
        session
            .selected_text()
            .is_some_and(|text| text.contains("cmd0"))
    );
    while session.jump_prompt(false) {}
    assert_eq!(session.offset.get(), 0);
    // 没有命令标记：什么都不做，按键该交给终端
    let (plain, _alerts) = session_with_alerts();
    plain.feed_for_test("hello\r\n");
    assert!(!plain.jump_prompt(true));
}

#[test]
fn copies_the_last_commands_output() {
    let session = session_with_commands(3);
    assert_eq!(session.last_command_output().as_deref(), Some("out2"));
    assert!(session.copy_last_output());
    assert_eq!(aureole::clipboard::read_text().as_deref(), Some("out2"));
    let (plain, _alerts) = session_with_alerts();
    assert!(!plain.copy_last_output());
}
