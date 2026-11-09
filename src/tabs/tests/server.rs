//! 服务器目标：连接、重连、右侧面板。

use super::*;

/// 开一个服务器标签：按 ssh 起（没有本机的起始目录），标签叫主机名，右侧面板自己出现
#[test]
fn opening_a_server_tab_connects_names_the_tab_and_shows_the_panel() {
    let (harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    assert!(workspace.new_tab_ssh("  prod-hk "));
    harness.frame();
    harness.advance(secs(1.0));
    let launch = fake.launch(0);
    assert_eq!(
        launch.target,
        Target::Ssh {
            host: "prod-hk".into()
        }
    );
    assert_eq!(launch.directory, None, "服务器上没有本机的目录");
    assert_eq!(
        workspace.titles.get_untracked().get(&1).map(String::as_str),
        Some("prod-hk")
    );
    assert_eq!(harness.text("status-target").as_deref(), Some("prod-hk"));
    assert!(harness.bounds("panel").is_some(), "连到服务器的会话有面板");
    // 本机标签不受影响
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(fake.launch(1).target, Target::Local);
    assert!(harness.bounds("panel").is_none());
}

/// 主机名不能被 ssh 当成选项：不开标签，也不起任何进程
#[test]
fn a_host_that_looks_like_an_option_is_refused() {
    let (_harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    for host in ["-oProxyCommand=x", "", "a b", "a;b", "$(id)"] {
        assert!(!workspace.new_tab_ssh(host), "{host:?}");
    }
    assert!(ids(&workspace).is_empty());
    assert!(fake.launches.borrow().is_empty());
}

/// 侧栏的「远程」：没有服务器时不存在；有了才出现；点主机连上去，再点就切过去不重复开
#[test]
fn the_remote_group_appears_only_when_there_are_servers() {
    let (harness, workspace) = mounted();
    harness.advance(secs(1.0));
    assert!(
        harness.bounds("remote").is_none(),
        "没有服务器就没有「远程」"
    );
    workspace.add_config_hosts(vec!["prod-hk".into(), "staging".into(), "prod-hk".into()]);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("remote").is_some());
    assert_eq!(
        workspace.hosts.get_untracked(),
        ["prod-hk", "staging"],
        "去重"
    );
    harness.click_named("host-prod-hk");
    harness.advance(secs(1.0));
    assert_eq!(ssh_host(&workspace, 2).as_deref(), Some("prod-hk"));
    workspace.select(1);
    harness.click_named("host-prod-hk");
    harness.advance(secs(1.0));
    assert_eq!(ids(&workspace), [1, 2], "已经有会话就切过去，不重复开");
    assert_eq!(workspace.active.get_untracked(), 2);
    // 设置里关了「远程功能」：整组消失
    workspace.form.remote.set(false);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("remote").is_none());
}

/// 服务器会话断线（ssh 非 0 退出）：写明原因，等回车重连；回车之后用同一个主机再起一次，别的键不往死连接里写
#[test]
fn a_dropped_connection_waits_for_enter_and_reconnects() {
    let (harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab_ssh("prod-hk");
    harness.frame();
    let session = workspace.tabs.get_untracked()[0].session();
    session.exited_for_test(255);
    harness.frame();
    assert!(
        session
            .screen_text_for_test()
            .replace('\0', "")
            .contains("连接已断开，退出码 255")
    );
    assert_eq!(ids(&workspace), [1], "断线不关标签");
    harness.type_text("ls");
    assert_eq!(fake.pty(0).take(), "", "没有人听的连接里不写字");
    assert_eq!(fake.launches.borrow().len(), 1);
    harness.key_down(NamedKey::Enter);
    harness.frame();
    assert_eq!(fake.launches.borrow().len(), 2, "回车重连");
    assert_eq!(
        fake.launch(1).target,
        Target::Ssh {
            host: "prod-hk".into()
        }
    );
    // 重连之后又能打字了，写进新的连接
    harness.type_text("ls");
    assert_eq!(fake.pty(1).take(), "ls");
}

/// 服务器上正常退出（`exit`）标签关掉；本机的会话出错退出不提示重连
#[test]
fn a_clean_exit_closes_a_server_tab_and_local_failures_do_not_offer_reconnect() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab_ssh("prod-hk");
    workspace.new_tab();
    harness.frame();
    let tabs = workspace.tabs.get_untracked();
    tabs[1].session().exited_for_test(3);
    assert!(
        !tabs[1]
            .session()
            .screen_text_for_test()
            .replace('\0', "")
            .contains("重新连接")
    );
    tabs[0].session().exited_for_test(0);
    harness.frame();
    assert_eq!(ids(&workspace), [2]);
}

/// 重启后服务器的标签回来，但不自己连（不在启动时弹密码、host key 的提示）：停在「已断开」，按回车才连；
/// 手动连过的主机也记住了，`~/.ssh/config` 里的不存
#[test]
fn server_tabs_come_back_disconnected_and_wait_for_enter() {
    let root = directory("ssh-restore");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank_with_traffic_lights(store.clone());
        workspace.new_tab_ssh("prod-hk");
        workspace.new_tab();
        workspace.add_config_hosts(vec!["from-config".into()]);
        workspace.save_now();
    }
    let (harness, workspace, fake) = blank_with_traffic_lights(store);
    assert_eq!(
        workspace.hosts.get_untracked(),
        ["prod-hk"],
        "只有手动连过的存下来了"
    );
    assert_eq!(workspace.restore(), 2);
    harness.frame();
    assert_eq!(ssh_host(&workspace, 1).as_deref(), Some("prod-hk"));
    assert_eq!(
        fake.launches.borrow().len(),
        1,
        "只有本机的标签起了进程，服务器的没连"
    );
    assert_eq!(fake.launch(0).target, Target::Local);
    workspace.select(1);
    harness.frame();
    let session = workspace.tabs.get_untracked()[0].session();
    assert!(
        session
            .screen_text_for_test()
            .replace('\0', "")
            .contains("已断开 prod-hk")
    );
    harness.key_down(NamedKey::Enter);
    harness.frame();
    assert_eq!(fake.launches.borrow().len(), 2);
    assert_eq!(
        fake.launch(1).target,
        Target::Ssh {
            host: "prod-hk".into()
        }
    );
    std::fs::remove_dir_all(root).ok();
}

/// 命令面板里「连接到服务器…」：开输入框，填不能用的名字留在原地并写明原因，填对了就连
#[test]
fn the_connect_dialog_refuses_bad_names_and_connects_good_ones() {
    let (harness, workspace) = mounted();
    workspace.connect_prompt.set(true);
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(harness.focused_name().as_deref(), Some("connect-field"));
    harness.type_text("-oProxyCommand=x");
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(
        workspace.connect_prompt.get_untracked(),
        "名字不能用，框还开着"
    );
    assert!(
        harness
            .text("connect-hint")
            .unwrap_or_default()
            .contains("不能用")
    );
    assert_eq!(ids(&workspace), [1]);
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("prod-hk");
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(!workspace.connect_prompt.get_untracked());
    assert_eq!(ssh_host(&workspace, 2).as_deref(), Some("prod-hk"));
}

/// 本机标签没有监控的对象；服务器标签有（带复用套接字，监控借终端的连接）
#[test]
fn only_server_tabs_have_something_to_monitor() {
    let (_harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab();
    assert_eq!(workspace.monitor_plan(), None);
    workspace.new_tab_ssh("prod-hk");
    let (tab, target, control) = workspace.monitor_plan().expect("服务器标签要监控");
    assert_eq!((tab, target.host()), (2, Some("prod-hk")));
    let control = control.expect("服务器有套接字");
    assert!(control.to_string_lossy().contains("nimbus-"), "{control:?}");
}

/// 服务器标签的 ssh 带复用选项（终端当主连接）；拆出来的终端和本机的都不带，不抢主连接
#[test]
fn only_the_first_terminal_of_a_server_tab_owns_the_control_socket() {
    let (_harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    workspace.new_tab_ssh("prod-hk");
    workspace.split(Axis::Columns);
    workspace.new_tab();
    assert!(fake.launch(0).control.is_some(), "第一个终端是主连接");
    assert_eq!(fake.launch(1).control, None, "拆出来的各连各的");
    assert_eq!(fake.launch(2).control, None, "本机没有");
}
