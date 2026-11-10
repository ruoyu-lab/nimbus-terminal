//! 按指定的 shell 新建会话（Windows 上的命令提示符、Windows PowerShell、PowerShell 7）：拆分、重启都跟着标签走，
//! 标题叫 shell 的名字，加号旁边的下拉列出每一种。

use super::*;

/// 用来指定的程序：Windows 上是命令提示符，别的平台随便一个 shell（逻辑和平台无关，只看传没传下去）
fn chosen() -> String {
    if cfg!(windows) {
        r"C:\Windows\System32\cmd.exe".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

/// 指定了 shell 的标签：自己起的、拆出来的都用它；别的标签照旧看设置（`None`）。
/// 重启以后还是它，没指定的还是没指定
#[test]
fn a_chosen_shell_follows_the_tab_through_splits_and_restarts() {
    let root = directory("shells");
    let store = Store::at(root.join("config"));
    let program = chosen();
    {
        let (_harness, workspace, fake) = blank(store.clone());
        workspace.new_tab_with_shell(&program);
        workspace.split(Axis::Columns);
        workspace.new_tab();
        assert_eq!(fake.launch(0).shell.as_deref(), Some(program.as_str()));
        assert_eq!(
            fake.launch(1).shell.as_deref(),
            Some(program.as_str()),
            "拆出来的终端用标签的 shell"
        );
        assert_eq!(fake.launch(2).shell, None, "普通的新建看设置");
        workspace.save_now();
    }
    let (_harness, workspace, fake) = blank(store);
    assert_eq!(workspace.restore(), 2);
    let shells: Vec<_> = (0..3).map(|index| fake.launch(index).shell).collect();
    assert_eq!(
        shells,
        [Some(program.clone()), Some(program), None],
        "恢复出来的两个终端还是那个 shell，另一个标签还是默认"
    );
}

/// Windows 上本机的标签默认叫它起的 shell；别的平台还是「终端」
#[test]
fn local_tabs_are_named_after_their_shell_on_windows() {
    let (_harness, workspace, _fake) = blank(Store::none());
    workspace.new_tab_with_shell(&chosen());
    workspace.new_tab();
    let titles = workspace.titles.get_untracked();
    if cfg!(windows) {
        assert_eq!(titles[&1].as_str(), "命令提示符");
        assert_eq!(
            titles[&2].as_str(),
            local_title(),
            "没指定的叫默认 shell 的名字"
        );
    } else {
        assert_eq!(titles[&1].as_str(), DEFAULT_TITLE);
        assert_eq!(titles[&2].as_str(), DEFAULT_TITLE);
    }
}

/// 加号旁边的下拉（Windows）：每种 shell 一项，装了的点了按它开新标签，没装的点了什么也不发生。
/// 装了 WSL 的发行版时，后面一个「WSL」小标题，下面每个发行版一项
#[cfg(windows)]
#[test]
fn the_shell_menu_lists_every_shell_and_only_installed_ones_open() {
    let (harness, workspace) = mounted();
    let shells = crate::pty::shells();
    assert_eq!(
        shells[..3]
            .iter()
            .map(|shell| shell.id.as_str())
            .collect::<Vec<_>>(),
        ["pwsh", "powershell", "cmd"]
    );
    click(&harness, "new-tab-shells");
    assert_eq!(
        harness.window_bounds("menu-shells-wsl").is_some(),
        shells.len() > 3,
        "有发行版才有 WSL 那一节"
    );
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(0.5));
    for shell in shells {
        click(&harness, "new-tab-shells");
        let menu = harness.window_bounds("menu-shells").expect("下拉弹出来了");
        let chevron = harness.window_bounds("new-tab-shells").expect("箭头在");
        assert!(menu.y >= chevron.y + chevron.height, "挂在箭头下面");
        let before = ids(&workspace).len();
        click_menu(&harness, &format!("menu-shell-{}", shell.id));
        if shell.installed {
            assert_eq!(ids(&workspace).len(), before + 1, "{} 开了新标签", shell.id);
            let tab = workspace
                .tabs
                .get_untracked()
                .last()
                .cloned()
                .expect("有标签");
            assert_eq!(tab.shell.as_deref(), Some(shell.program.as_str()));
            assert!(harness.bounds("menu-shells").is_none(), "点完收起");
        } else {
            assert_eq!(ids(&workspace).len(), before, "没装的 {} 点不了", shell.id);
            harness.key_down(NamedKey::Escape);
            harness.advance(secs(0.5));
        }
    }
}

/// WSL 的标签（Windows）：叫发行版的名字；拆出来的终端还是那个发行版，从 WSL 里报的目录（换成 `\\wsl.localhost\…`）起
#[cfg(windows)]
#[test]
fn wsl_tabs_are_named_after_their_distribution_and_split_where_the_shell_is() {
    let (_harness, workspace, fake) = blank(Store::none());
    let distribution = crate::wsl::distributions()
        .into_iter()
        .next()
        .map_or_else(|| "Ubuntu".to_string(), |distribution| distribution.name);
    let program = crate::wsl::program(&distribution);
    workspace.new_tab_with_shell(&program);
    assert_eq!(
        workspace.titles.get_untracked()[&1].as_str(),
        distribution.as_str()
    );
    assert_eq!(fake.launch(0).shell.as_deref(), Some(program.as_str()));
    let host = std::env::var("COMPUTERNAME").expect("有 COMPUTERNAME");
    workspace
        .tabs
        .get_untracked()
        .last()
        .expect("有标签")
        .session()
        .feed_for_test(&format!("\x1b]7;file://{host}/tmp\x07"));
    workspace.split(Axis::Columns);
    let split = fake.launch(1);
    assert_eq!(split.shell.as_deref(), Some(program.as_str()));
    // 起始目录要真的在才用：装了这个发行版时才进得去
    if crate::wsl::distributions()
        .iter()
        .any(|installed| installed.name == distribution)
    {
        assert_eq!(
            split.directory,
            Some(format!(r"\\wsl.localhost\{distribution}\tmp").into())
        );
    }
}

/// 侧栏的「WSL」（Windows）：列出装了的发行版；点一个就开它的会话、不归文件夹，会话列在它下面、不在本机的会话里；
/// 再点就切过去不重复开；右键能再开一个；从新建的下拉开的也归到这里。一个发行版都没有时整组不在
#[cfg(windows)]
#[test]
fn the_wsl_group_lists_distributions_and_opens_sessions() {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    *workspace.distribution_lister.borrow_mut() =
        Rc::new(|| vec!["Ubuntu".to_string(), "Debian".to_string()]);
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("wsl").is_some());
    let (ubuntu, debian) = (
        harness.bounds("wsl-Ubuntu").expect("Ubuntu 在"),
        harness.bounds("wsl-Debian").expect("Debian 在"),
    );
    assert!(ubuntu.y < debian.y, "按列出来的顺序");
    harness.click_named("wsl-Ubuntu");
    harness.advance(secs(1.0));
    let ubuntu_program = crate::wsl::program("Ubuntu");
    assert_eq!(
        fake.launch(0).shell.as_deref(),
        Some(ubuntu_program.as_str())
    );
    let session = harness.bounds("session-1").expect("会话列出来了");
    let debian = harness.bounds("wsl-Debian").expect("Debian 在");
    assert!(
        session.y > ubuntu.y && session.y < debian.y,
        "列在 Ubuntu 下面"
    );
    // 已经有会话就切过去，不重复开
    workspace.new_tab();
    harness.frame();
    harness.click_named("wsl-Ubuntu");
    harness.advance(secs(1.0));
    assert_eq!(ids(&workspace), [1, 2]);
    assert_eq!(workspace.active.get_untracked(), 1);
    // 右键可以再开一个
    right_click_named(&harness, "wsl-Ubuntu");
    click_menu(&harness, "menu-new-session-wsl-Ubuntu");
    settle(&harness, || ids(&workspace).len() == 3);
    // 从新建的下拉开的也一样：不归文件夹，列在发行版下面
    workspace.new_tab_with_shell(&crate::wsl::program("Debian"));
    harness.advance(secs(1.0));
    let tabs = workspace.tabs.get_untracked();
    let distributions: Vec<_> = tabs.iter().map(Tab::wsl_distribution).collect();
    assert_eq!(
        distributions,
        [Some("Ubuntu"), None, Some("Ubuntu"), Some("Debian")]
    );
    assert!(tabs.iter().all(|tab| tab.folder.is_none()));
    let last = harness.bounds("session-4").expect("会话列出来了");
    assert!(last.y > harness.bounds("wsl-Debian").expect("Debian 在").y);
    // 发行版都卸了：整组不在（重新列一遍时）
    *workspace.distribution_lister.borrow_mut() = Rc::new(Vec::new);
    workspace.refresh_distributions();
    harness.advance(secs(1.0));
    assert!(harness.bounds("wsl").is_none());
}

/// 拖文件进来时按这个终端起的 shell 的规矩引（Windows）：同一个带 `$` 的名字，PowerShell 里用单引号，cmd 里用双引号
#[cfg(windows)]
#[test]
fn dropped_paths_are_quoted_for_the_shell_the_tab_started() {
    let powershell = crate::pty::shells()
        .into_iter()
        .find(|shell| shell.id == "powershell")
        .expect("有 Windows PowerShell")
        .program;
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    workspace.new_tab_with_shell(&powershell);
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.frame();
    let drop = |harness: &Harness| {
        let [x, y] = harness.window_bounds("grid").expect("网格在").center();
        harness.drop_files(x, y, vec![r"C:\data\a$b.txt".into()]);
        harness.advance(secs(1.0));
    };
    drop(&harness);
    assert_eq!(fake.pty(0).take(), r"'C:\data\a$b.txt' ");
    workspace.new_tab_with_shell(&chosen());
    harness.advance(secs(1.0));
    drop(&harness);
    assert_eq!(fake.pty(1).take(), r#""C:\data\a$b.txt" "#);
}
