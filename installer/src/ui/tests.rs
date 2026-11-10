//! 安装界面：每种状态显示的字和按钮；点「立即安装」真的走完引擎（预览模式：解压到临时目录，不碰系统）。
//!
//! 看样子：`NIMBUS_INSTALLER_SHOTS=目录 cargo test -p nimbus-installer screenshots`，每种状态浅色、深色各存一张

use std::path::PathBuf;
use std::time::{Duration, Instant};

use aureole::testing::Harness;
use aureole_installer::{Manifest, Package, Progress, Session, State};

use super::*;

fn package() -> Package {
    Package::preview(Manifest {
        name: "Nimbus".into(),
        identifier: "com.ruoyu-lab.nimbus".into(),
        version: "0.2.1".into(),
        publisher: "Nimbus".into(),
        executable: "nimbus-terminal.exe".into(),
        ..Manifest::default()
    })
}

/// 每个测试自己的临时目录
fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("nimbus-installer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn mount(session: Session, dark: bool) -> Harness {
    let harness = Harness::new(WIDTH, HEIGHT, view(session, dark));
    harness.frame();
    harness
}

fn click(harness: &Harness, name: &str) {
    let at = harness
        .bounds(name)
        .unwrap_or_else(|| panic!("没有 {name}"));
    harness.click(at.x + at.width / 2.0, at.y + at.height / 2.0);
    harness.frame();
}

fn wait_until_finished(harness: &Harness, session: Session) {
    let start = Instant::now();
    while session.state().get_untracked() == State::Running {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "装了 30 秒还没完"
        );
        std::thread::sleep(Duration::from_millis(2));
        harness.advance(Duration::from_millis(33));
    }
    harness.frame();
}

/// 第一次装：名字、版本、一句介绍、装到哪、桌面快捷方式；点「立即安装」走完，换成对勾和「打开 Nimbus」
#[test]
fn installing_goes_from_one_button_to_done() {
    let target = scratch("install").join("Nimbus");
    let session = Session::preview(package(), &target);
    let harness = mount(session, false);
    assert_eq!(harness.text("标题").as_deref(), Some("Nimbus"));
    assert_eq!(harness.text("版本").as_deref(), Some(VERSION));
    assert_eq!(harness.text("说明").as_deref(), Some(TAGLINE));
    assert!(
        harness
            .text("安装位置")
            .is_some_and(|text| text.starts_with("安装到 ") && text.ends_with("Nimbus")),
        "{:?}",
        harness.text("安装位置")
    );
    assert!(harness.bounds("桌面快捷方式").is_some());
    assert!(harness.bounds("链接：更改").is_some());
    click(&harness, "按钮：立即安装");
    wait_until_finished(&harness, session);
    assert_eq!(session.state().get_untracked(), State::Done);
    assert_eq!(harness.text("标题").as_deref(), Some("装好了"));
    assert!(harness.bounds("完成").is_some(), "图标换成对勾");
    assert!(harness.bounds("按钮：打开 Nimbus").is_some());
    assert!(harness.bounds("链接：完成").is_some());
    assert!(harness.bounds("桌面快捷方式").is_none(), "装完就不用选了");
    let _ = std::fs::remove_dir_all(target.parent().expect("有上一级"));
}

/// 目录里已经装过：说清楚是从哪个版本更新，按钮叫「立即更新」；同一个版本是重新安装
#[test]
fn an_existing_install_reads_as_an_update() {
    let session = Session::preview(package(), scratch("update"));
    let harness = mount(session, false);
    session.previous_version().set(Some("0.2.0".into()));
    harness.frame();
    assert_eq!(
        harness.text("说明").as_deref(),
        Some(format!("会从 0.2.0 更新到 {VERSION}").as_str())
    );
    assert!(harness.bounds("按钮：立即更新").is_some());
    assert!(harness.bounds("按钮：立即安装").is_none());
    session.previous_version().set(Some("0.2.1".into()));
    harness.frame();
    assert_eq!(
        harness.text("说明").as_deref(),
        Some(format!("这台电脑上已经装了，会重新安装 {VERSION}").as_str())
    );
}

/// 装的时候：按钮换成进度条，下面写百分比
#[test]
fn running_shows_a_thin_progress_bar() {
    let session = Session::preview(package(), scratch("running"));
    let harness = mount(session, false);
    session.state().set(State::Running);
    session.progress().set(Progress {
        fraction: 0.42,
        ..session.progress().get_untracked()
    });
    harness.frame();
    assert!(harness.bounds("进度条").is_some());
    assert!(harness.bounds("按钮：立即安装").is_none());
    assert_eq!(harness.text("进度").as_deref(), Some("正在安装… 42%"));
}

/// 失败：Nimbus 还开着时说人话；别的原因原样写出来
#[test]
fn failures_say_what_to_do() {
    let session = Session::preview(package(), scratch("failed"));
    let harness = mount(session, false);
    session.state().set(State::Failed {
        message: r"C:\x\nimbus-terminal.exe 正被使用，关掉应用后再试".into(),
        file_in_use: true,
    });
    harness.frame();
    assert_eq!(harness.text("标题").as_deref(), Some("没装成"));
    assert_eq!(
        harness.text("说明").as_deref(),
        Some("Nimbus 还开着：关掉它以后再试一次")
    );
    assert!(harness.bounds("按钮：再试一次").is_some());
    session.state().set(State::Ready);
    harness.frame();
    session.state().set(State::Failed {
        message: "磁盘满了".into(),
        file_in_use: false,
    });
    harness.frame();
    assert_eq!(harness.text("说明").as_deref(), Some("磁盘满了"));
}

/// 卸载：同一个样子，说清楚设置会留着
#[test]
fn uninstalling_says_settings_stay() {
    let session = Session::preview_uninstall(package(), scratch("uninstall"));
    let harness = mount(session, false);
    assert_eq!(harness.text("标题").as_deref(), Some("卸载 Nimbus"));
    assert!(
        harness
            .text("说明")
            .is_some_and(|text| text.contains("设置")),
    );
    assert!(harness.bounds("按钮：卸载").is_some());
    assert!(harness.bounds("链接：取消").is_some());
    assert!(harness.bounds("桌面快捷方式").is_none());
}

/// 存图看样子（设了 `NIMBUS_INSTALLER_SHOTS` 才跑）
#[test]
fn screenshots() {
    let Some(out) = std::env::var_os("NIMBUS_INSTALLER_SHOTS").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).expect("建目录");
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let shot = |harness: &Harness, name: &str| {
            // 图标在后台线程解码：等它画出来
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(10));
                harness.advance(Duration::from_millis(16));
            }
            harness.frame();
            harness
                .screenshot()
                .save_png(out.join(format!("{theme}-{name}.png")))
                .expect("存截图");
        };
        let directory = PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Nimbus");
        let session = Session::preview(package(), &directory);
        let harness = mount(session, dark);
        shot(&harness, "1-ready");
        session.state().set(State::Running);
        session.progress().set(Progress {
            fraction: 0.62,
            ..session.progress().get_untracked()
        });
        shot(&harness, "2-running");
        session.state().set(State::Done);
        shot(&harness, "3-done");
        session.state().set(State::Failed {
            message: String::new(),
            file_in_use: true,
        });
        shot(&harness, "4-failed");
        let update = Session::preview(package(), &directory);
        let harness = mount(update, dark);
        update.previous_version().set(Some("0.2.0".into()));
        shot(&harness, "5-update");
        let uninstall = Session::preview_uninstall(package(), &directory);
        let harness = mount(uninstall, dark);
        shot(&harness, "6-uninstall");
        uninstall.state().set(State::Done);
        shot(&harness, "7-uninstalled");
    }
}
