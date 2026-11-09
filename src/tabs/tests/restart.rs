//! 重启后：标签、文件夹、排布、终端里的内容都回来。

use super::*;

/// 文件夹、开着的标签（顺序、名字、关掉时的目录）、侧栏宽度都存下来，下次启动恢复；
/// 设置里「启动时」选空白就不恢复标签（文件夹还在）
#[test]
fn folders_and_tabs_survive_a_restart() {
    let root = directory("persist");
    let store = Store::at(root.join("config"));
    let (project, nested) = (root.join("project"), root.join("project").join("sub"));
    std::fs::create_dir_all(&nested).expect("建得了目录");
    {
        let (_harness, workspace, fake) = blank(store.clone());
        let folder = workspace.add_directory(&project).expect("是目录");
        workspace.create_folder("临时");
        workspace.new_tab_in(folder);
        workspace.new_tab();
        workspace.new_tab();
        workspace.rename_tab(2, "构建");
        // 第一个标签里 shell cd 进了子目录；标签条里把第三个拖到最前面
        fake.pty(0).set_directory(&nested);
        workspace.move_tab(3, 0);
        workspace.select(2);
        workspace.sidebar_width.set(280.0);
        workspace.save_now();
    }
    assert!(root.join("config").join("state.json").exists());
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(
            workspace
                .folders
                .get_untracked()
                .iter()
                .map(|folder| (folder.name.clone(), folder.path.clone()))
                .collect::<Vec<_>>(),
            [
                ("project".to_string(), Some(project.clone())),
                ("临时".to_string(), None)
            ],
            "文件夹和它们的目录回来了"
        );
        assert_eq!(workspace.sidebar_width.get_untracked(), 280.0);
        assert!(ids(&workspace).is_empty(), "标签要 restore 才开");
        assert_eq!(workspace.restore(), 3);
        // 标签条的顺序存下来了：原来的第三个在最前面
        let launches: Vec<_> = (0..3).map(|index| fake.launch(index).directory).collect();
        assert_eq!(
            launches,
            [
                Some(project.clone()),
                Some(nested.clone()),
                Some(project.clone())
            ],
            "各自回到关掉时的目录（没问到目录的用文件夹的）"
        );
        // 开的先后和标签条一致：原来标签条里第三个（id 3）现在是 1，「构建」（原来的 2）现在是 3
        let titles = workspace.titles.get_untracked();
        assert_eq!(titles[&3].as_str(), "构建", "起过的名字也在");
        assert_eq!(titles[&1].as_str(), local_title());
        assert_eq!(workspace.active.get_untracked(), 3, "选中的还是原来那个");
        assert_eq!(
            workspace
                .tabs
                .get_untracked()
                .iter()
                .map(|tab| tab.folder)
                .collect::<Vec<_>>(),
            [Some(1), Some(1), Some(1)]
        );
        // 再关一次、改设置：下次启动不恢复标签
        workspace.save_now();
        workspace.form.startup.set(Startup::Empty);
    }
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(
            workspace.form.startup.get_untracked(),
            Startup::Empty,
            "设置也存了"
        );
        assert_eq!(workspace.restore(), 0);
        assert!(
            fake.launches.borrow().is_empty(),
            "空白启动：一个终端也不起"
        );
        assert_eq!(workspace.folders.get_untracked().len(), 2, "文件夹还在");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 启动时侧栏收起的设置：下次启动侧栏是收起的
#[test]
fn the_sidebar_start_setting_is_applied_at_launch() {
    let root = directory("sidebar-start");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert!(!workspace.collapsed.get_untracked());
        workspace.form.sidebar.set(SidebarStart::Closed);
    }
    let (_harness, workspace, _fake) = blank(store);
    assert!(workspace.collapsed.get_untracked());
    let _ = std::fs::remove_dir_all(&root);
}

/// 终端里的内容跟着标签存下来，下次恢复标签时先放回去（带颜色），新 shell 的提示符接在后面；
/// 设置里关掉就不存、不恢复，已经存的文件也删掉
#[test]
fn terminal_output_is_saved_and_comes_back_with_the_tab() {
    let root = directory("output");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        workspace.new_tab();
        workspace.new_tab();
        workspace.tabs.get_untracked()[0]
            .session()
            .feed_for_test("\x1b[32mhello\x1b[0m 世界\r\nsecond line");
        workspace.save_now();
        assert!(root.join("config").join("output-0.ansi").exists());
        assert!(
            !root.join("config").join("output-1.ansi").exists(),
            "没有内容的标签不存文件"
        );
        // 再来新输出才重写；关掉一个标签后多出来的文件要删
        workspace.tabs.get_untracked()[1]
            .session()
            .feed_for_test("another");
        workspace.save_now();
        assert!(root.join("config").join("output-1.ansi").exists());
        workspace.close_tab(2);
        workspace.save_now();
        assert!(!root.join("config").join("output-1.ansi").exists());
    }
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 1);
        let session = workspace.tabs.get_untracked()[0].session().clone();
        let screen = session.screen_text_for_test();
        let screen = screen.replace('\0', "");
        let mut lines = screen.lines().map(str::trim_end);
        assert_eq!(lines.next(), Some("hello 世界"));
        assert_eq!(lines.next(), Some("second line"));
        assert!(
            session.transcript(100).contains("\x1b[0;38;5;2mhello"),
            "颜色也回来了：{:?}",
            session.transcript(100)
        );
        // 关掉「恢复时带上终端里的内容」：文件删掉，下次不恢复内容
        workspace.form.save_output.set(false);
        workspace.save_now();
        assert!(!root.join("config").join("output-0.ansi").exists());
    }
    {
        let (_harness, workspace, _fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 1);
        let screen = workspace.tabs.get_untracked()[0]
            .session()
            .screen_text_for_test();
        assert!(screen.trim().is_empty(), "没存内容：{screen:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 拆分的排布、每个终端的目录和里面的内容、聚焦的是哪个、分隔条的位置，都存下来，下次启动照样恢复
#[test]
fn split_layouts_survive_a_restart() {
    let root = directory("split-persist");
    let store = Store::at(root.join("config"));
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(&a).expect("建得了目录");
    std::fs::create_dir_all(&b).expect("建得了目录");
    {
        let (_harness, workspace, fake) = blank(store.clone());
        workspace.new_tab();
        fake.pty(0).set_directory(&a);
        workspace.split(Axis::Columns);
        fake.pty(1).set_directory(&b);
        workspace.split(Axis::Rows);
        let tab = workspace.tabs.get_untracked()[0].clone();
        let leaves = tab.layout.get_untracked().leaves();
        leaves[0].1.feed_for_test("first pane output");
        leaves[1].1.feed_for_test("second pane output");
        leaves[2].1.feed_for_test("third pane output");
        workspace.focus_pane(1, leaves[1].0);
        if let Pane::Split { ratio, .. } = tab.layout.get_untracked() {
            ratio.set(0.3);
        }
        workspace.new_tab();
        workspace.tabs.get_untracked()[1]
            .session()
            .feed_for_test("other tab");
        workspace.finish();
    }
    {
        let (_harness, workspace, fake) = blank(store.clone());
        assert_eq!(workspace.restore(), 2);
        let tabs = workspace.tabs.get_untracked();
        let leaves = tabs[0].layout.get_untracked().leaves();
        assert_eq!(leaves.len(), 3, "三个终端回来了");
        assert_eq!(fake.launch(0).directory, Some(a.clone()));
        assert_eq!(fake.launch(1).directory, Some(b.clone()));
        let texts: Vec<String> = leaves
            .iter()
            .map(|(_, session)| session.screen_text_for_test().replace('\0', ""))
            .collect();
        assert!(texts[0].contains("first pane output"));
        assert!(texts[1].contains("second pane output"));
        assert!(texts[2].contains("third pane output"));
        assert!(
            tabs[1]
                .session()
                .screen_text_for_test()
                .contains("other tab")
        );
        assert_eq!(
            tabs[0].focus.get_untracked(),
            leaves[1].0,
            "聚焦的还是第二个"
        );
        match tabs[0].layout.get_untracked() {
            Pane::Split {
                axis,
                ratio,
                second,
                ..
            } => {
                assert_eq!(axis, Axis::Columns);
                assert!((ratio.get_untracked() - 0.3).abs() < 0.001, "分隔条的位置");
                assert!(
                    matches!(
                        *second,
                        Pane::Split {
                            axis: Axis::Rows,
                            ..
                        }
                    ),
                    "右边又上下分了"
                );
            }
            Pane::Leaf { .. } => panic!("应该是拆分的"),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 退出时（窗口已经关了、界面已经卸载）还要能把终端里最后的内容存下来；存过一次之后不再存
/// （窗口关了以后 shell 收到 SIGHUP 退出，屏幕上多出来的「进程已退出」不该被存进去）
#[test]
fn the_workspace_saves_its_last_output_after_the_window_is_gone() {
    let root = directory("exit-save");
    let store = Store::at(root.join("config"));
    let (harness, workspace, _fake) = blank(store.clone());
    workspace.new_tab();
    let session = workspace.tabs.get_untracked()[0].session();
    session.feed_for_test("npm run dev\r\nready in 300 ms");
    drop(harness);
    workspace.finish();
    let saved = std::fs::read_to_string(root.join("config").join("output-0.ansi")).expect("存了");
    assert!(
        saved.contains("ready in 300 ms"),
        "最后的输出也在：{saved:?}"
    );
    session.feed_for_test("\r\n[进程已退出，退出码 129]");
    workspace.save_now();
    let again = std::fs::read_to_string(root.join("config").join("output-0.ansi")).expect("在");
    assert!(!again.contains("进程已退出"), "退出之后不再存");
    let _ = std::fs::remove_dir_all(&root);
}

/// 顺序是存下来的：重启后文件夹还是拖过之后的顺序
#[test]
fn the_folder_order_survives_a_restart() {
    let directory = directory("folder-order");
    let store = Store::at(directory.clone());
    {
        let workspace = Workspace::new(Fake::default().spawn(), None, store.clone());
        for name in ["甲", "乙", "丙"] {
            workspace.create_folder(name);
        }
        let last = workspace.folders.get_untracked()[2].id;
        workspace.move_folder(last, 0);
        workspace.save_now();
    }
    let workspace = Workspace::new(Fake::default().spawn(), None, store);
    assert_eq!(folder_names(&workspace), ["丙", "甲", "乙"]);
    std::fs::remove_dir_all(directory).ok();
}

/// 面板宽度重启后还在
#[test]
fn the_panel_width_survives_a_restart() {
    let directory = directory("panel-width");
    let store = Store::at(directory.clone());
    {
        let workspace = Workspace::new(Fake::default().spawn(), None, store.clone());
        workspace.panel_width.set(400.0);
        workspace.save_now();
    }
    let workspace = Workspace::new(Fake::default().spawn(), None, store);
    assert_eq!(workspace.panel_width.get_untracked(), 400.0);
    std::fs::remove_dir_all(directory).ok();
}
