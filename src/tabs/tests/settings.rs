//! 设置页和外观。

use super::*;

/// 起始目录的优先：文件夹自己的目录，其次设置里指定的，都不是目录就用默认（主目录）
#[test]
fn the_start_directory_comes_from_the_folder_then_the_settings() {
    let (folder_dir, settings_dir) = (directory("folder-dir"), directory("settings-dir"));
    let (_harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    assert_eq!(fake.launch(0).directory, None, "默认是主目录");
    workspace
        .form
        .directory
        .set(crate::settings::DirectoryKind::Custom);
    workspace
        .form
        .directory_path
        .set(settings_dir.display().to_string());
    workspace.new_tab();
    assert_eq!(fake.launch(1).directory, Some(settings_dir.clone()));
    let folder = workspace.add_directory(&folder_dir).expect("是目录");
    workspace.new_tab_in(folder);
    assert_eq!(
        fake.launch(2).directory,
        Some(folder_dir.clone()),
        "文件夹的目录优先"
    );
    workspace
        .form
        .directory_path
        .set("/不存在的目录/xyz".to_string());
    workspace.new_tab();
    assert_eq!(
        fake.launch(3).directory,
        Some(folder_dir.clone()),
        "在文件夹里：沿用文件夹的"
    );
    workspace.select(1);
    workspace.new_tab();
    assert_eq!(
        fake.launch(4).directory,
        None,
        "指定的目录不存在：退回主目录"
    );
    assert!(
        workspace
            .add_directory(Path::new("/不存在的目录/xyz"))
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&folder_dir);
    let _ = std::fs::remove_dir_all(&settings_dir);
}

/// 设置里的 shell、配色会带到新终端上
#[test]
fn settings_choices_reach_new_terminals() {
    let (_harness, workspace, fake) = blank(Store::none());
    workspace.new_tab();
    let first = fake.launch(0);
    assert_eq!(first.shell, None);
    assert!(!first.dark_background);
    workspace.form.shell.set(ShellKind::Program);
    workspace.new_tab();
    assert_eq!(fake.launch(1).shell, None, "指定程序但路径是空的：还是默认");
    workspace.form.shell_program.set("  /bin/bash ".to_string());
    workspace.form.appearance.set(ThemeChoice::Dark);
    workspace.new_tab();
    let third = fake.launch(2);
    assert_eq!(third.shell.as_deref(), Some("/bin/bash"));
    assert!(third.dark_background, "深色配色告诉程序底色是深的");
}

/// 点设置：顶上的标签条、新建按钮、左右分、上下分都收起来，换成「设置」；侧栏顶上是「返回」，
/// 在平时放第一行的位置；点返回回到进来之前的会话
#[test]
fn settings_hides_the_tab_strip_and_splits_and_the_back_button_returns() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    for shown in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "toggle-sidebar",
    ] {
        assert!(harness.bounds(shown).is_some(), "平时有 {shown}");
    }
    assert!(harness.bounds("settings-title").is_none());
    let first_row = harness.window_bounds("nav-new").expect("平时侧栏的第一行");
    click(&harness, "nav-settings");
    assert!(workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2, "当前会话没变");
    for gone in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "nav-new",
        "nav-settings",
        "session-1",
        "grid",
    ] {
        assert!(harness.bounds(gone).is_none(), "设置页里 {gone} 不在了");
    }
    assert!(harness.bounds("toggle-sidebar").is_some(), "侧栏开关还在");
    assert_eq!(harness.text("settings-title").as_deref(), Some("设置"));
    assert!(harness.bounds("settings-view").is_some(), "卡片里是设置页");
    let back = harness.window_bounds("settings-back").expect("返回在");
    assert!(
        (back.x - first_row.x).abs() < 1.5 && (back.y - first_row.y).abs() < 1.5,
        "返回在侧栏最上面、平时第一行的位置：{back:?} vs {first_row:?}"
    );
    assert_eq!(harness.text("settings-back").as_deref(), Some("返回"));
    for page in SettingsPage::ALL {
        let bounds = harness.window_bounds(page.name()).expect("分类在");
        assert!(bounds.y > back.y, "「{}」在返回下面", page.label());
    }
    click(&harness, "settings-back");
    assert!(!workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2);
    for shown in [
        "tab-1",
        "tab-2",
        "new-tab",
        "split-columns",
        "split-rows",
        "nav-new",
        "grid",
    ] {
        assert!(harness.bounds(shown).is_some(), "回来以后 {shown} 又在了");
    }
    assert!(harness.bounds("settings-title").is_none());
    assert!(harness.bounds("settings-back").is_none());
}

/// 设置页里 ⌘, 再按一次、⌘W、点标签/快捷键切标签，都是退出设置；设置页不占标签
#[test]
fn settings_can_be_left_from_the_keyboard_and_is_not_a_tab() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    command(&harness, ",");
    assert!(workspace.settings_open.get_untracked());
    command(&harness, ",");
    assert!(!workspace.settings_open.get_untracked(), "再按一次退出");
    command(&harness, ",");
    command(&harness, "w");
    assert!(
        !workspace.settings_open.get_untracked(),
        "⌘W 在设置页里是返回"
    );
    assert_eq!(ids(&workspace), [1, 2], "没有误关会话");
    command(&harness, ",");
    command(&harness, "1");
    assert!(
        !workspace.settings_open.get_untracked(),
        "⌘1 切到标签就离开设置"
    );
    assert_eq!(workspace.active.get_untracked(), 1);
    command(&harness, ",");
    harness.key_down_with(Key::Named(NamedKey::Tab), Modifiers::CONTROL);
    assert!(!workspace.settings_open.get_untracked());
    assert_eq!(workspace.active.get_untracked(), 2);
    assert_eq!(bar_ids(&workspace), [1, 2], "标签条里从来没有设置");
    // 命令面板里也能开
    command(&harness, "k");
    let panel = harness.bounds("palette").expect("面板在");
    let row = harness.bounds("cmd-settings").expect("命令在");
    let origin = (
        (WINDOW_SIZE.0 as f32 - panel.width) / 2.0,
        (WINDOW_SIZE.1 as f32 - panel.height) / 2.0,
    );
    let [x, y] = row.center();
    harness.click(origin.0 + x, origin.1 + y);
    harness.frame();
    assert!(workspace.settings_open.get_untracked());
}

/// 设置的侧栏和平时的侧栏换面板有交接动画：刚换的时候两块都在，动画完了旧的才卸掉；
/// 收起侧栏时设置的分类只剩图标
#[test]
fn the_settings_sidebar_hands_over_and_collapses_to_icons() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    workspace.open_settings();
    harness.advance(ms(40));
    assert!(
        harness.bounds("nav-new").is_some() && harness.bounds("settings-general").is_some(),
        "交接中两块面板都在"
    );
    harness.advance(secs(1.0));
    assert!(harness.bounds("nav-new").is_none() && harness.bounds("settings-general").is_some());
    command(&harness, "b");
    harness.advance(secs(1.0));
    for page in SettingsPage::ALL {
        let bounds = harness.window_bounds(page.name()).expect("分类图标在");
        assert!(bounds.width <= 48.0, "收起时只剩图标，宽 {}", bounds.width);
    }
    let back = harness.window_bounds("settings-back").expect("返回在");
    assert!(back.width <= 48.0, "返回也只剩图标");
    workspace.close_settings();
    harness.advance(secs(1.0));
    assert!(harness.bounds("nav-new").is_some() && harness.bounds("settings-general").is_none());
}

/// 每一类设置页都能点开，点的选择真的写进设置、换页回来还在；设置页的内容占满卡片的宽度
#[test]
fn every_settings_page_opens_and_its_choices_stick() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    harness.advance(secs(1.0));
    for page in SettingsPage::ALL {
        click(&harness, page.name());
        assert_eq!(workspace.settings_page.get_untracked(), page);
        assert!(
            harness.bounds(&format!("{}-page", page.name())).is_some(),
            "「{}」的页面在",
            page.label()
        );
        for other in SettingsPage::ALL.into_iter().filter(|&other| other != page) {
            assert!(
                harness.bounds(&format!("{}-page", other.name())).is_none(),
                "别的页面卸掉"
            );
        }
        let (card, list) = (
            harness.window_bounds("card").expect("卡片在"),
            harness.window_bounds("settings-list").expect("列表在"),
        );
        assert!(
            (list.width - (card.width - 64.0)).abs() < 1.5,
            "「{}」列表宽 {} 应是卡片宽 {} 去掉两边各 32",
            page.label(),
            list.width,
            card.width
        );
    }
    let form = workspace.form;
    click(&harness, "settings-general");
    assert!(
        harness.bounds("set-shell-path").is_none(),
        "没选指定程序时不出现路径框"
    );
    click(&harness, "set-startup-empty");
    click(&harness, "set-shell-custom");
    assert!(
        harness.bounds("set-shell-path").is_some(),
        "指定程序下面出现路径框"
    );
    assert!(
        harness.bounds("set-shell-path-pick").is_some(),
        "还有「选择…」"
    );
    click(&harness, "set-directory-custom");
    assert!(harness.bounds("set-directory-path").is_some());
    click(&harness, "set-title-reported");
    click(&harness, "set-confirm-close");
    click(&harness, "set-last-tab-quit");
    click(&harness, "set-sidebar-closed");
    click(&harness, "settings-appearance");
    click(&harness, "set-appearance-dark");
    click(&harness, "set-cursor-bar");
    click(&harness, "set-cursor-blink");
    // 外观页比卡片高，最下面的要滚下去才点得到
    harness.wheel(600.0, 300.0, 0.0, 20.0);
    harness.advance(secs(1.0));
    // 滚过的页面里元素的位置要按窗口坐标点
    click_menu(&harness, "set-status");
    click(&harness, "settings-terminal");
    click(&harness, "set-bell-flash");
    click(&harness, "set-copy-on-select");
    click(&harness, "set-right-click");
    click(&harness, "settings-general");
    assert_eq!(form.startup.get_untracked(), Startup::Empty);
    assert_eq!(form.shell.get_untracked(), ShellKind::Program);
    assert_eq!(form.title.get_untracked(), TitleSource::Reported);
    assert!(!form.confirm_close.get_untracked(), "默认开着，点一下关掉");
    assert_eq!(form.last_tab.get_untracked(), LastTab::Quit);
    assert_eq!(form.sidebar.get_untracked(), SidebarStart::Closed);
    assert_eq!(form.appearance.get_untracked(), ThemeChoice::Dark);
    assert_eq!(
        form.cursor.get_untracked(),
        crate::settings::CursorStyle::Bar
    );
    assert!(!form.cursor_blink.get_untracked());
    assert!(!form.status_bar.get_untracked());
    assert_eq!(form.bell.get_untracked(), crate::settings::Bell::Flash);
    assert!(form.copy_on_select.get_untracked());
    assert!(form.right_click_paste.get_untracked());
    assert!(
        harness.bounds("set-shell-path").is_some(),
        "换页回来，指定程序还开着"
    );
    assert_eq!(ids(&workspace), [1], "点设置不新开、不关掉会话");
    // 恢复默认
    click(&harness, "settings-about");
    click(&harness, "set-reset");
    assert_eq!(form.snapshot(), Settings::default());
}

/// 换配色：终端区域的底色马上跟着变（浅色白、深色深）；换字号：行列数跟着重算
#[test]
fn palette_and_font_settings_change_the_terminal_right_away() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    let corner = |harness: &Harness| {
        let card = harness.bounds("card").expect("卡片在");
        let shot = harness.screenshot();
        shot.pixel((card.x + 4.0) as u32, (card.y + 4.0) as u32).0
    };
    let bright = corner(&harness);
    assert!(
        bright[..3].iter().all(|&c| c > 240),
        "浅色是白底：{bright:?}"
    );
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let dark = corner(&harness);
    assert!(dark[..3].iter().all(|&c| c < 80), "深色是深底：{dark:?}");
    workspace.form.appearance.set(ThemeChoice::Light);
    harness.advance(secs(1.0));
    assert!(corner(&harness)[..3].iter().all(|&c| c > 240), "切回浅色");
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let (columns, rows) = workspace.size.get_untracked();
    workspace.form.font_size.set(22.0);
    harness.advance(secs(1.0));
    let (bigger_columns, bigger_rows) = workspace.size.get_untracked();
    assert!(
        bigger_columns < columns && bigger_rows < rows,
        "字大了放得下的行列少了：{columns}×{rows} → {bigger_columns}×{bigger_rows}"
    );
    workspace.form.font_size.set(13.0);
    harness.advance(secs(1.0));
    assert_eq!(
        workspace.size.get_untracked(),
        (columns, rows),
        "改回来行列也回来"
    );
    workspace.form.line_height.set(1.6);
    harness.advance(secs(1.0));
    assert!(workspace.size.get_untracked().1 < rows, "行距大了行数少了");
}

/// 外观页顶上的小样是真的：它的底色跟着配色走
#[test]
fn the_appearance_page_has_a_live_preview() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    let preview = harness.window_bounds("settings-preview").expect("小样在");
    let sample = |harness: &Harness| {
        harness
            .screenshot()
            .pixel(
                (preview.x + preview.width - 6.0) as u32,
                (preview.y + 6.0) as u32,
            )
            .0
    };
    assert!(sample(&harness)[..3].iter().all(|&c| c > 240));
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    assert!(sample(&harness)[..3].iter().all(|&c| c < 80));
}

/// 设置里关掉状态条：状态条消失
#[test]
fn the_status_bar_can_be_turned_off() {
    let (harness, workspace) = mounted();
    assert!(harness.bounds("status-target").is_some());
    workspace.form.status_bar.set(false);
    harness.advance(secs(1.0));
    assert!(harness.bounds("status-target").is_none());
    workspace.form.status_bar.set(true);
    harness.advance(secs(1.0));
    assert!(harness.bounds("status-target").is_some());
}

/// 字体：这个平台上能选的每一个，终端里的字都是等宽的，一格的宽度大约是字号的 0.6 倍，
/// 不是退到界面字体后撑开的两倍宽
#[test]
fn the_terminal_font_is_monospace_for_every_choice() {
    use crate::settings::FontChoice;
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    for &font in FontChoice::OFFERED {
        workspace.form.font.set(font);
        harness.advance(secs(1.0));
        let grid = harness.bounds("grid").expect("终端在");
        let columns = workspace.size.get_untracked().0 as f32;
        let cell = grid.width / columns;
        assert!(
            (6.0..9.5).contains(&cell),
            "{font:?}：一格宽 {cell}，13 号等宽字应该在 8 上下"
        );
    }
}

/// 深色：整个窗口（标题栏、侧栏、卡片、终端）一起换；换的时候状态一点不丢（标签、设置页停在原来的页）；
/// 选「跟随系统」就跟着系统的外观走
#[test]
fn the_whole_window_follows_the_appearance_setting() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    let brightness = |harness: &Harness, x: f32, y: f32| {
        let pixel = harness.screenshot().pixel(x as u32, y as u32).0;
        pixel[..3].iter().map(|&c| c as u32).sum::<u32>() / 3
    };
    let card = harness.window_bounds("card").expect("卡片在");
    let probes = |harness: &Harness| {
        [
            brightness(harness, 600.0, 8.0),                   // 标题栏
            brightness(harness, 8.0, 400.0),                   // 侧栏
            brightness(harness, card.x + 4.0, card.y + 300.0), // 卡片
        ]
    };
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "默认（跟随系统，系统是浅色）是浅的"
    );
    click(&harness, "set-appearance-dark");
    assert!(
        probes(&harness).iter().all(|&b| b < 60),
        "深色：标题栏、侧栏、卡片都是深的 {:?}",
        probes(&harness)
    );
    assert!(workspace.settings_open.get_untracked(), "还在设置页");
    assert_eq!(
        workspace.settings_page.get_untracked(),
        SettingsPage::Appearance
    );
    assert!(harness.bounds("settings-appearance-page").is_some());
    assert_eq!(ids(&workspace), [1, 2], "标签一个没丢");
    assert!(
        harness.bounds("set-appearance-dark").is_some(),
        "控件在，点得动"
    );
    // 回到终端：终端也是深的，字是浅色
    click(&harness, "settings-back");
    let terminal = harness.window_bounds("grid").expect("终端在");
    assert!(
        brightness(
            &harness,
            terminal.x + terminal.width - 4.0,
            terminal.y + 300.0
        ) < 60
    );
    // 跟随系统
    workspace.close_settings();
    workspace.open_settings();
    harness.advance(secs(1.0));
    click(&harness, "set-appearance-system");
    harness.set_system_appearance(aureole::Appearance::Light);
    harness.advance(secs(1.0));
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "系统浅色就是浅色"
    );
    harness.set_system_appearance(aureole::Appearance::Dark);
    harness.advance(secs(1.0));
    // 离屏渲染里没有系统的材质，半透明的底盖在浅色的清屏色上，比真机里浅一点
    assert!(
        probes(&harness).iter().all(|&b| b < 130),
        "系统深色就是深色 {:?}",
        probes(&harness)
    );
    click(&harness, "set-appearance-light");
    assert!(
        probes(&harness).iter().all(|&b| b > 230),
        "选浅色就不跟系统"
    );
}

/// 切换深浅色时界面整个重建，但入场动画（侧栏滑进来、标签淡入）不能重放：刚切完的头几帧，
/// 侧栏的行就在原来的位置、完全不透明
#[test]
fn switching_the_appearance_does_not_replay_the_enter_animations() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    harness.advance(secs(1.0));
    let before = (
        harness.window_bounds("nav-new").expect("在"),
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("session-1").expect("在"),
    );
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(ms(30));
    let after = (
        harness.window_bounds("nav-new").expect("在"),
        harness.window_bounds("tab-1").expect("在"),
        harness.window_bounds("session-1").expect("在"),
    );
    assert_eq!(
        before, after,
        "刚切完，各处还在原来的位置，没有从旁边滑进来"
    );
    // 正常的入场动画还在：新开的标签照样淡入
    // 豁免只管换色后那一瞬间：过一会儿新开的东西照样有入场动画
    assert!(crate::theme::just_switched());
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(!crate::theme::just_switched());
    // 设成和现在一样的不算「换」
    crate::theme::set_dark(true);
    assert!(!crate::theme::just_switched(), "没变就不算");
}

/// 外观页有「粗细」（常规、粗两档），点了存得住
#[test]
fn the_appearance_page_has_a_font_weight_switch() {
    use crate::settings::FontWeight;
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    harness.advance(secs(1.0));
    assert!(
        harness.window_bounds("set-font-weight-regular").is_some(),
        "常规在"
    );
    assert!(
        harness.window_bounds("set-font-weight-light").is_none(),
        "只有两档"
    );
    click_menu(&harness, "set-font-weight-bold");
    assert_eq!(workspace.form.snapshot().font_weight, FontWeight::Bold);
    click_menu(&harness, "set-font-weight-regular");
    assert_eq!(workspace.form.snapshot().font_weight, FontWeight::Regular);
}

/// 字体粗细真的改变终端里字的粗细：粗的比细的墨多，格子大小不变
#[test]
fn the_font_weight_setting_changes_how_heavy_the_terminal_text_is() {
    use crate::settings::FontWeight;
    let (harness, workspace) = mounted();
    let session = workspace.tabs.get_untracked()[0].session();
    session.feed_for_test(&"MMMMMMMMMMMMMMMMMMMMMMMM\r\n".repeat(10));
    harness.advance(secs(1.0));
    let ink = |harness: &Harness| {
        let grid = harness.window_bounds("grid").expect("在");
        let shot = harness.screenshot();
        let mut sum = 0u64;
        for y in grid.y as u32..(grid.y + 130.0) as u32 {
            for x in grid.x as u32..(grid.x + 260.0) as u32 {
                sum += 255 - u64::from(shot.pixel(x, y).0[0]);
            }
        }
        sum
    };
    let columns = workspace.size.get_untracked();
    workspace.form.font_weight.set(FontWeight::Regular);
    harness.advance(secs(1.0));
    let light = ink(&harness);
    workspace.form.font_weight.set(FontWeight::Bold);
    harness.advance(secs(1.0));
    let bold = ink(&harness);
    assert!(
        bold as f64 > light as f64 * 1.1,
        "粗的字墨更多：{light} → {bold}"
    );
    assert_eq!(workspace.size.get_untracked(), columns, "粗细不改格子大小");
}

/// 关于页只留有用的：版本、框架的链接、GitHub 图标（源码）、配置文件夹、恢复默认；
/// 链接点了交给系统打开
#[test]
fn the_about_page_links_to_the_framework_and_the_source() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::About);
    harness.advance(secs(1.0));
    let tree = format!("{:?}", harness.element_tree());
    for useless in ["终端仿真", "上的终端模拟器", "自研 pty"] {
        assert!(!tree.contains(useless), "关于里不该有「{useless}」");
    }
    assert!(tree.contains(crate::product::NAME));
    assert_eq!(harness.text("about-framework").as_deref(), Some("Aureole"));
    assert_eq!(harness.text("about-github").as_deref(), Some("GitHub"));
    assert!(harness.bounds("set-reset").is_some());
    click(&harness, "about-framework");
    click(&harness, "about-github");
    assert_eq!(
        harness.opened_urls(),
        [
            crate::product::FRAMEWORK_URL.to_string(),
            crate::product::REPOSITORY.to_string()
        ]
    );
}

/// 深色里的控件看得清：关着的开关是深色的槽（不是一条白的）、数字框的加减按钮悬停不发白；
/// 浅色里关着的开关还是浅灰的槽
#[test]
fn switches_and_steppers_are_readable_in_dark() {
    let (harness, workspace) = mounted();
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Terminal);
    harness.advance(secs(1.0));
    let brightness = |harness: &Harness, x: f32, y: f32| {
        let pixel = harness.screenshot().pixel(x as u32, y as u32).0;
        pixel[..3].iter().map(|&c| c as u32).sum::<u32>() / 3
    };
    let track = |harness: &Harness| {
        let bounds = harness.window_bounds("set-copy-on-select").expect("开关在");
        brightness(
            harness,
            bounds.x + bounds.width - 5.0,
            bounds.y + bounds.height / 2.0,
        )
    };
    let light_off = track(&harness);
    assert!(light_off > 190, "浅色里关着的槽是浅灰：{light_off}");
    workspace.form.appearance.set(ThemeChoice::Dark);
    harness.advance(secs(1.0));
    let dark_off = track(&harness);
    assert!(dark_off < 90, "深色里关着的槽是深的：{dark_off}");
    workspace.form.copy_on_select.set(true);
    harness.advance(secs(1.0));
    let dark_on = track(&harness);
    assert!(
        dark_on > dark_off + 30,
        "开着的比关着的亮得多：{dark_off} → {dark_on}"
    );
    // 数字框的加号：鼠标放上去是一层淡灰，不是白的
    let stepper = harness.window_bounds("set-scrollback").expect("数字框在");
    let (x, y) = (
        stepper.x + stepper.width - 12.0,
        stepper.y + stepper.height / 2.0,
    );
    harness.mouse_move(x, y);
    harness.advance(secs(1.0));
    let hovered = brightness(&harness, x + 6.0, y + 8.0);
    assert!(hovered < 110, "悬停在加号上不发白：{hovered}");
}

/// ⌘+ / ⌘- 加减字号，⌘0 回默认；到头了不再变
#[test]
fn font_size_shortcuts_zoom_and_reset() {
    let (harness, workspace) = mounted();
    let default = crate::settings::Settings::default().font_size;
    assert_eq!(workspace.form.font_size.get_untracked(), default);
    command(&harness, "=");
    assert_eq!(workspace.form.font_size.get_untracked(), default + 1.0);
    command(&harness, "-");
    command(&harness, "-");
    assert_eq!(workspace.form.font_size.get_untracked(), default - 1.0);
    command(&harness, "0");
    assert_eq!(workspace.form.font_size.get_untracked(), default);
    for _ in 0..40 {
        command(&harness, "=");
    }
    assert_eq!(
        workspace.form.font_size.get_untracked(),
        crate::settings::FONT_SIZE.1
    );
    for _ in 0..60 {
        command(&harness, "-");
    }
    assert_eq!(
        workspace.form.font_size.get_untracked(),
        crate::settings::FONT_SIZE.0
    );
}
