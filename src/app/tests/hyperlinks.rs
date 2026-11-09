//! 链接：高亮、点击、路径。

use super::*;

/// 网址一直是链接的颜色；指针停上去加下划线、光标变手指，移开就恢复；文字光标回来
#[test]
fn links_are_colored_and_underline_on_hover() {
    let (harness, _recorder, grid, cell, session) = mounted_with(
        "go https://example.com/a now\r\n",
        crate::settings::Settings::default(),
    );
    let link_color = view::colors(crate::theme::is_dark()).link;
    let style_at = |column: usize| session.grid.cell(0, column).expect("有格子").style;
    // 没有指针时：网址是链接的颜色，没有下划线；别的字是原样
    assert_eq!(style_at(5).foreground, Some(link_color));
    assert!(!style_at(5).underline);
    assert_ne!(style_at(0).foreground, Some(link_color));
    assert_eq!(harness.cursor(), Cursor::Default);
    let (x, y) = center(grid, cell, 0, 8);
    harness.mouse_move(x, y);
    harness.frame();
    assert_eq!(harness.cursor(), Cursor::Pointer, "在链接上是手指");
    assert!(
        style_at(3).underline && style_at(23).underline,
        "整个网址都有下划线"
    );
    assert!(
        !style_at(0).underline && !style_at(28).underline,
        "旁边的字没有"
    );
    let (ox, oy) = center(grid, cell, 0, 1);
    harness.mouse_move(ox, oy);
    harness.frame();
    assert_eq!(harness.cursor(), Cursor::Text, "移开变回文字光标");
    assert!(!style_at(8).underline, "下划线撤掉");
    assert_eq!(style_at(8).foreground, Some(link_color), "颜色还在");
}

/// 文件路径平时不着色（提示符里的路径太多），指针停在本机真有的路径上才有颜色和下划线；不存在的、服务器上的没有
#[cfg(unix)]
#[test]
fn paths_light_up_only_when_openable() {
    let directory = crate::store::tests::scratch("links-hover");
    std::fs::create_dir_all(&directory).expect("建目录");
    std::fs::write(directory.join("a.txt"), "x").expect("写文件");
    // 临时目录的路径太长会折行：用一条短的符号链接指过去，屏幕上写短路径
    let short = std::path::PathBuf::from(format!("/tmp/nl{}", std::process::id()));
    let _ = std::fs::remove_file(&short);
    std::os::unix::fs::symlink(&directory, &short).expect("建链接");
    let existing = format!("{}/a.txt", short.display());
    let text = format!("{existing} /no/such/file\r\n");
    let (harness, _recorder, grid, cell, session) =
        mounted_with(&text, crate::settings::Settings::default());
    let link_color = view::colors(crate::theme::is_dark()).link;
    let style_at = |column: usize| session.grid.cell(0, column).expect("有格子").style;
    let on_path = existing.chars().count() / 2;
    let missing = existing.chars().count() + 5;
    assert_ne!(style_at(on_path).foreground, Some(link_color), "平时不着色");
    let (x, y) = center(grid, cell, 0, on_path);
    harness.mouse_move(x, y);
    harness.frame();
    assert_eq!(
        harness.cursor(),
        Cursor::Text,
        "不是本机的终端：路径不算链接"
    );
    session.set_local(true);
    let (nx, ny) = center(grid, cell, 0, on_path + 1);
    harness.mouse_move(nx, ny);
    harness.mouse_move(x, y);
    harness.frame();
    assert_eq!(harness.cursor(), Cursor::Pointer);
    assert!(style_at(on_path).underline);
    assert_eq!(style_at(on_path).foreground, Some(link_color));
    let (mx, my) = center(grid, cell, 0, missing);
    harness.mouse_move(mx, my);
    harness.frame();
    assert_eq!(harness.cursor(), Cursor::Text, "不存在的路径不是链接");
    assert!(!style_at(missing).underline);
    std::fs::remove_file(&short).ok();
    std::fs::remove_dir_all(&directory).ok();
}

/// OSC 8 超链接：显示的字不是网址也能点、也有链接色和悬停下划线，点开的是背后的地址
#[test]
fn osc_8_links_open_their_target() {
    let (harness, _recorder, grid, cell, session) = mounted_with(
        "read \x1b]8;;https://example.com/real\x07the docs\x1b]8;;\x07 now\r\n",
        crate::settings::Settings::default(),
    );
    let link_color = view::colors(crate::theme::is_dark()).link;
    let style_at = |column: usize| session.grid.cell(0, column).expect("有格子").style;
    assert_eq!(style_at(6).foreground, Some(link_color), "常显链接色");
    assert_ne!(style_at(0).foreground, Some(link_color));
    let (x, y) = center(grid, cell, 0, 8);
    harness.mouse_move(x, y);
    harness.frame();
    assert_eq!(harness.cursor(), Cursor::Pointer);
    assert!(style_at(5).underline && style_at(12).underline);
    assert!(!style_at(14).underline, "链接外面的字没有");
    harness.click_with(x, y, Modifiers::SUPER);
    assert_eq!(harness.opened_urls(), ["https://example.com/real"]);
}

/// ⌘ 加点击打开终端里的网址；不按 ⌘ 点、点在别处都不开；程序抓着鼠标时 ⌘ 点链接也归链接
#[test]
fn command_click_opens_urls() {
    let (harness, recorder, grid, cell, _session) = mounted_with(
        "see https://example.com/a. now\r\n",
        crate::settings::Settings::default(),
    );
    let (x, y) = center(grid, cell, 0, 8);
    harness.click(x, y);
    assert!(harness.opened_urls().is_empty(), "不按 ⌘ 不开");
    let (nx, ny) = center(grid, cell, 0, 1);
    harness.click_with(nx, ny, Modifiers::SUPER);
    assert!(harness.opened_urls().is_empty(), "点在别处不开");
    harness.click_with(x, y, Modifiers::SUPER);
    assert_eq!(harness.opened_urls(), ["https://example.com/a"]);
    assert!(recorder.take().is_empty(), "点链接不往 shell 里写东西");

    // 程序抓着鼠标（1000）：⌘ 点链接还是开链接，不上报
    let (harness, recorder, grid, cell, _session) = mounted_with(
        "\x1b[?1000h\x1b[?1006hsee https://example.com/b now",
        crate::settings::Settings::default(),
    );
    let (x, y) = center(grid, cell, 0, 8);
    harness.click_with(x, y, Modifiers::SUPER);
    assert_eq!(harness.opened_urls(), ["https://example.com/b"]);
    assert!(recorder.take().is_empty());
}

/// 文件路径：只有本机的终端才打得开，路径要真的存在；`~`、相对路径都能认
#[test]
fn paths_open_only_for_local_sessions_and_existing_files() {
    let directory = crate::store::tests::scratch("links");
    std::fs::create_dir_all(&directory).expect("建目录");
    let file = directory.join("note.txt");
    std::fs::write(&file, "hi").expect("写文件");
    let program = directory.join("run.command");
    std::fs::write(&program, "#!/bin/sh").expect("写文件");
    let (_harness, _recorder, _grid, _cell, session) =
        mounted_with("", crate::settings::Settings::default());
    let path = |text: &str| links::Link::Path {
        path: text.to_string(),
        line: None,
    };
    let note = file.to_string_lossy().to_string();
    assert!(
        !session.open_link(&path(&note)),
        "服务器的终端不开本机的路径"
    );
    session.set_local(true);
    assert!(session.open_link(&path(&note)));
    assert!(
        !session.open_link(&path(&format!("{note}.missing"))),
        "不存在的不开"
    );
    assert_eq!(
        session.resolve_path(&note),
        file.canonicalize().ok(),
        "绝对路径原样"
    );
    assert!(
        session
            .resolve_path("./relative-without-a-directory")
            .is_none()
    );
    // 脚本只在访达里选中，不被点一下就运行
    assert!(!links::is_plain_document(&program));
    assert!(links::is_plain_document(&file));
    std::fs::remove_dir_all(&directory).ok();
}
