//! 出图用的测试（`cargo test -- --ignored` 才跑），不是检查。

use super::*;

/// 评审界面用：把几种状态存成 PNG。平时不跑；
/// `UI_SHOTS=目录 cargo test screenshots_for_review -- --ignored` 出图（离屏渲染，没有系统材质）
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_review() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let save = |harness: &Harness, name: &str| {
        harness.advance(secs(1.0));
        harness
            .screenshot()
            .save_png(directory.join(name))
            .expect("存得下 PNG");
    };
    // 第一次启动
    let (harness, workspace, _fake) = blank(Store::none());
    harness.set_scale_factor(2.0);
    save(&harness, "0-first-launch.png");
    let project = directory.join("项目");
    std::fs::create_dir_all(&project).expect("建得了目录");
    workspace.add_directory(&project);
    workspace.create_folder("临时");
    workspace.new_tab_in(1);
    workspace.tabs.get_untracked()[0].session().feed_for_test("\x1b[32mlicha\x1b[0m \x1b[34m~/终端\x1b[0m % cargo test\r\nrunning 77 tests\r\ntest result: \x1b[32mok\x1b[0m. 77 passed\r\n");
    workspace.new_tab();
    save(&harness, "1-expanded.png");
    workspace.split(Axis::Columns);
    workspace.split(Axis::Rows);
    save(&harness, "1b-split.png");
    workspace.close_pane(2, workspace.tabs.get_untracked()[1].focus.get_untracked());
    workspace.close_pane(2, workspace.tabs.get_untracked()[1].focus.get_untracked());
    right_click_named(&harness, "targets-blank");
    save(&harness, "2-targets-menu.png");
    harness.key_down(NamedKey::Escape);
    harness.advance(secs(1.0));
    command(&harness, "b");
    save(&harness, "3-collapsed.png");
    command(&harness, "b");
    harness.click_named("nav-settings");
    save(&harness, "4-settings-general.png");
    harness.click_named("set-shell-custom");
    harness.click_named("set-directory-custom");
    save(&harness, "4b-settings-fields.png");
    harness.click_named("settings-appearance");
    save(&harness, "5-settings-appearance.png");
    harness.click_named("set-appearance-dark");
    save(&harness, "5b-settings-dark.png");
    harness.click_named("settings-terminal");
    save(&harness, "6-settings-terminal.png");
    harness.click_named("settings-shortcuts");
    save(&harness, "7-settings-shortcuts.png");
    harness.click_named("settings-about");
    save(&harness, "8-settings-about.png");
    command(&harness, "b");
    save(&harness, "9-settings-collapsed.png");
    command(&harness, "b");
    harness.click_named("settings-back");
    save(&harness, "10-back-with-dark-terminal.png");
    let _ = std::fs::remove_dir_all(&project);
}

/// README 里的截图：`README_SHOTS=docs/screenshots cargo test screenshots_for_readme -- --ignored`
/// （离屏渲染，没有系统的窗口材质，和真机上半透明的侧栏略有不同）
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_readme() {
    let Ok(directory) = std::env::var("README_SHOTS") else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("建得了目录");
    let save = |harness: &Harness, name: &str| {
        harness.advance(secs(1.0));
        harness
            .screenshot()
            .save_png(directory.join(name))
            .expect("存得下 PNG");
    };
    const BUILD: &str = "\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m \x1b[1;34mgit:(\x1b[31mmain\x1b[34m)\x1b[0m cargo test\r\n   \x1b[1;32mCompiling\x1b[0m nimbus-terminal v0.1.0\r\n    \x1b[1;32mFinished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 4.21s\r\n     \x1b[1;32mRunning\x1b[0m unittests src/main.rs\r\n\r\nrunning 170 tests\r\ntest vt::tests::mouse_modes_switch_on_and_off_independently ... \x1b[32mok\x1b[0m\r\ntest tabs::tests::split_layouts_survive_a_restart ... \x1b[32mok\x1b[0m\r\ntest tabs::tests::folders_and_tabs_survive_a_restart ... \x1b[32mok\x1b[0m\r\n\r\ntest result: \x1b[32mok\x1b[0m. 170 passed; 0 failed\r\n\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m \x1b[1;34mgit:(\x1b[31mmain\x1b[34m)\x1b[0m ";
    const LISTING: &str = "\x1b[1;32m➜\x1b[0m \x1b[36m~/项目\x1b[0m ls -la\r\ndrwxr-xr-x   9 licha  staff   288 Nov  6 22:30 \x1b[1;34m.\x1b[0m\r\ndrwxr-xr-x  14 licha  staff   448 Nov  6 22:30 \x1b[1;34mnimbus-terminal\x1b[0m\r\ndrwxr-xr-x   6 licha  staff   192 Nov  6 21:02 \x1b[1;34maureole-framework\x1b[0m\r\n-rw-r--r--   1 licha  staff  1179 Nov  6 22:12 Cargo.toml\r\n\x1b[1;32m➜\x1b[0m \x1b[36m~/项目\x1b[0m ";
    const LOG: &str = "\x1b[1;32m➜\x1b[0m \x1b[36mnimbus-terminal\x1b[0m tail -f server.log\r\n\x1b[90m22:31:07\x1b[0m \x1b[32mINFO\x1b[0m  listening on 127.0.0.1:3000\r\n\x1b[90m22:31:12\x1b[0m \x1b[32mINFO\x1b[0m  GET /api/tabs 200 3ms\r\n\x1b[90m22:31:14\x1b[0m \x1b[33mWARN\x1b[0m  slow query 412ms\r\n\x1b[90m22:31:19\x1b[0m \x1b[32mINFO\x1b[0m  GET /api/panes 200 5ms\r\n";
    let project = directory.join("项目");
    let scene = |dark: bool| {
        let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
        harness.set_scale_factor(1.5);
        workspace.form.appearance.set(if dark {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        });
        std::fs::create_dir_all(&project).expect("建得了目录");
        let folder = workspace.add_directory(&project).expect("是目录");
        workspace.create_folder("实验");
        workspace.new_tab_in(folder);
        workspace.new_tab_in(folder);
        workspace.new_tab();
        let tabs = workspace.tabs.get_untracked();
        tabs[0].session().feed_for_test(LISTING);
        tabs[1].session().feed_for_test(BUILD);
        tabs[2].session().feed_for_test(LOG);
        workspace.select(2);
        (harness, workspace)
    };
    let (harness, _workspace) = scene(false);
    save(&harness, "light.png");
    let (harness, workspace) = scene(true);
    save(&harness, "dark.png");
    // 分屏
    workspace.select(2);
    workspace.split(Axis::Columns);
    workspace.tabs.get_untracked()[1]
        .session()
        .feed_for_test(LISTING);
    workspace.split(Axis::Rows);
    workspace.tabs.get_untracked()[1]
        .session()
        .feed_for_test(LOG);
    save(&harness, "split.png");
    // 设置
    workspace.open_settings();
    workspace.settings_page.set(SettingsPage::Appearance);
    save(&harness, "settings.png");
    let _ = std::fs::remove_dir_all(&project);
}

/// 评审右侧面板：`UI_SHOTS=目录 cargo test screenshots_for_panel -- --ignored`
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_panel() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let (harness, workspace) = mounted();
    harness.set_scale_factor(2.0);
    for name in ["项目", "临时"] {
        workspace.create_folder(name);
    }
    workspace.new_tab();
    make_remote(&workspace, 2);
    harness.frame();
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(PathBuf::from(directory).join("panel.png"))
        .expect("存得下 PNG");
}

/// 评审服务器相关界面：`UI_SHOTS=目录 cargo test screenshots_for_remote -- --ignored`
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_remote() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    harness.set_scale_factor(2.0);
    workspace.new_tab();
    workspace.add_config_hosts(vec!["prod-hk".into(), "staging".into()]);
    workspace.new_tab_ssh("prod-hk");
    harness.frame();
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(directory.join("remote.png"))
        .expect("存得下 PNG");
    workspace.connect_prompt.set(true);
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(directory.join("connect.png"))
        .expect("存得下 PNG");
}

/// 评审监控页：`UI_SHOTS=目录 cargo test screenshots_for_monitor -- --ignored`
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_monitor() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    harness.set_scale_factor(2.0);
    workspace.polling.set(true);
    workspace.new_tab_ssh("prod-hk");
    let (mut busy, mut total) = (0u64, 0u64);
    for step in 0..60u64 {
        total += 200;
        busy += (60.0 + 35.0 * (step as f64 * 0.35).sin()) as u64 * 2;
        workspace.apply_reading(1, Ok(reading(busy, total)));
        if step == 7 {
            // 刚连上不久：只有几个采样
            harness.frame();
            harness.advance(secs(1.0));
            harness
                .screenshot()
                .save_png(PathBuf::from(&directory).join("monitor-short.png"))
                .expect("存得下 PNG");
        }
    }
    harness.frame();
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(PathBuf::from(directory).join("monitor.png"))
        .expect("存得下 PNG");
}

/// 评审文件页：`UI_SHOTS=目录 cargo test screenshots_for_files -- --ignored`
#[test]
#[ignore = "出图用，不是检查"]
fn screenshots_for_files() {
    let Ok(directory) = std::env::var("UI_SHOTS") else {
        return;
    };
    let (harness, workspace, _remote) = mounted_files();
    harness.set_scale_factor(2.0);
    settle(&harness, || {
        files_of(&workspace).status == crate::files::Status::Ready
    });
    workspace.files_upload(vec![
        PathBuf::from("/Users/me/new.txt"),
        PathBuf::from("/Users/me/readme.txt"),
    ]);
    workspace.files_download("readme.txt");
    settle(&harness, || files_of(&workspace).transfers.len() == 3);
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(PathBuf::from(&directory).join("files.png"))
        .expect("存得下 PNG");
    harness.click_named("files-path-button");
    harness.advance(secs(1.0));
    harness
        .screenshot()
        .save_png(PathBuf::from(&directory).join("files-edit.png"))
        .expect("存得下 PNG");
}
