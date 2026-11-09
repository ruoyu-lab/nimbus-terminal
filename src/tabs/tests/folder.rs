//! 侧栏的文件夹和起始目录。

use super::*;

/// 侧栏没有「本机」。目标下面的空白处右键能新建文件夹，文件夹上右键「创建会话」开一个终端并归在它下面
#[test]
fn a_folder_made_from_the_blank_area_holds_a_new_session() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    assert!(harness.text("folder-1").is_none(), "一开始没有文件夹");
    right_click_named(&harness, "targets-blank");
    assert!(
        harness.bounds("menu-targets").is_some(),
        "空白处右键是新建文件夹"
    );
    click_menu(&harness, "menu-new-folder");
    assert_eq!(
        harness.focused_name().as_deref(),
        Some("folder-field"),
        "焦点在名字框里"
    );
    assert!(
        harness
            .text_state("folder-field")
            .unwrap_or_default()
            .contains("文件夹"),
        "默认名字"
    );
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(harness.bounds("folder-dialog").is_none());
    assert_eq!(harness.text("folder-1").as_deref(), Some("文件夹"));
    assert_eq!(ids(&workspace), [1], "新建文件夹不开终端");
    right_click_named(&harness, "folder-1");
    click_menu(&harness, "menu-new-session-folder-1");
    assert_eq!(ids(&workspace), [1, 2]);
    assert_eq!(workspace.tabs.get_untracked()[1].folder, Some(1));
    assert_eq!(workspace.active.get_untracked(), 2);
    let (folder, session) = (
        harness.window_bounds("folder-1").expect("文件夹在"),
        harness
            .window_bounds("session-2")
            .expect("新会话在文件夹下面"),
    );
    assert!(folder.y < session.y, "会话排在自己的文件夹下面");
    click(&harness, "folder-1");
    assert!(
        harness.bounds("session-2").is_none(),
        "再点一下文件夹，里面的会话收起来"
    );
}

/// 真 shell：从文件夹里新建的终端，屏幕上有提示符，而且 shell 真的站在文件夹对应的目录里
#[test]
fn a_real_shell_starts_in_the_folders_directory_and_shows_a_prompt() {
    let project = directory("real-shell");
    let workspace = Workspace::new(Rc::new(crate::app::spawn), None, Store::none());
    let folder = workspace.add_directory(&project).expect("是目录");
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.frame();
    assert!(harness.bounds("no-tabs").is_some(), "启动是空的");
    workspace.new_tab_in(folder);
    harness.frame();
    assert!(harness.bounds("no-tabs").is_none() && harness.bounds("grid").is_some());
    let session = workspace.tabs.get_untracked()[0].session().clone();
    let mut saw = String::new();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        harness.advance(ms(50));
        saw = session.screen_text_for_test();
        if saw.chars().any(|c| !c.is_whitespace() && c != '\0') {
            break;
        }
    }
    assert!(
        saw.chars().any(|c| !c.is_whitespace() && c != '\0'),
        "屏幕是空的：{saw:?}"
    );
    let expected = project.canonicalize().expect("目录在");
    let mut actual = None;
    for _ in 0..40 {
        actual = session
            .working_directory()
            .and_then(|path| path.canonicalize().ok());
        if actual.as_ref() == Some(&expected) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(actual, Some(expected), "shell 起在文件夹的目录里");
    let _ = std::fs::remove_dir_all(&project);
}

/// 第一次启动：一个标签也没有，面板上写「没有标签」，下面是「选择目录」；平时的终端、监控开关已经没有了
#[test]
fn the_first_launch_is_empty_with_a_choose_directory_button_below_the_label() {
    let (harness, workspace, fake) = blank(Store::none());
    assert!(ids(&workspace).is_empty());
    assert!(fake.launches.borrow().is_empty(), "启动时一个终端也没起");
    assert_eq!(harness.text("no-tabs").as_deref(), Some("没有标签"));
    let (label, button) = (
        harness.bounds("no-tabs").expect("标题在"),
        harness
            .bounds("choose-directory")
            .expect("选择目录的按钮在"),
    );
    assert!(
        button.y > label.y + label.height - 0.5,
        "按钮在「没有标签」下面"
    );
    assert_eq!(
        harness.text("choose-directory").as_deref(),
        Some("选择目录")
    );
    assert!(harness.bounds("grid").is_none());
    assert!(harness.bounds("folder-1").is_none(), "目标里是空的");
    for gone in ["mode-terminal", "mode-monitor", "mode-terminal-icon"] {
        assert!(harness.bounds(gone).is_none(), "{gone} 已经去掉");
    }
}

/// 点「选择目录」弹出系统的选目录对话框；选好后目录进「目标」，并在里面开一个终端，从那个目录起
#[test]
fn choosing_a_directory_adds_a_folder_and_opens_a_terminal_in_it() {
    let project = directory("choose");
    let (harness, workspace, fake) = blank(Store::none());
    click(&harness, "choose-directory");
    let requests = harness.file_dialogs();
    assert_eq!(requests.len(), 1, "弹出了一个对话框");
    assert_eq!(requests[0].kind, aureole::FileDialogKind::OpenDirectory);
    assert!(ids(&workspace).is_empty(), "还没选，不开终端");
    assert!(harness.respond_file_dialog(vec![project.clone()]));
    harness.advance(secs(1.0));
    let folders = workspace.folders.get_untracked();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path.as_deref(), Some(project.as_path()));
    assert_eq!(
        Some(folders[0].name.as_str()),
        project.file_name().and_then(|name| name.to_str()),
        "文件夹用目录的名字"
    );
    assert_eq!(ids(&workspace), [1]);
    assert_eq!(
        workspace.tabs.get_untracked()[0].folder,
        Some(folders[0].id)
    );
    assert_eq!(fake.launch(0).directory, Some(project.clone()));
    assert!(harness.bounds("grid").is_some(), "终端挂上了");
    assert!(harness.bounds("no-tabs").is_none());
    // 取消对话框什么也不做
    workspace.close_tab(1);
    harness.advance(secs(1.0));
    click(&harness, "choose-directory");
    assert!(harness.respond_file_dialog(Vec::new()));
    assert!(ids(&workspace).is_empty());
    assert_eq!(workspace.folders.get_untracked().len(), 1);
    let _ = std::fs::remove_dir_all(&project);
}

/// 把目录从访达拖到「目标」：加成一个文件夹；文件和已经有的目录不重复加；在里面创建终端就从那个目录起
#[test]
fn dropping_a_directory_on_the_targets_makes_a_folder_that_starts_terminals_there() {
    let project = directory("drop");
    let file = project.join("a.txt");
    std::fs::write(&file, "x").expect("写得进");
    let (harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    harness.advance(secs(1.0));
    let [x, y] = harness
        .window_bounds("targets-zone")
        .expect("目标区域在")
        .center();
    harness.hover_files(Some([x, y]));
    harness.advance(ms(50));
    harness.drop_files(x, y, vec![file.clone(), project.clone()]);
    harness.advance(secs(1.0));
    let folders = workspace.folders.get_untracked();
    assert_eq!(folders.len(), 1, "文件不算，目录加了一个");
    assert_eq!(folders[0].path.as_deref(), Some(project.as_path()));
    assert!(harness.bounds("folder-1").is_some(), "侧栏里出现了");
    assert_eq!(ids(&workspace), [1], "只加文件夹，不自己开终端");
    // 同一个目录再拖一次：还是一个
    harness.drop_files(x, y, vec![project.clone()]);
    harness.advance(secs(1.0));
    assert_eq!(workspace.folders.get_untracked().len(), 1);
    // 在文件夹里创建终端：从拖进来的目录起；再 ⌘T 新建（当前在这个文件夹里）也一样
    right_click_named(&harness, "folder-1");
    click_menu(&harness, "menu-new-session-folder-1");
    assert_eq!(fake.launch(1).directory, Some(project.clone()));
    workspace.new_tab();
    assert_eq!(fake.launch(2).directory, Some(project.clone()));
    // 不在文件夹里的终端不受影响
    assert_eq!(fake.launch(0).directory, None);
    let _ = std::fs::remove_dir_all(&project);
}

/// 终端都有图标：直接列在「目标」下的，图标和文件夹的图标同一列、一样大，名字也和文件夹的名字对齐；文件夹里的缩进一级
#[test]
fn loose_terminals_have_an_icon_aligned_with_the_folder_icon() {
    let (harness, workspace, _fake) = blank(Store::none());
    let folder = workspace.create_folder("工作");
    workspace.new_tab();
    workspace.new_tab_in(folder);
    harness.advance(secs(1.0));
    let (folder_icon, session_icon) = (
        harness
            .window_bounds("folder-icon-1")
            .expect("文件夹图标在"),
        harness.window_bounds("session-icon-1").expect("终端图标在"),
    );
    assert!((folder_icon.x - session_icon.x).abs() < 0.5, "同一列");
    assert!(
        (folder_icon.width - session_icon.width).abs() < 0.5,
        "一样大"
    );
    let (row_name, folder_row) = (
        harness.window_bounds("session-1").expect("行在"),
        harness.window_bounds("folder-1").expect("文件夹行在"),
    );
    assert!(
        (row_name.x - folder_row.x).abs() < 0.5 && (row_name.width - folder_row.width).abs() < 0.5
    );
    // 文件夹里的终端也有图标，缩进到文件夹名字的那一列
    let (child_icon, folder_row) = (
        harness
            .window_bounds("session-icon-2")
            .expect("文件夹里的终端也有图标"),
        harness.window_bounds("folder-icon-1").expect("在"),
    );
    assert!(
        (child_icon.x - (folder_row.x + folder_row.width + 8.0)).abs() < 0.5,
        "图标和文件夹的名字同一列：{child_icon:?} {folder_row:?}"
    );
    assert!((child_icon.width - folder_row.width).abs() < 0.5, "一样大");
    // 收起侧栏：目标下的终端还在，只剩图标
    command(&harness, "b");
    harness.advance(secs(1.0));
    let icon = harness
        .window_bounds("session-icon-1")
        .expect("收起后图标还在");
    let sidebar = harness.window_bounds("sidebar").expect("侧栏在");
    assert!(icon.x + icon.width < sidebar.x + sidebar.width);
    assert!(
        harness.text("session-1").is_none() || harness.text("session-1").as_deref() == Some("")
    );
}

/// `~` 展开成主目录；`/` 和 `\` 都认，`~名字` 不动
#[test]
fn tilde_expands_to_the_home_directory_with_either_separator() {
    let Some(home) = crate::store::user_home() else {
        return;
    };
    assert_eq!(crate::tabs::folders::expand_home("~"), Some(home.clone()));
    assert_eq!(
        crate::tabs::folders::expand_home("~/work"),
        Some(home.join("work"))
    );
    assert_eq!(
        crate::tabs::folders::expand_home("~\\work"),
        Some(home.join("work"))
    );
    assert_eq!(
        crate::tabs::folders::expand_home("~work"),
        Some(std::path::PathBuf::from("~work"))
    );
    assert_eq!(crate::tabs::folders::expand_home(""), None);
    assert_eq!(
        crate::tabs::folders::expand_home("/tmp"),
        Some(std::path::PathBuf::from("/tmp"))
    );
}

/// 按住文件夹上下拖过相邻的文件夹就换位；拖的时候所有文件夹收成一行，松手后各自恢复展开
#[test]
fn dragging_a_folder_reorders_it() {
    let (harness, workspace) = mounted();
    for name in ["甲", "乙", "丙"] {
        workspace.create_folder(name);
    }
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(folder_names(&workspace), ["甲", "乙", "丙"]);
    let first = workspace.folders.get_untracked()[0].id;
    let [x, y] = harness
        .bounds(&format!("folder-{first}"))
        .expect("文件夹在")
        .center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x, y + 20.0);
    harness.mouse_move(x, y + 64.0);
    assert!(
        workspace.folder_drag.get_untracked().is_some(),
        "拖动开始了"
    );
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert_eq!(folder_names(&workspace), ["乙", "丙", "甲"], "向下拖了两格");
    assert!(workspace.folder_drag.get_untracked().is_none());
    assert_eq!(harness.window_drag_count(), 0);
    // 点一下不拖：只是展开收起，顺序不变
    harness.advance(secs(1.0));
    let [x, y] = harness
        .bounds(&format!("folder-{first}"))
        .expect("文件夹在")
        .center();
    harness.click(x, y);
    assert_eq!(folder_names(&workspace), ["乙", "丙", "甲"]);
}
