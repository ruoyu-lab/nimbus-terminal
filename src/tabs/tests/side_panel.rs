//! 右侧面板：监控、显示和隐藏。

use super::*;

/// 关掉最后一个标签：窗口还在，面板上写没有标签；再新建就又能用
#[test]
fn closing_the_last_tab_leaves_an_empty_panel() {
    let (harness, workspace) = mounted();
    command(&harness, "w");
    assert!(ids(&workspace).is_empty(), "会话都关了");
    assert!(harness.bounds("card").is_some(), "窗口还在");
    assert!(harness.bounds("grid").is_none(), "没有终端");
    assert_eq!(harness.text("no-tabs").as_deref(), Some("没有标签"));
    assert!(
        harness.bounds("status-target").is_none(),
        "没有会话就不显示状态条"
    );
    command(&harness, "t");
    assert_eq!(ids(&workspace), [2]);
    assert!(harness.bounds("grid").is_some(), "新建以后终端回来了");
    assert!(harness.bounds("no-tabs").is_none());
}

/// 只用本机的人：没有右侧面板，标题栏上也没有它的开关，命令面板里没有这条命令
#[test]
fn local_sessions_have_no_right_panel_at_all() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_none());
    assert!(harness.bounds("toggle-panel").is_none());
    assert!(!workspace.has_panels());
}

/// 连到服务器的会话：面板自己出现，可以收起、拖宽；换回本机的标签它就消失
#[test]
fn a_remote_session_shows_the_panel_and_a_local_one_hides_it() {
    let (harness, workspace) = mounted();
    workspace.new_tab();
    make_remote(&workspace, 2);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_some(), "服务器会话有面板");
    assert!(harness.bounds("toggle-panel").is_some());
    assert!(
        harness.bounds("panel-tab-monitor").is_some()
            && harness.bounds("panel-tab-files").is_some()
    );
    // 卡片和面板都在窗口里，互不压住
    let (card, panel) = (
        harness.bounds("card").unwrap(),
        harness.bounds("panel").unwrap(),
    );
    let panel_card = harness.bounds("panel-card").unwrap();
    assert!(
        (panel_card.x + panel_card.width - (WINDOW_SIZE.0 as f32 - 8.0)).abs() < 0.5,
        "面板卡片离窗口右边留 8：{panel_card:?}"
    );
    assert!(
        card.x + card.width <= panel.x + 0.5,
        "主卡片 {card:?} 不能压到面板 {panel:?}"
    );
    assert!(panel.x + panel.width <= WINDOW_SIZE.0 as f32 + 0.5);
    // 换回本机标签
    workspace.select(1);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_none(), "本机标签没有面板");
    // 再切回来，面板又出现（开关几次都没事）
    workspace.select(2);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_some());
    harness.click_named("toggle-panel");
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_none(), "点开关收起");
    harness.click_named("toggle-panel");
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_some(), "再点展开");
}

/// 按住面板左边的空隙往左拖，面板变宽；拖不过范围；宽度会存下来
#[test]
fn dragging_the_panel_edge_resizes_it_within_limits() {
    let (harness, workspace) = mounted();
    make_remote(&workspace, 1);
    harness.frame();
    harness.advance(secs(1.0));
    let before = workspace.panel_width.get_untracked();
    let [x, y] = harness.bounds("panel-resize").expect("拖动条在").center();
    harness.mouse_move(x, y);
    assert_eq!(harness.cursor(), Cursor::ResizeHorizontal);
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 40.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert!(
        (workspace.panel_width.get_untracked() - (before + 40.0)).abs() < 1.0,
        "往左拖 40，宽了 40"
    );
    harness.mouse_down(x - 40.0, y, MouseButton::Left);
    harness.mouse_move(x - 900.0, y);
    harness.mouse_up(MouseButton::Left);
    assert_eq!(
        workspace.panel_width.get_untracked(),
        crate::chrome::PANEL_MAX
    );
    assert_eq!(harness.window_drag_count(), 0);
}

/// 设置里关掉「远程功能」：服务器会话也不显示面板和开关
#[test]
fn turning_off_remote_features_hides_the_panel_everywhere() {
    let (harness, workspace) = mounted();
    make_remote(&workspace, 1);
    harness.frame();
    assert!(harness.bounds("panel").is_some());
    workspace.form.remote.set(false);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_none());
    assert!(harness.bounds("toggle-panel").is_none());
}

/// 读数进来，监控页上就有了：运行时间、内存、磁盘、负载；第二次读数才有 CPU
#[test]
fn readings_show_up_on_the_monitor_page() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true); // 不让页面挂上时真的去取数：这里只测读数怎么显示
    workspace.new_tab_ssh("prod-hk");
    harness.frame();
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("monitor").is_some(),
        "服务器标签的面板默认是监控页"
    );
    assert_eq!(harness.text("monitor-status").as_deref(), Some("正在读取…"));
    workspace.apply_reading(1, Ok(reading(0, 100)));
    workspace.apply_reading(1, Ok(reading(50, 200)));
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(
        harness.text("monitor-status").as_deref(),
        Some("运行了 3 天 4 小时")
    );
    assert!(
        harness.bounds("monitor-memory-bar").is_some()
            && harness.bounds("monitor-disk-bar").is_some()
    );
    assert!(harness.bounds("monitor-cpu-line").is_some());
    let page = harness.element_tree().to_string();
    for expected in [
        "50%",
        "15.6 GB / 7.8 GB"
            .replace("15.6 GB / 7.8 GB", "7.8 GB / 15.6 GB")
            .as_str(),
        "0.52  0.61  0.70",
    ] {
        assert!(page.contains(expected), "监控页上要有 {expected}：{page}");
    }
}

/// 失败时写明原因，数字留着；断线、不支持各有各的说法
#[test]
fn failures_are_explained_on_the_page() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true);
    workspace.new_tab_ssh("prod-hk");
    workspace.apply_reading(1, Ok(reading(0, 100)));
    for (failure, expected) in [
        (Failure::NotConnected, "未连接"),
        (Failure::Unsupported, "暂不支持监控"),
        (Failure::Failed("超时".into()), "读取失败：超时"),
    ] {
        workspace.apply_reading(1, Err(failure));
        harness.frame();
        harness.advance(secs(1.0));
        assert!(
            harness
                .text("monitor-status")
                .unwrap_or_default()
                .contains(expected),
            "{expected}"
        );
    }
    assert!(
        workspace.monitors.get_untracked()[&1].latest.is_some(),
        "最后的数字还在"
    );
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::Failed("超时".into())
    );
}

/// 面板自己的连接要密码（Windows 上只能输密码的服务器）：监控页上是密码框；输错了写「密码不对」；
/// 输了点「连接」：记下这台服务器的密码，框清空，马上再取一次（回到「正在读取」）。文件页同样有这个框
#[test]
fn a_password_server_asks_for_its_password_on_the_panel() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true); // 不真的去连：这里只测界面
    workspace.new_tab_ssh("lab-pass");
    harness.frame();
    harness.advance(secs(1.0));
    workspace.apply_reading(1, Err(Failure::PasswordNeeded { wrong: false }));
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel-password").is_some());
    assert_eq!(
        harness.text("panel-password-note").as_deref(),
        Some("输一次，监控和文件页都能用。")
    );
    assert!(
        harness
            .text("monitor-status")
            .is_some_and(|status| status.contains("要输密码"))
    );
    // 看样子：`NIMBUS_PANEL_SHOTS=文件 cargo test a_password_server_asks` 存一张监控页的图
    if let Some(out) = std::env::var_os("NIMBUS_PANEL_SHOTS") {
        harness
            .screenshot()
            .save_png(std::path::PathBuf::from(out))
            .expect("存截图");
    }
    workspace.apply_reading(1, Err(Failure::PasswordNeeded { wrong: true }));
    harness.frame();
    assert_eq!(
        harness.text("panel-password-note").as_deref(),
        Some("密码不对，再输一次。")
    );
    click(&harness, "panel-password-field");
    harness.type_text("s3cret");
    click(&harness, "panel-password-connect");
    harness.advance(secs(0.5));
    assert_eq!(
        crate::ssh::take_password("lab-pass").as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::Waiting,
        "马上再取一次"
    );
    assert!(harness.bounds("panel-password").is_none());
    // 文件页：读目录时服务器要密码，也是这个框
    let remote = Arc::new(FakeRemote::default());
    remote.fail_with(Some(Failure::PasswordNeeded { wrong: false }));
    *workspace.remote.borrow_mut() = remote;
    workspace.panel_tab.set(crate::target::PanelKind::Files);
    harness.frame();
    harness.advance(secs(1.0));
    settle(&harness, || {
        workspace
            .files
            .get_untracked()
            .get(&1)
            .is_some_and(|state| {
                state.status == crate::files::Status::PasswordNeeded { wrong: false }
            })
    });
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel-password").is_some(), "文件页上也有");
    assert!(
        harness
            .text("files-notice")
            .is_some_and(|notice| notice.contains("要输密码"))
    );
}

/// 标签关了，它的监控数据跟着清掉；已经关了的标签迟到的读数丢掉
#[test]
fn closing_a_tab_drops_its_monitor_data() {
    let (_harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab_ssh("prod-hk");
    workspace.new_tab();
    workspace.apply_reading(1, Ok(reading(0, 100)));
    assert!(workspace.monitors.get_untracked().contains_key(&1));
    workspace.close_tab(1);
    assert!(workspace.monitors.get_untracked().is_empty());
    workspace.apply_reading(1, Ok(reading(1, 200)));
    assert!(
        workspace.monitors.get_untracked().is_empty(),
        "关了的标签不收读数"
    );
}

/// 真的走一遍取数：后台线程里借套接字跑 ssh，套接字不在（测试里没有真连接）就是「未连接」，
/// 结果回到主线程写进状态；取数没回来之前不叠着发
#[test]
fn polling_runs_in_the_background_and_reports_not_connected_without_a_connection() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab_ssh("prod-hk");
    workspace.poll_monitor();
    workspace.poll_monitor();
    for _ in 0..50 {
        harness.advance(ms(50));
        if workspace.monitors.get_untracked().contains_key(&1) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::NotConnected
    );
    // 本机标签：什么也不发，也不会卡在「取数中」
    workspace.new_tab();
    workspace.poll_monitor();
    workspace.poll_monitor();
    assert!(!workspace.monitors.get_untracked().contains_key(&2));
}

/// 设置里关了远程功能：不取数
#[test]
fn nothing_is_polled_when_remote_features_are_off() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.form.remote.set(false);
    workspace.new_tab_ssh("prod-hk");
    workspace.poll_monitor();
    for _ in 0..5 {
        harness.advance(ms(50));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(workspace.monitors.get_untracked().is_empty());
}

/// 取数在途的时候监控页卸掉了（关了标签、收起面板、换页）：任务不能跟着被取消，
/// 否则「取数中」的标记永远是真，重新打开以后一直停在「正在读取」
#[test]
fn a_poll_in_flight_survives_the_monitor_page_unmounting() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    // 一个卡住的取数（真的 ssh 要几秒）：放行之前不返回
    let release = Arc::new(AtomicBool::new(false));
    let gate = release.clone();
    *workspace.fetch.borrow_mut() = Arc::new(move |_, _| {
        while !gate.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(Failure::NotConnected)
    });
    workspace.new_tab_ssh("prod-hk");
    for _ in 0..5 {
        harness.frame();
        if workspace.polling.get() {
            break;
        }
    }
    assert!(workspace.polling.get(), "页面一挂上就取了，还没回来");
    workspace.toggle_panel();
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("monitor").is_none(), "页面卸掉了");
    release.store(true, Ordering::Release);
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        harness.advance(ms(50));
        if !workspace.polling.get() {
            break;
        }
    }
    assert!(
        !workspace.polling.get(),
        "页面卸掉以后任务也要跑完，不能一直占着「取数中」"
    );
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::NotConnected
    );
    // 再打开：又能取了，数据能进来
    *workspace.fetch.borrow_mut() = Arc::new(|_, _| Err(Failure::Unsupported));
    workspace.toggle_panel();
    for _ in 0..100 {
        harness.advance(ms(50));
        std::thread::sleep(std::time::Duration::from_millis(10));
        if workspace.monitors.get_untracked()[&1].status == Status::Unsupported {
            break;
        }
    }
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::Unsupported
    );
}

/// 面板展开、收起有动画：展开时宽度逐渐变大（不是一下子到位），收起时面板还在一会儿再消失；拖宽度仍然跟手
#[test]
fn the_panel_animates_open_and_closed() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true); // 这里不测取数
    workspace.new_tab_ssh("prod-hk");
    harness.frame();
    harness.advance(secs(1.0));
    let target = workspace.panel_width.get_untracked();
    let width = |harness: &Harness| harness.bounds("panel").map(|panel| panel.width);
    assert_eq!(width(&harness), Some(target), "展开完是目标宽度");
    // 收起：马上还在，慢慢变窄，最后没了
    workspace.toggle_panel();
    harness.frame();
    harness.advance(ms(40));
    let closing = width(&harness).expect("退场动画期间面板还在");
    assert!(
        closing > 0.0 && closing < target,
        "收起中：{closing}（目标 {target}）"
    );
    harness.advance(secs(1.0));
    assert!(harness.bounds("panel").is_none(), "收完就卸掉了");
    // 展开：从窄变宽
    workspace.toggle_panel();
    harness.frame();
    harness.advance(ms(40));
    let opening = width(&harness).expect("面板挂上了");
    assert!(opening < target, "展开中：{opening}（目标 {target}）");
    harness.advance(secs(1.0));
    assert_eq!(width(&harness), Some(target));
    // 拖宽度不走弹簧：宽度一变马上就是新宽度
    let [x, y] = harness.bounds("panel-resize").expect("拖动条在").center();
    harness.mouse_down(x, y, MouseButton::Left);
    harness.mouse_move(x - 50.0, y);
    harness.mouse_up(MouseButton::Left);
    harness.frame();
    assert_eq!(
        width(&harness),
        Some(target + 50.0),
        "拖的时候跟手，没有动画拖着"
    );
}
