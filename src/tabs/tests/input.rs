//! 粘贴确认、命令历史、链接的右键菜单。

use super::*;

/// 右键点在网址上：菜单最前面有「打开链接」「复制链接」；点在别处没有
#[test]
fn right_clicking_a_link_offers_open_and_copy() {
    let (harness, workspace) = mounted();
    let session = workspace.focused_session().expect("有终端");
    session.feed_for_test("see https://example.com/x now\r\n");
    harness.frame();
    let grid = harness.window_bounds("grid").expect("网格在");
    let [columns, rows] = session.grid_size_for_test();
    let cell = (grid.width / columns as f32, grid.height / rows as f32);
    let at = |column: usize| {
        (
            grid.x + (column as f32 + 0.5) * cell.0,
            grid.y + 0.5 * cell.1,
        )
    };
    let pane = workspace.tabs.get_untracked()[0]
        .layout
        .get_untracked()
        .first_id();
    // 点在别处：没有链接的项
    let (x, y) = at(1);
    harness.right_click(x, y);
    harness.advance(secs(1.0));
    assert!(
        harness
            .bounds(&format!("menu-pane-open-link-{pane}"))
            .is_none()
    );
    assert!(
        harness.bounds(&format!("menu-pane-paste-{pane}")).is_some(),
        "普通菜单在"
    );
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    // 点在网址上
    let (x, y) = at(8);
    harness.right_click(x, y);
    harness.advance(secs(1.0));
    assert!(
        harness
            .bounds(&format!("menu-pane-copy-link-{pane}"))
            .is_some()
    );
    let [mx, my] = harness
        .window_bounds(&format!("menu-pane-open-link-{pane}"))
        .expect("菜单项在")
        .center();
    harness.click(mx, my);
    harness.advance(secs(1.0));
    assert_eq!(harness.opened_urls(), ["https://example.com/x"]);
}

/// 程序没开括号粘贴时粘贴多行文字：先弹确认框，点「粘贴」才写进去，点「取消」就不写；单行、开了括号粘贴的不问
#[test]
fn multiline_paste_asks_first_unless_bracketed() {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    workspace.new_tab();
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    let session = workspace.focused_session().expect("有终端");
    aureole::clipboard::write_text("echo one\necho two\n".to_string());
    session.paste();
    harness.advance(secs(1.0));
    assert!(harness.bounds("paste-dialog").is_some(), "多行要先问");
    assert_eq!(fake.pty(0).take(), "", "还没写进去");
    let [x, y] = harness
        .window_bounds("paste-cancel")
        .expect("按钮在")
        .center();
    harness.click(x, y);
    harness.advance(secs(1.0));
    assert!(harness.bounds("paste-dialog").is_none());
    assert_eq!(fake.pty(0).take(), "", "取消就不写");
    session.paste();
    harness.advance(secs(1.0));
    let [x, y] = harness
        .window_bounds("paste-confirm")
        .expect("按钮在")
        .center();
    harness.click(x, y);
    harness.advance(secs(1.0));
    assert_eq!(fake.pty(0).take(), "echo one\recho two\r", "确认后照常粘贴");
    // 单行（只有结尾的换行）不问
    aureole::clipboard::write_text("echo one\n".to_string());
    session.paste();
    harness.advance(secs(1.0));
    assert!(harness.bounds("paste-dialog").is_none());
    assert_eq!(fake.pty(0).take(), "echo one\r");
    // 程序开了括号粘贴：多行也不问
    session.feed_for_test("\x1b[?2004h");
    aureole::clipboard::write_text("a\nb".to_string());
    session.paste();
    harness.advance(secs(1.0));
    assert!(harness.bounds("paste-dialog").is_none());
    assert_eq!(fake.pty(0).take(), "\x1b[200~a\nb\x1b[201~");
    // 设置里关了：不问
    session.feed_for_test("\x1b[?2004l");
    workspace.form.confirm_paste.set(false);
    aureole::clipboard::write_text("x\ny".to_string());
    session.paste();
    harness.advance(secs(1.0));
    assert!(harness.bounds("paste-dialog").is_none());
    assert_eq!(fake.pty(0).take(), "x\ry");
}

/// 命令历史：跑完的命令（有命令标记）记下来、存盘；命令面板里打字能搜到，选中写在提示符处但不按回车
#[test]
fn finished_commands_become_searchable_history() {
    let directory = directory("history");
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::at(&directory));
    workspace.new_tab();
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    let session = workspace.focused_session().expect("有终端");
    for command in ["git status", "cargo test --release", "git status"] {
        session.feed_for_test(&format!(
            "\x1b]133;A\x07$ \x1b]133;B\x07{command}\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07"
        ));
    }
    assert_eq!(
        workspace.history_matches("git", 10),
        ["git status"],
        "重复的只留一条"
    );
    assert_eq!(
        workspace.history_matches("CARGO", 10),
        ["cargo test --release"],
        "不分大小写"
    );
    // 存了盘：重开还在
    let reopened = Workspace::new(Fake::default().spawn(), None, Store::at(&directory));
    assert_eq!(
        reopened.history_matches("", 10),
        ["git status", "cargo test --release"]
    );
    // 命令面板里搜
    command(&harness, "k");
    harness.type_text("cargo");
    harness.advance(secs(1.0));
    let [x, y] = harness
        .window_bounds("cmd-history-0")
        .expect("历史里的命令出现在面板里")
        .center();
    harness.click(x, y);
    harness.advance(secs(1.0));
    assert_eq!(
        fake.pty(0).take(),
        "cargo test --release",
        "写在提示符处，没有回车"
    );
    std::fs::remove_dir_all(&directory).ok();
}

/// 从访达拖文件到本机终端：路径（含空格的加引号）写在光标处，空格隔开，没有回车；服务器的终端不收
#[test]
fn dropping_files_from_the_desktop_types_their_paths() {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    workspace.new_tab();
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    let [x, y] = harness.window_bounds("grid").expect("网格在").center();
    harness.drop_files(
        x,
        y,
        vec![
            "/tmp/plain.txt".into(),
            "/tmp/with space.txt".into(),
            "/tmp/it's.txt".into(),
            "/tmp/line\nbreak.txt".into(),
        ],
    );
    harness.advance(secs(1.0));
    assert_eq!(
        fake.pty(0).take(),
        "/tmp/plain.txt '/tmp/with space.txt' '/tmp/it'\\''s.txt' ",
        "带换行的整个跳过，别的引好、空格隔开、没有回车"
    );
    // 服务器的终端：本机的路径没有意义，不写
    assert!(workspace.new_tab_ssh("devbox"));
    harness.frame();
    let [x, y] = harness.window_bounds("grid").expect("网格在").center();
    harness.drop_files(x, y, vec!["/tmp/plain.txt".into()]);
    harness.advance(secs(1.0));
    assert_eq!(fake.pty(1).take(), "");
}
