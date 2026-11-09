//! 容器目标。

use super::*;

/// 开一个容器标签：按 docker exec 起（没有本机目录、没有 SSH 套接字），标签叫容器名，面板自己出现；
/// 手动连过的容器记进侧栏的名单
#[test]
fn opening_a_container_tab_connects_names_the_tab_and_shows_the_panel() {
    let (harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true); // 不让面板挂上时真的去取数
    assert!(workspace.new_tab_docker("  web-1 "));
    harness.frame();
    harness.advance(secs(1.0));
    let launch = fake.launch(0);
    assert_eq!(
        launch.target,
        Target::Docker {
            container: "web-1".into()
        }
    );
    assert_eq!(launch.directory, None, "容器里没有本机的目录");
    assert_eq!(launch.control, None, "容器没有 SSH 的复用套接字");
    assert_eq!(
        workspace.titles.get_untracked().get(&1).map(String::as_str),
        Some("web-1")
    );
    assert_eq!(harness.text("status-target").as_deref(), Some("web-1"));
    assert!(harness.bounds("panel").is_some(), "连到容器的会话有面板");
    assert!(
        workspace
            .containers
            .get_untracked()
            .contains(&"web-1".to_string())
    );
    let (tab, target, control) = workspace.monitor_plan().expect("容器标签要监控");
    assert_eq!((tab, target.container(), control), (1, Some("web-1"), None));
    // 本机标签不受影响
    workspace.new_tab();
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(fake.launch(1).target, Target::Local);
    assert!(harness.bounds("panel").is_none());
}

/// 容器名不能被 docker 当成选项：不开标签，也不起任何进程
#[test]
fn a_container_name_that_looks_like_an_option_is_refused() {
    let (_harness, workspace, fake) = blank_with_traffic_lights(Store::none());
    for name in ["-it", "", "a b", "a;b", "$(id)", "a:b"] {
        assert!(!workspace.new_tab_docker(name), "{name:?}");
    }
    assert!(ids(&workspace).is_empty());
    assert!(fake.launches.borrow().is_empty());
}

/// 侧栏的「容器」：启动时列一遍运行中的容器；点一个就连上去，再点就切过去不重复开；右键能再开一个；
/// 设置里关了「远程功能」整组消失
#[test]
fn the_containers_group_lists_running_containers_and_opens_sessions() {
    let fake = Fake::default();
    let workspace = Workspace::new(fake.spawn(), None, Store::none());
    // 在界面跑起来之前换成假的名单：这台机器上可能真的有 docker，不能让它的成绩单盖过来
    *workspace.container_lister.borrow_mut() =
        Arc::new(|| Ok(vec!["web-1".to_string(), "db".to_string()]));
    workspace.polling.set(true);
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.set_title_bar_info(title_bar());
    harness.frame();
    settle(&harness, || workspace.containers.get_untracked().len() == 2);
    harness.advance(secs(1.0));
    assert!(harness.bounds("containers").is_some());
    assert!(
        harness.bounds("container-web-1").is_some() && harness.bounds("container-db").is_some()
    );
    harness.click_named("container-web-1");
    harness.advance(secs(1.0));
    assert_eq!(docker_container(&workspace, 1).as_deref(), Some("web-1"));
    assert_eq!(
        fake.launch(0).target,
        Target::Docker {
            container: "web-1".into()
        }
    );
    // 已经有会话就切过去，不重复开
    workspace.new_tab();
    harness.frame();
    harness.click_named("container-web-1");
    harness.advance(secs(1.0));
    assert_eq!(ids(&workspace), [1, 2]);
    assert_eq!(workspace.active.get_untracked(), 1);
    // 右键可以再开一个
    right_click_named(&harness, "container-web-1");
    click_menu(&harness, "menu-new-session-container-web-1");
    settle(&harness, || ids(&workspace).len() == 3);
    assert_eq!(docker_container(&workspace, 3).as_deref(), Some("web-1"));
    // 关了「远程功能」：整组消失
    workspace.form.remote.set(false);
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("containers").is_none());
}

/// 重启后容器的标签回来，但不自己连：停在「已断开」，按回车才连；手动连过的容器也记住了
#[test]
fn container_tabs_come_back_disconnected_and_wait_for_enter() {
    let root = directory("docker-restore");
    let store = Store::at(root.join("config"));
    {
        let (_harness, workspace, _fake) = blank_with_traffic_lights(store.clone());
        workspace.polling.set(true);
        workspace.new_tab_docker("web-1");
        workspace.new_tab();
        workspace.save_now();
    }
    let (harness, workspace, fake) = blank_with_traffic_lights(store);
    workspace.polling.set(true);
    assert!(
        workspace
            .containers
            .get_untracked()
            .contains(&"web-1".to_string()),
        "手动连过的存下来了"
    );
    assert_eq!(workspace.restore(), 2);
    harness.frame();
    assert_eq!(docker_container(&workspace, 1).as_deref(), Some("web-1"));
    assert_eq!(
        fake.launches.borrow().len(),
        1,
        "只有本机的标签起了进程，容器的没连"
    );
    assert_eq!(fake.launch(0).target, Target::Local);
    workspace.select(1);
    harness.frame();
    let session = workspace.tabs.get_untracked()[0].session();
    assert!(
        session
            .screen_text_for_test()
            .replace('\0', "")
            .contains("已断开 web-1")
    );
    harness.key_down(NamedKey::Enter);
    harness.frame();
    assert_eq!(fake.launches.borrow().len(), 2);
    assert_eq!(
        fake.launch(1).target,
        Target::Docker {
            container: "web-1".into()
        }
    );
    std::fs::remove_dir_all(root).ok();
}

/// 「连接到容器…」的输入框：填不能用的名字留在原地并写明原因，填对了就连
#[test]
fn the_docker_dialog_refuses_bad_names_and_connects_good_ones() {
    let (harness, workspace) = mounted();
    workspace.polling.set(true);
    workspace.docker_prompt.set(true);
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(harness.focused_name().as_deref(), Some("docker-field"));
    harness.type_text("-it");
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(
        workspace.docker_prompt.get_untracked(),
        "名字不能用，框还开着"
    );
    assert!(
        harness
            .text("docker-hint")
            .unwrap_or_default()
            .contains("不能用")
    );
    assert_eq!(ids(&workspace), [1]);
    harness.key_down_with(Key::Character("a".into()), Modifiers::SUPER);
    harness.type_text("web-1");
    harness.key_down(NamedKey::Enter);
    harness.advance(secs(1.0));
    assert!(!workspace.docker_prompt.get_untracked());
    assert_eq!(docker_container(&workspace, 2).as_deref(), Some("web-1"));
}

/// 取容器监控数：目标整个传给取数函数，套接字是 None（那是 SSH 才有的）
#[test]
fn polling_a_container_passes_the_target_without_a_control_socket() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    *workspace.fetch.borrow_mut() = Arc::new(move |target: &Target, control: Option<&Path>| {
        record
            .lock()
            .expect("锁")
            .push((target.clone(), control.map(Path::to_path_buf)));
        Err(Failure::NotConnected)
    });
    workspace.new_tab_docker("web-1");
    settle(&harness, || {
        workspace.monitors.get_untracked().contains_key(&1)
    });
    assert_eq!(
        seen.lock().expect("锁").as_slice(),
        [(
            Target::Docker {
                container: "web-1".into()
            },
            None
        )],
        "面板一挂上就取了一次"
    );
    assert_eq!(
        workspace.monitors.get_untracked()[&1].status,
        Status::NotConnected
    );
}

/// 容器的读数：CPU 是百分比，第一次就有；运行时间、磁盘、负载取不到就不写不画，内存照常
#[test]
fn container_readings_show_percent_without_disk_or_load() {
    let (harness, workspace, _fake) = blank_with_traffic_lights(Store::none());
    workspace.polling.set(true);
    workspace.new_tab_docker("web-1");
    harness.frame();
    harness.advance(secs(1.0));
    assert!(harness.bounds("monitor").is_some());
    workspace.apply_reading(
        1,
        Ok(Reading {
            cpu: CpuSample::Percent(42.0),
            mem_total: 2 * 1024 * 1024,
            mem_available: 1024 * 1024,
            disk: None,
            load: None,
            uptime: None,
        }),
    );
    harness.frame();
    harness.advance(secs(1.0));
    assert_eq!(
        harness.text("monitor-status").as_deref(),
        Some("已连接"),
        "运行时间取不到就不写"
    );
    assert!(harness.bounds("monitor-memory-bar").is_some());
    assert!(
        harness.bounds("monitor-disk-bar").is_none(),
        "取不到磁盘就不画"
    );
    assert!(
        harness.bounds("monitor-cpu-line").is_some(),
        "百分比第一次就能画"
    );
    let page = harness.element_tree().to_string();
    assert!(page.contains("42%"), "监控页上要有 42%：{page}");
    assert!(page.contains("1.0 GB / 2.0 GB"), "监控页上要有内存：{page}");
    assert!(!page.contains("磁盘"), "没有磁盘块：{page}");
    assert!(!page.contains("负载"), "容器没有负载块：{page}");
}

/// 真容器里的终端：`NIMBUS_DOCKER_LAB=容器名 cargo test a_real_container_gives`。
/// `docker exec` 放进真 pty：屏幕上有提示符，打的字真的进容器的 shell。平时直接返回，不碰 Docker
#[test]
fn a_real_container_gives_an_interactive_shell() {
    let Ok(container) = std::env::var("NIMBUS_DOCKER_LAB") else {
        return;
    };
    let workspace = Workspace::new(Rc::new(crate::app::spawn), None, Store::none());
    *workspace.container_lister.borrow_mut() = Arc::new(|| Ok(Vec::new()));
    workspace.polling.set(true); // 这里只测终端，面板不取数
    let harness = Harness::new(
        WINDOW_SIZE.0 as f32,
        WINDOW_SIZE.1 as f32,
        chrome::build(workspace.clone()),
    );
    harness.frame();
    assert!(workspace.new_tab_docker(&container));
    harness.frame();
    let session = workspace.tabs.get_untracked()[0].session().clone();
    let mut saw = String::new();
    for _ in 0..60 {
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
    // 打个算出来的数：回显里只有命令本身，答案只能是容器里的 shell 说的
    harness.type_text("echo nimbus-$((6*7))");
    harness.key_down(NamedKey::Enter);
    let mut answered = String::new();
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        harness.advance(ms(50));
        answered = session.screen_text_for_test();
        if answered.contains("nimbus-42") {
            break;
        }
    }
    assert!(
        answered.contains("nimbus-42"),
        "容器里的 shell 没回话：{answered:?}"
    );
    workspace.close_tab(1);
}
