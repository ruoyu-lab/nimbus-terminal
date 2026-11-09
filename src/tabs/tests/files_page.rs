//! 右侧面板的文件页：浏览、上传、下载、改路径、拖进终端。

use super::*;

/// 「选择…」弹系统对话框（程序是选文件、目录是选文件夹），选好填进路径框
#[test]
fn the_pick_buttons_fill_the_path_fields() {
    let (harness, workspace) = mounted();
    workspace.form.shell.set(ShellKind::Program);
    workspace
        .form
        .directory
        .set(crate::settings::DirectoryKind::Custom);
    workspace.open_settings();
    harness.advance(secs(1.0));
    click(&harness, "set-shell-path-pick");
    let request = harness.file_dialogs().remove(0);
    assert_eq!(request.kind, aureole::FileDialogKind::OpenFile);
    assert!(harness.respond_file_dialog(vec![PathBuf::from("/bin/bash")]));
    assert_eq!(workspace.form.shell_program.get_untracked(), "/bin/bash");
    click(&harness, "set-directory-path-pick");
    let request = harness.file_dialogs().remove(0);
    assert_eq!(request.kind, aureole::FileDialogKind::OpenDirectory);
    assert!(harness.respond_file_dialog(vec![PathBuf::from("/tmp")]));
    assert_eq!(workspace.form.directory_path.get_untracked(), "/tmp");
}

/// 容器的文件页：列目录、下载都冲着容器去；右键「在此打开终端」开的是 docker exec，进的是容器里的目录
#[test]
fn the_files_page_works_for_containers_too() {
    let (harness, workspace, remote, launches) = mounted_docker_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    assert_eq!(remote.calls()[0], "list ~");
    assert_eq!(harness.text("files-path").as_deref(), Some("/home/deploy"));
    right_click_named(&harness, "file-readme.txt");
    click_menu(&harness, "menu-file-download-readme.txt");
    settle(&harness, || {
        remote
            .calls()
            .iter()
            .any(|call| call.starts_with("download"))
    });
    assert_eq!(
        remote.calls()[1],
        "download /home/deploy/readme.txt dir=false"
    );
    assert!(
        remote
            .targets
            .lock()
            .expect("锁")
            .iter()
            .all(|target| *target
                == Target::Docker {
                    container: "web-1".into()
                }),
        "文件页的每次调用都冲着容器去"
    );
    right_click_named(&harness, "file-src");
    click_menu(&harness, "menu-file-terminal-src");
    settle(&harness, || launches.borrow().len() >= 2);
    let launch = launches.borrow()[1].clone();
    assert_eq!(
        launch.target,
        Target::Docker {
            container: "web-1".into()
        }
    );
    assert_eq!(
        launch.remote_directory.as_deref(),
        Some("/home/deploy/src"),
        "进的是点的那个文件夹"
    );
    assert_eq!(workspace.active.get_untracked(), 2, "新终端切到前台");
}

/// 文件页读出主目录：文件夹在前、隐藏文件默认不显示、不认识的类型显示但没有下载按钮
#[test]
fn the_files_page_lists_the_home_directory() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    assert_eq!(remote.calls()[0], "list ~", "第一次读主目录");
    assert_eq!(harness.text("files-path").as_deref(), Some("/home/deploy"));
    assert!(harness.bounds("file-src").is_some() && harness.bounds("file-readme.txt").is_some());
    assert!(harness.bounds("file-.env").is_none(), "隐藏文件默认不显示");
    assert!(
        harness.bounds("download-readme.txt").is_some() && harness.bounds("download-src").is_some()
    );
    assert!(
        harness.bounds("download-pipe").is_none(),
        "管道没有下载按钮"
    );
    harness.click_named("files-hidden");
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("file-.env").is_some(),
        "打开「显示隐藏」后出来了"
    );
}

/// 点文件夹进去，点上一级回来，点刷新重新读
#[test]
fn folders_open_and_the_up_button_goes_back() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    harness.click_named("file-src");
    settle(&harness, || {
        files_of(&workspace).path.as_deref() == Some("/home/deploy/src")
    });
    harness.advance(secs(1.0));
    assert_eq!(
        harness.text("files-path").as_deref(),
        Some("/home/deploy/src")
    );
    assert!(harness.bounds("file-main.rs").is_some());
    harness.click_named("files-up");
    settle(&harness, || {
        files_of(&workspace).path.as_deref() == Some("/home/deploy")
    });
    assert_eq!(files_of(&workspace).path.as_deref(), Some("/home/deploy"));
    harness.click_named("files-refresh");
    settle(&harness, || remote.calls().len() >= 4);
    assert_eq!(
        remote.calls(),
        [
            "list ~",
            "list /home/deploy/src",
            "list /home/deploy",
            "list /home/deploy"
        ]
    );
}

/// 下载：文件、文件夹（带 -r 的标志），存到下载目录；点下载不会顺带打开文件夹；失败写原因
#[test]
fn downloads_go_to_the_download_folder_and_report_the_result() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    harness.click_named("download-readme.txt");
    settle(&harness, || {
        files_of(&workspace)
            .transfers
            .iter()
            .all(|transfer| transfer.state != TransferState::Running)
            && !files_of(&workspace).transfers.is_empty()
    });
    assert_eq!(
        remote.calls()[1],
        "download /home/deploy/readme.txt dir=false"
    );
    // 存成的路径按本机的写法显示（Windows 上最后一段前面是反斜杠）
    let saved = PathBuf::from("/tmp/nimbus-downloads").join("readme.txt");
    assert_eq!(
        files_of(&workspace).transfers[0].state,
        TransferState::Done(format!("已存到 {}", saved.display()))
    );
    harness.click_named("download-src");
    settle(&harness, || {
        files_of(&workspace).transfers.len() == 2
            && files_of(&workspace).transfers[1].state != TransferState::Running
    });
    assert_eq!(remote.calls()[2], "download /home/deploy/src dir=true");
    assert_eq!(
        files_of(&workspace).path.as_deref(),
        Some("/home/deploy"),
        "点下载没有顺带打开文件夹"
    );
    harness.advance(secs(1.0));
    assert!(harness.bounds("files-transfers").is_some());
    assert!(
        harness
            .text("transfer-state-1")
            .unwrap_or_default()
            .contains("已存到")
    );
    // 失败的下载写原因
    workspace.files.update(|files| {
        files
            .get_mut(&1)
            .expect("有")
            .entries
            .push(FakeRemote::entry("boom", Kind::File, 1));
    });
    workspace.files_download("boom");
    settle(&harness, || {
        files_of(&workspace)
            .transfers
            .iter()
            .any(|transfer| matches!(transfer.state, TransferState::Failed(_)))
    });
    assert!(
        files_of(&workspace)
            .transfers
            .iter()
            .any(|transfer| transfer.state == TransferState::Failed("没权限".into()))
    );
    // 不能下载的和带控制字符的名字：直接失败，不去碰远端
    let before = remote.calls().len();
    workspace.files_download("pipe");
    assert_eq!(remote.calls().len(), before);
    assert!(
        files_of(&workspace)
            .transfers
            .iter()
            .any(|transfer| transfer.name == "pipe"
                && transfer.state == TransferState::Failed("这种文件不能下载".into()))
    );
}

/// 上传：传到现在这个目录，传完重新读目录；远端已有同名的不覆盖；带换行的名字拒绝
#[test]
fn uploads_land_in_the_current_directory_and_never_overwrite() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    workspace.files_upload(vec![
        PathBuf::from("/Users/me/new.txt"),
        PathBuf::from("/Users/me/readme.txt"),
        PathBuf::from("/Users/me/line\nbreak.txt"),
    ]);
    settle(&harness, || {
        remote
            .calls()
            .iter()
            .filter(|call| call.starts_with("list"))
            .count()
            >= 2
    });
    let calls = remote.calls();
    assert!(calls.contains(&"upload /Users/me/new.txt -> /home/deploy".to_string()));
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.starts_with("upload"))
            .count(),
        1,
        "同名的和带换行的没有传：{calls:?}"
    );
    let transfers = files_of(&workspace).transfers;
    assert_eq!(transfers.len(), 3);
    assert!(transfers.iter().any(|transfer| transfer.name == "new.txt"
        && transfer.state == TransferState::Done("已上传".into())));
    assert!(
        transfers
            .iter()
            .any(|transfer| transfer.name == "readme.txt"
                && transfer.state == TransferState::Failed("远端已有同名文件，没有覆盖".into()))
    );
    assert!(
        transfers.iter().any(|transfer| transfer.state
            == TransferState::Failed("这个文件名不能传（带控制字符）".into()))
    );
    harness.advance(secs(1.0));
    harness.click_named("files-clear");
    harness.advance(secs(1.0));
    assert!(files_of(&workspace).transfers.is_empty(), "清除记录");
    assert!(harness.bounds("files-transfers").is_none());
}

/// 文件从访达拖到面板上就是上传
#[test]
fn dropping_files_on_the_panel_uploads_them() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    let [x, y] = harness.bounds("files").expect("文件页在").center();
    harness.drop_files(x, y, vec![PathBuf::from("/Users/me/dropped.txt")]);
    settle(&harness, || {
        remote.calls().iter().any(|call| call.starts_with("upload"))
    });
    assert!(
        remote
            .calls()
            .contains(&"upload /Users/me/dropped.txt -> /home/deploy".to_string())
    );
}

/// 没连上时写明白，连上之后自己出来（每 2 秒再试）；不支持、出错各有说法；条目读到之后断线不清空列表
#[test]
fn the_files_page_explains_failures_and_recovers_by_itself() {
    let (harness, workspace, remote) = mounted_files();
    // 等挂上时的第一次读完成：这个测试的前提是「一开始已经读到过目录」
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    remote.fail_with(Some(Failure::NotConnected));
    workspace.files_refresh();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::NotConnected
    });
    harness.advance(secs(1.0));
    // 一开始已经读到过目录：列表留着，上面一行小字说连接断了
    assert!(
        harness
            .text("files-banner")
            .unwrap_or_default()
            .contains("连接断了")
    );
    assert!(harness.bounds("file-readme.txt").is_some());
    // 连上了：不用点任何东西，定时重试把列表带出来
    remote.fail_with(None);
    settle(&harness, || {
        harness.advance(secs(1.0));
        files_of(&workspace).status == crate::files::Status::Ready
    });
    assert_eq!(files_of(&workspace).status, crate::files::Status::Ready);
    // 读到之后再失败：列表还在，不被一句错误顶掉
    remote.fail_with(Some(Failure::Failed("超时".into())));
    workspace.files_refresh();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Failed("超时".into())
    });
    harness.advance(secs(1.0));
    assert!(harness.bounds("file-readme.txt").is_some(), "列表还在");
    assert_eq!(
        harness.text("files-banner").as_deref(),
        Some("读取失败：超时")
    );
    // 不支持
    let (harness, workspace, remote) = mounted_files();
    remote.fail_with(Some(Failure::Unsupported));
    workspace.files_refresh();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Unsupported
    });
    harness.advance(secs(1.0));
    assert!(
        harness
            .text("files-notice")
            .unwrap_or_default()
            .contains("暂不支持")
    );
}

/// 关了标签，它的文件状态跟着清掉；设置里关了「远程功能」不去读
#[test]
fn closing_a_tab_drops_its_files_and_the_setting_stops_reading() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    workspace.new_tab();
    workspace.close_tab(1);
    assert!(workspace.files.get_untracked().is_empty());
    let (_harness, workspace, remote_off) = {
        let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
        let remote = Arc::new(FakeRemote::default());
        *workspace.remote.borrow_mut() = remote.clone();
        workspace.form.remote.set(false);
        workspace.polling.set(true);
        workspace.new_tab_ssh("prod-hk");
        workspace.files_load(None);
        (harness, workspace, remote)
    };
    assert!(remote_off.calls().is_empty() && workspace.files.get_untracked().is_empty());
    let _ = remote;
}

/// 点路径变成输入框（带着现在的路径、焦点在里面）；回车去那个路径：绝对路径、相对现在目录的、`~`
#[test]
fn clicking_the_path_turns_it_into_an_input_that_navigates() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    assert!(harness.bounds("files-path-field").is_none(), "平时是文字");
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("files-path-field"),
        "焦点在输入框里"
    );
    assert!(
        harness
            .text_state("files-path-field")
            .unwrap_or_default()
            .contains("\"/home/deploy\""),
        "带着现在的路径"
    );
    assert!(harness.bounds("files-path").is_none(), "文字换成了输入框");
    // 绝对路径
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("/home");
    harness.key_down(NamedKey::Enter);
    settle(&harness, || {
        files_of(&workspace).path.as_deref() == Some("/home")
    });
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("files-path-field").is_none(),
        "回车之后又是文字"
    );
    assert_eq!(harness.text("files-path").as_deref(), Some("/home"));
    // 相对现在的目录
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("deploy");
    harness.key_down(NamedKey::Enter);
    settle(&harness, || {
        files_of(&workspace).path.as_deref() == Some("/home/deploy")
    });
    // ~ 是主目录
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("~");
    harness.key_down(NamedKey::Enter);
    settle(&harness, || remote.calls().len() >= 4);
    assert_eq!(
        remote.calls(),
        ["list ~", "list /home", "list /home/deploy", "list ~"]
    );
}

/// Esc、点别处、什么都不填：取消，不去读任何目录
#[test]
fn escaping_or_leaving_the_path_input_changes_nothing() {
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    harness.type_text("junk");
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    assert!(harness.bounds("files-path-field").is_none(), "Esc 取消");
    assert_eq!(harness.text("files-path").as_deref(), Some("/home/deploy"));
    // 清空了回车：也是取消
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.key_down(NamedKey::Backspace);
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    // 点别处：失去焦点就取消
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    assert!(harness.bounds("files-path-field").is_some());
    harness.click_named("file-readme.txt");
    harness.advance(secs(1.0));
    assert!(harness.bounds("files-path-field").is_none(), "点别处取消");
    assert_eq!(
        remote.calls(),
        ["list ~"],
        "取消都没有去读目录：{:?}",
        remote.calls()
    );
}

/// 还没读到目录（没连上）时没有路径可改：点了没反应
#[test]
fn the_path_cannot_be_edited_before_a_directory_is_known() {
    let (harness, workspace, remote) = mounted_files();
    // 等挂上时的第一次读完成：在途的成功结果会在状态被删后编号重新对得上，把路径写回来
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    remote.fail_with(Some(Failure::NotConnected));
    workspace.files.update(|files| {
        files.remove(&1);
    });
    workspace.files_load(None);
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::NotConnected
    });
    harness.advance(secs(1.0));
    if harness.bounds("files-path-button").is_some() {
        harness.click_named("files-path-button");
        harness.advance(secs(1.0));
    }
    assert!(harness.bounds("files-path-field").is_none());
}

/// 文件夹上右键「在这里新建终端」：新开一个服务器标签、进那个远端目录；
/// 文件上右键是在所在目录；空白处右键是在现在这个目录
#[test]
fn right_clicking_files_opens_a_terminal_in_the_remote_directory() {
    let (harness, workspace, fake_remote, launches) = mounted_files_with_launches();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    let _ = fake_remote;
    right_click_named(&harness, "file-src");
    click_menu(&harness, "menu-file-terminal-src");
    settle(&harness, || launches.borrow().len() >= 2);
    let launch = launches.borrow()[1].clone();
    assert_eq!(
        launch.target,
        Target::Ssh {
            host: "prod-hk".into()
        }
    );
    assert_eq!(
        launch.remote_directory.as_deref(),
        Some("/home/deploy/src"),
        "进的是点的那个文件夹"
    );
    assert_eq!(workspace.active.get_untracked(), 2, "新终端切到前台");
    workspace.select(1);
    harness.advance(secs(1.0));
    // 文件：所在目录
    right_click_named(&harness, "file-readme.txt");
    click_menu(&harness, "menu-file-terminal-readme.txt");
    settle(&harness, || launches.borrow().len() >= 3);
    assert_eq!(
        launches.borrow()[2].remote_directory.as_deref(),
        Some("/home/deploy")
    );
    workspace.select(1);
    harness.advance(secs(1.0));
    // 空白处
    right_click_named(&harness, "files-list");
    click_menu(&harness, "menu-files-blank-terminal");
    settle(&harness, || launches.borrow().len() >= 4);
    assert_eq!(
        launches.borrow()[3].remote_directory.as_deref(),
        Some("/home/deploy")
    );
    // 不能下载的（管道）右键菜单里没有「下载」
    workspace.select(1);
    harness.advance(secs(1.0));
    right_click_named(&harness, "file-pipe");
    harness.advance(secs(1.0));
    assert!(harness.window_bounds("menu-file-download-pipe").is_none());
    assert!(harness.window_bounds("menu-file-copy-pipe").is_some());
}

/// 右键「下载」「复制路径」
#[test]
fn the_context_menu_can_download_and_copy_the_path() {
    let (harness, workspace, remote, _launches) = mounted_files_with_launches();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    right_click_named(&harness, "file-readme.txt");
    click_menu(&harness, "menu-file-download-readme.txt");
    settle(&harness, || {
        remote
            .calls()
            .iter()
            .any(|call| call.starts_with("download"))
    });
    assert_eq!(
        remote.calls()[1],
        "download /home/deploy/readme.txt dir=false"
    );
    right_click_named(&harness, "file-readme.txt");
    click_menu(&harness, "menu-file-copy-readme.txt");
    assert_eq!(
        aureole::clipboard::read_text().as_deref(),
        Some("/home/deploy/readme.txt")
    );
}

/// 没有目录可用（相对路径、不能用的）也能开终端，只是进主目录：不会因为目录不对就开不了
#[test]
fn a_bad_remote_directory_still_opens_a_plain_terminal() {
    let (_harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    assert!(workspace.new_tab_ssh_in("prod-hk", Some("relative".into())));
    assert_eq!(ids(&workspace), [1]);
}

/// 把文件页的条目拖到终端上：写进它的绝对路径（要引的引好），后面一个空格，不按回车
#[test]
fn dragging_an_entry_into_the_terminal_types_its_quoted_path() {
    let (harness, workspace, remote, _launches) = mounted_files_with_launches();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    // 远端有一个名字带空格、引号的文件
    workspace.files.update(|files| {
        let state = files.get_mut(&1).expect("有");
        state
            .entries
            .push(FakeRemote::entry("my file's.txt", Kind::File, 3));
    });
    harness.advance(secs(1.0));
    let pty = fake_pty(&workspace);
    let terminal = harness.window_bounds("grid").expect("终端在").center();
    for (name, expected) in [
        ("readme.txt", "/home/deploy/readme.txt "),
        ("my file's.txt", "'/home/deploy/my file'\\''s.txt' "),
        ("src", "/home/deploy/src "),
    ] {
        let from = harness
            .window_bounds(&format!("file-{name}"))
            .expect("行在")
            .center();
        harness.drag(from, terminal);
        harness.advance(secs(1.0));
        assert_eq!(pty.take(), expected, "{name}");
    }
    assert!(
        remote
            .calls()
            .iter()
            .all(|call| !call.starts_with("download")),
        "拖进终端只是写路径，不下载"
    );
}

/// 带换行的远端文件名（能让你执行任何命令）拖进终端：一个字也不写
#[test]
fn a_remote_name_with_a_newline_is_never_typed_into_the_terminal() {
    let (harness, workspace, _remote, _launches) = mounted_files_with_launches();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    workspace.files.update(|files| {
        files
            .get_mut(&1)
            .expect("有")
            .entries
            .push(FakeRemote::entry("a\nrm -rf ~", Kind::File, 1));
    });
    harness.advance(secs(1.0));
    let pty = fake_pty(&workspace);
    let session = workspace.tabs.get_untracked()[0].session();
    assert!(!session.insert_path("/home/deploy/a\nrm -rf ~"));
    assert!(!session.insert_path("/x\r"));
    assert!(session.insert_path("/ok/path"));
    assert_eq!(pty.take(), "/ok/path ");
}

/// 把条目拖进终端之后，键盘焦点要在终端上：直接就能打字，不用再点一下终端
#[test]
fn after_dropping_a_path_the_terminal_has_keyboard_focus() {
    let (harness, workspace, _remote, _launches) = mounted_files_with_launches();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    let pty = fake_pty(&workspace);
    let terminal = harness.window_bounds("grid").expect("终端在").center();
    let from = harness
        .window_bounds("file-readme.txt")
        .expect("行在")
        .center();
    harness.drag(from, terminal);
    harness.advance(secs(1.0));
    assert_eq!(pty.take(), "/home/deploy/readme.txt ");
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("grid"),
        "焦点在终端上"
    );
    harness.type_text("ls");
    assert_eq!(pty.take(), "ls", "打的字进了终端");
    // 拆分之后拖到另一个终端：焦点跟着去那一个，字写进那一个
    workspace.split(Axis::Columns);
    harness.advance(secs(1.0));
    let tab = workspace.tabs.get_untracked()[0].clone();
    let (first, second) = {
        let leaves = tab.layout.get_untracked().leaves();
        (leaves[0].0, leaves[1].0)
    };
    assert_ne!(tab.focus.get_untracked(), first, "拆分后聚焦的是新的那个");
    let other = harness
        .window_bounds(&format!("pane-{first}"))
        .expect("第一个终端在")
        .center();
    let from = harness.window_bounds("file-src").expect("行在").center();
    harness.drag(from, other);
    harness.advance(secs(1.0));
    assert_eq!(
        tab.focus.get_untracked(),
        first,
        "聚焦到被放进路径的那个终端"
    );
    assert_eq!(harness.focused_name().as_deref(), Some("grid"));
    let _ = second;
}

/// 去过的目录马上显示（不等网络），后台重新读，读回来有变化再换：「上一级」不再要等一个往返
#[test]
fn revisiting_a_directory_shows_the_cached_listing_while_it_refreshes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (harness, workspace, remote) = mounted_files();
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    harness.click_named("file-src");
    settle(&harness, || {
        files_of(&workspace).path.as_deref() == Some("/home/deploy/src")
            && files_of(&workspace).status == crate::files::Status::Ready
    });
    // 网络变慢（卡住），服务器上的主目录也变了
    let gate = Arc::new(AtomicBool::new(false));
    *remote.gate.lock().expect("锁") = Some(gate.clone());
    *remote.home_override.lock().expect("锁") =
        Some(vec![FakeRemote::entry("fresh.txt", Kind::File, 1)]);
    harness.click_named("files-up");
    harness.frame();
    // 读还卡着，但页面已经是缓存的主目录了
    let state = files_of(&workspace);
    assert_eq!(
        state.path.as_deref(),
        Some("/home/deploy"),
        "马上回到上一级"
    );
    assert_eq!(state.status, crate::files::Status::Loading, "后台在刷新");
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("file-readme.txt").is_some(),
        "显示的是缓存的那份"
    );
    assert!(harness.bounds("file-fresh.txt").is_none());
    assert!(
        harness.bounds("files-notice").is_none(),
        "不写「正在读取」把列表顶掉"
    );
    // 放行：读回来，换成新的
    gate.store(true, Ordering::Release);
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("file-fresh.txt").is_some(),
        "读回来之后换成新的"
    );
    assert!(harness.bounds("file-readme.txt").is_none());
    // 用 ~ 回主目录也走缓存（读过一次 ~ 就知道主目录是哪个）
    assert_eq!(files_of(&workspace).home.as_deref(), Some("/home/deploy"));
}
