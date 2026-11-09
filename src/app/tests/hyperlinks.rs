//! 链接：高亮、点击、路径；拖进本机终端的文件路径怎么写。

use super::*;

/// 点链接要按的键：macOS 是 ⌘，Windows、Linux 是 Ctrl（见 `open_link_at`）
fn open_modifier() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::SUPER
    } else {
        Modifiers::CONTROL
    }
}

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
    harness.click_with(x, y, open_modifier());
    assert_eq!(harness.opened_urls(), ["https://example.com/real"]);
}

/// ⌘（Windows、Linux 上是 Ctrl）加点击打开终端里的网址；不按 ⌘ 点、点在别处都不开；
/// 程序抓着鼠标时 ⌘ 点链接也归链接
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
    harness.click_with(nx, ny, open_modifier());
    assert!(harness.opened_urls().is_empty(), "点在别处不开");
    harness.click_with(x, y, open_modifier());
    assert_eq!(harness.opened_urls(), ["https://example.com/a"]);
    assert!(recorder.take().is_empty(), "点链接不往 shell 里写东西");

    // 程序抓着鼠标（1000）：⌘ 点链接还是开链接，不上报
    let (harness, recorder, grid, cell, _session) = mounted_with(
        "\x1b[?1000h\x1b[?1006hsee https://example.com/b now",
        crate::settings::Settings::default(),
    );
    let (x, y) = center(grid, cell, 0, 8);
    harness.click_with(x, y, open_modifier());
    assert_eq!(harness.opened_urls(), ["https://example.com/b"]);
    assert!(recorder.take().is_empty());
}

/// Windows 的写法（盘符开头的、`.\` 开头的）在本机的终端里也会亮；`.\` 从 shell 现在的目录算起；不存在的不亮
#[cfg(windows)]
#[test]
fn windows_paths_light_up_only_when_openable() {
    let directory = crate::store::tests::scratch("links-hover-windows");
    std::fs::create_dir_all(&directory).expect("建目录");
    std::fs::write(directory.join("a.txt"), "x").expect("写文件");
    // 临时目录的路径太长会折行：盘符开头的用系统目录（短，而且一定在）
    let system = std::env::var("SystemRoot").expect("有系统目录");
    let text = format!("{system} .\\a.txt C:\\no\\such\\file\r\n");
    let (harness, recorder, grid, cell, session) =
        mounted_with(&text, crate::settings::Settings::default());
    session.set_local(true);
    recorder.set_directory(&directory);
    let cursor_at = |column: usize| {
        let (x, y) = center(grid, cell, 0, column);
        harness.mouse_move(x, y);
        harness.frame();
        harness.cursor()
    };
    assert_eq!(cursor_at(2), Cursor::Pointer, "盘符开头的");
    let relative = system.chars().count() + 3;
    assert_eq!(cursor_at(relative), Cursor::Pointer, "从 shell 的目录算起");
    assert_eq!(
        cursor_at(relative + 9),
        Cursor::Text,
        "不存在的路径不是链接"
    );
    std::fs::remove_dir_all(&directory).ok();
}

/// 文件路径：只有本机的终端才打得开，路径要真的存在；`~`、相对路径都能认（Windows 上还有 `~\`、`.\`）；
/// 找到的路径交给外壳、给人看之前不带 Windows 的 `\\?\` 前缀
#[test]
fn paths_open_only_for_local_sessions_and_existing_files() {
    let directory = crate::store::tests::scratch("links");
    std::fs::create_dir_all(&directory).expect("建目录");
    let file = directory.join("note.txt");
    std::fs::write(&file, "hi").expect("写文件");
    let program = directory.join("run.command");
    std::fs::write(&program, "#!/bin/sh").expect("写文件");
    let (_harness, recorder, _grid, _cell, session) =
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
    let expected = file.canonicalize().ok().map(links::without_verbatim);
    assert_eq!(session.resolve_path(&note), expected, "绝对路径原样");
    assert!(
        !expected
            .as_deref()
            .expect("文件在")
            .to_string_lossy()
            .starts_with(r"\\?\"),
        "{expected:?}"
    );
    assert!(
        session
            .resolve_path("./relative-without-a-directory")
            .is_none()
    );
    // 相对路径从 shell 现在的目录算起
    recorder.set_directory(&directory);
    assert_eq!(session.resolve_path("./note.txt"), expected);
    if cfg!(windows) {
        assert_eq!(session.resolve_path(r".\note.txt"), expected);
        let home = crate::store::user_home()
            .and_then(|home| home.canonicalize().ok())
            .map(links::without_verbatim);
        assert_eq!(session.resolve_path(r"~\"), home, "~\\ 是主目录");
    }
    // 脚本只在访达里选中，不被点一下就运行
    assert!(!links::is_plain_document(&program));
    assert!(links::is_plain_document(&file));
    std::fs::remove_dir_all(&directory).ok();
}

/// Windows 上拖进本机终端的文件按 shell 的规矩写：一般的路径（空格、括号）cmd、PowerShell 都是双引号；
/// 带 `$` 的，设置里指定了 PowerShell 就用单引号，cmd 用双引号；服务器的终端不写
#[cfg(windows)]
#[test]
fn dropped_files_are_quoted_for_the_windows_shell() {
    let settings = crate::settings::Settings {
        shell: crate::settings::ShellKind::Program,
        shell_program: r"C:\Program Files\PowerShell\7\pwsh.exe".into(),
        ..Default::default()
    };
    let (_harness, recorder, _grid, _cell, session) = mounted_with("", settings);
    let files = [
        std::path::PathBuf::from(r"C:\Users\me\a.txt"),
        std::path::PathBuf::from(r"C:\Program Files (x86)\b c.txt"),
        std::path::PathBuf::from(r"C:\$Recycle\it's.txt"),
    ];
    assert!(!session.drop_files(&files), "服务器的终端不写");
    assert_eq!(recorder.take(), "");
    session.set_local(true);
    assert!(session.drop_files(&files));
    assert_eq!(
        recorder.take(),
        r#"C:\Users\me\a.txt "C:\Program Files (x86)\b c.txt" 'C:\$Recycle\it''s.txt' "#
    );
    session
        .form
        .shell_program
        .set(r"C:\WINDOWS\system32\cmd.exe".into());
    assert!(session.drop_files(&files[2..]));
    assert_eq!(recorder.take(), r#""C:\$Recycle\it's.txt" "#);
}
