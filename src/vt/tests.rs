//! 仿真层的测试：转义序列、光标、滚动、重放。

use super::*;

fn text(term: &Terminal, row: usize) -> String {
    term.screen().line_text(row)
}

#[test]
fn colored_output_keeps_the_pen_between_writes() {
    let mut term = Terminal::new(20, 4);
    term.feed(b"\x1b[31mred\x1b[0m plain");
    assert_eq!(text(&term, 0), "red plain");
    assert_eq!(
        term.screen().lines[0][0].attrs.foreground,
        Some(ColorSpec::Indexed(1))
    );
    assert_eq!(term.screen().lines[0][4].attrs.foreground, None);
}

#[test]
fn truecolor_and_256_both_reach_the_cell() {
    let mut term = Terminal::new(20, 4);
    term.feed(b"\x1b[38;2;255;0;128m\x1b[38;5;42m");
    assert_eq!(term.pen.foreground, Some(ColorSpec::Indexed(42)));
    term.feed(b"\x1b[38;2;1;2;3mx");
    assert_eq!(term.pen.foreground, Some(ColorSpec::Rgb(0x01_02_03)));
}

/// 不认识的序列必须整条吃掉：这是「跑 htop 不乱屏」的前提
#[test]
fn unknown_sequences_are_swallowed() {
    let mut term = Terminal::new(20, 4);
    term.feed(b"\x1b[>4;2my\x1b[?2026h\x1b[18~ok");
    assert_eq!(text(&term, 0), "yok");
}

#[test]
fn alternate_screen_hides_the_main_one_and_comes_back() {
    let mut term = Terminal::new(20, 4);
    term.feed(b"main\r\n\x1b[?1049halt\r\n\x1b[?1049l");
    assert!(!term.on_alternate);
    assert_eq!(text(&term, 0), "main");
}

#[test]
fn cursor_position_report_answers_the_shell() {
    let mut term = Terminal::new(20, 4);
    term.feed(b"ab\x1b[6n");
    assert_eq!(
        term.take_output(),
        b"\x1b[1;3R".to_vec(),
        "写了两个字，光标在第 3 列（1 起算）"
    );
    assert!(term.take_output().is_empty());
}

#[test]
fn erase_to_end_of_line_respects_the_cursor() {
    let mut term = Terminal::new(10, 2);
    term.feed(b"abcdef\r\x1b[3C\x1b[K");
    assert_eq!(text(&term, 0), "abc");
}

#[test]
fn scroll_region_makes_room_at_the_bottom() {
    let mut term = Terminal::new(10, 3);
    term.feed(b"one\r\ntwo\r\nthree");
    assert_eq!(text(&term, 2), "three");
    term.feed(b"\r\n!");
    assert_eq!(text(&term, 1), "three");
    assert_eq!(text(&term, 2), "!");
}

/// 不带参数的 `CSI m` 等于 `CSI 0 m`（很多脚本这么写）
#[test]
fn bare_sgr_resets_the_pen() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b[1;31m\x1b[m");
    assert_eq!(term.pen, Attributes::default());
}

/// `4:3`（弯曲下划线，neovim 用）是下划线的子参数，不能被当成「4 和 3」去打开斜体
#[test]
fn colon_subparameters_stay_with_their_parameter() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b[4:3m");
    assert!(term.pen.underline && !term.pen.italic);
    term.feed(b"\x1b[4:0m");
    assert!(!term.pen.underline);
    // 带空着的色彩空间那一格：`38:2::r:g:b`
    term.feed(b"\x1b[38:2::10:20:30m");
    assert_eq!(term.pen.foreground, Some(ColorSpec::Rgb(0x0a_14_1e)));
    term.feed(b"\x1b[48:5:200m");
    assert_eq!(term.pen.background, Some(ColorSpec::Indexed(200)));
}

#[test]
fn repeat_character_repeats_the_last_printed_one() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"x\x1b[3b");
    assert_eq!(text(&term, 0), "xxxx");
}

#[test]
fn line_drawing_charset_maps_and_switches_back() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b(0lqk\x1b(Bq");
    assert_eq!(text(&term, 0), "┌─┐q");
    // SO / SI 在 G0、G1 之间切
    term.feed(b"\r\x1b)0\x0eqx\x0fq");
    assert_eq!(text(&term, 0), "─│qq");
}

#[test]
fn osc_sets_the_title_and_strips_control_characters() {
    let mut term = Terminal::new(20, 2);
    term.feed("\x1b]0;我的\x01终端\x07".as_bytes());
    assert_eq!(term.title, "我的终端");
    term.feed(b"\x1b]2;second\x1b\\");
    assert_eq!(term.title, "second");
}

/// OSC 8：夹在开头和结尾之间的字挂着链接；同一个地址同一个编号；别的协议和控制字符当没有
#[test]
fn osc_8_attaches_links_to_cells() {
    let mut term = Terminal::new(40, 2);
    term.feed(b"a\x1b]8;;https://example.com/x\x07bc\x1b]8;;\x07d");
    let line = &term.screen().lines[0];
    assert_eq!(
        line.iter()
            .take(4)
            .map(|cell| cell.attrs.link)
            .collect::<Vec<_>>(),
        [0, 1, 1, 0]
    );
    assert_eq!(term.link_uris(), ["https://example.com/x"]);
    // SGR 0 不会把链接关掉，它们互不相干
    term.feed(b"\x1b]8;id=7;https://example.com/x\x1b\\e\x1b[0mf\x1b]8;;\x1b\\g");
    let line = &term.screen().lines[0];
    assert_eq!(line[4].attrs.link, 1, "同一个地址同一个编号");
    assert_eq!(line[5].attrs.link, 1);
    assert_eq!(line[6].attrs.link, 0);
    assert_eq!(term.link_uris().len(), 1);
    for bad in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://a.b/\u{1}x",
    ] {
        term.feed(format!("\x1b]8;;{bad}\x07z\x1b]8;;\x07").as_bytes());
    }
    assert_eq!(term.link_uris().len(), 1, "不安全的协议不收");
    term.feed(b"\x1b]8;;mailto:me@example.com\x07m");
    assert_eq!(term.link_uris().len(), 2);
}

#[test]
fn osc_9_and_777_become_alerts() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b]9;build done\x07");
    term.feed("\x1b]777;notify;标题;正\x01文\x1b\\".as_bytes());
    term.feed(b"\x1b]9;4;1;50\x07");
    term.feed(b"\x1b]9;\x07");
    assert_eq!(
        term.take_alerts(),
        vec![
            (String::new(), "build done".to_string()),
            ("标题".to_string(), "正文".to_string())
        ]
    );
    assert!(term.take_alerts().is_empty(), "取走就没了");
    for _ in 0..20 {
        term.feed(b"\x1b]9;spam\x07");
    }
    assert_eq!(term.take_alerts().len(), MAX_ALERTS);
}

/// shell 报的目录：（主机，路径）
fn reported(term: &Terminal) -> Option<(String, std::path::PathBuf)> {
    term.reported_directory()
        .map(|directory| (directory.host.clone(), directory.path.clone()))
}

/// 这个平台上的写法：Windows 上 `C:` 开头、反斜杠，别的平台去掉盘符、用斜杠
fn native(windows: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        windows.into()
    } else {
        windows[2..].replace('\\', "/").into()
    }
}

/// OSC 7：`file://主机/路径`，按百分号解码成本机写法的绝对路径（Windows 上 `/C:/x` 是 `C:\x`），结尾的分隔符去掉
#[test]
fn osc_7_reports_the_working_directory() {
    let mut term = Terminal::new(40, 4);
    assert_eq!(term.reported_directory(), None);
    let prefix = if cfg!(windows) { "/C:" } else { "" };
    term.feed(format!("\x1b]7;file://box{prefix}/Users/me/%E4%B8%AD%20x/\x1b\\").as_bytes());
    assert_eq!(
        reported(&term),
        Some(("box".to_string(), native(r"C:\Users\me\中 x")))
    );
    // 没写主机、协议大写、没编码的 `%`（有的 shell 不编码）
    term.feed(format!("\x1b]7;FILE://{prefix}/tmp/100%done\x07").as_bytes());
    assert_eq!(
        reported(&term),
        Some((String::new(), native(r"C:\tmp\100%done")))
    );
    // 根目录
    term.feed(format!("\x1b]7;file://box{prefix}/\x07").as_bytes());
    assert_eq!(reported(&term).map(|(_, path)| path), Some(native(r"C:\")));
}

/// 不收的报告：不是 file 协议、没有路径、解出来带控制字符或者不是 UTF-8；Windows 上没有盘符的。收不下就留着上一个
#[test]
fn bad_directory_reports_are_ignored() {
    let mut term = Terminal::new(40, 4);
    let prefix = if cfg!(windows) { "/C:" } else { "" };
    term.feed(format!("\x1b]7;file://box{prefix}/good\x07").as_bytes());
    let good = reported(&term);
    assert_eq!(
        good.as_ref().map(|(_, path)| path.clone()),
        Some(native(r"C:\good"))
    );
    let mut bad = vec![
        "http://box/x".to_string(),
        "file://box".to_string(),
        format!("file://box{prefix}/a%0Ab"),
        format!("file://box{prefix}/%FF"),
    ];
    if cfg!(windows) {
        // 服务器上 Unix 的路径、盘符后面不是分隔符
        bad.extend([
            "file://box/home/me".to_string(),
            "file://box/C:x".to_string(),
        ]);
    }
    for text in bad {
        term.feed(format!("\x1b]7;{text}\x07").as_bytes());
        assert_eq!(reported(&term), good, "{text:?}");
    }
}

/// Windows 上没有盘符的 `/home/me`（WSL 里的 shell 报的）另外记下，不当本机的目录、不顶掉本机的；
/// 同样每个提示符开始时清掉。别的平台上它就是本机的目录
#[test]
fn linux_directory_reports_are_kept_apart_on_windows() {
    let mut term = Terminal::new(40, 4);
    term.feed(b"\x1b]7;file://box/home/me/%E4%B8%AD/\x07");
    let linux = || {
        term.reported_linux_directory()
            .map(|directory| (directory.host.clone(), directory.path.clone()))
    };
    if cfg!(windows) {
        assert_eq!(
            linux(),
            Some(("box".to_string(), "/home/me/中".to_string()))
        );
        assert_eq!(term.reported_directory(), None);
        term.feed(b"\x1b]7;file://box/C:/x\x07\x1b]7;file://box/\x07");
        assert_eq!(reported(&term), Some(("box".to_string(), r"C:\x".into())));
        assert_eq!(
            term.reported_linux_directory()
                .map(|directory| directory.path.as_str()),
            Some("/"),
            "根目录"
        );
        term.feed(b"\x1b]133;A\x07");
        assert_eq!(term.reported_linux_directory(), None, "新的提示符，还没报");
    } else {
        assert_eq!(linux(), None);
        assert_eq!(
            reported(&term),
            Some(("box".to_string(), "/home/me/中".into()))
        );
    }
}

/// 每个提示符开始（OSC 133;A）时清掉，等这个提示符重新报；副屏里的不理
#[test]
fn directory_reports_last_until_the_next_prompt() {
    let mut term = Terminal::new(40, 4);
    let prefix = if cfg!(windows) { "/C:" } else { "" };
    term.feed(format!("\x1b]7;file://box{prefix}/old\x07").as_bytes());
    term.feed(b"\x1b]133;A\x07");
    assert_eq!(term.reported_directory(), None, "新的提示符，还没报");
    term.feed(format!("\x1b]7;file://box{prefix}/new\x07$ \x1b]133;B\x07").as_bytes());
    assert_eq!(
        reported(&term).map(|(_, path)| path),
        Some(native(r"C:\new"))
    );
    term.feed(format!("\x1b[?1049h\x1b]7;file://box{prefix}/vim\x07\x1b[?1049l").as_bytes());
    assert_eq!(
        reported(&term).map(|(_, path)| path),
        Some(native(r"C:\new"))
    );
}

/// ConEmu 的 OSC 9;9 是当前目录（引号可有可无），不弹通知；Windows 上网络路径不收
#[test]
fn osc_9_9_reports_the_directory_instead_of_an_alert() {
    let mut term = Terminal::new(40, 4);
    let windows = native(r"C:\Windows");
    term.feed(format!("\x1b]9;9;\"{}\"\x07", windows.display()).as_bytes());
    assert!(term.take_alerts().is_empty(), "不是通知");
    assert_eq!(reported(&term), Some((String::new(), windows.clone())));
    if cfg!(windows) {
        term.feed(b"\x1b]9;9;C:/Users/me/\x1b\\");
        assert_eq!(
            reported(&term).map(|(_, path)| path),
            Some(r"C:\Users\me".into())
        );
        term.feed(b"\x1b]9;9;\\\\server\\share\x07");
        term.feed(b"\x1b]9;9;relative\x07");
        assert_eq!(
            reported(&term).map(|(_, path)| path),
            Some(r"C:\Users\me".into()),
            "网络路径、相对路径不收"
        );
    }
    assert!(term.take_alerts().is_empty());
}

#[test]
fn tab_stops_can_be_set_and_cleared() {
    let mut term = Terminal::new(40, 2);
    term.feed(b"\x1b[3g\x1b[1;5H\x1bH\r\ta");
    assert_eq!(text(&term, 0), "    a");
}

/// 软复位把没收拾干净的状态救回来：隐藏的光标、卷起的滚动区域
#[test]
fn soft_reset_restores_modes_and_region() {
    let mut term = Terminal::new(20, 6);
    term.feed(b"\x1b[?25l\x1b[2;4r\x1b[1m\x1b[!p");
    assert!(term.modes.cursor_visible);
    assert_eq!(term.pen, Attributes::default());
    assert_eq!(
        (term.screen().region_top, term.screen().region_bottom),
        (0, 6)
    );
}

/// 按网格的规矩回放改动（先挪、后画），结果必须和屏幕一模一样。
/// 这是「先写再滚、区域滚动、插入删除行」这些组合下不乱屏的总开关
#[test]
fn replaying_damage_reproduces_the_screen() {
    struct Mirror(Vec<String>);
    impl Mirror {
        fn apply(&mut self, term: &mut Terminal) {
            let damage = term.damage();
            let rows = self.0.len();
            let n = damage.scrolled_out.min(rows);
            self.0.rotate_left(n);
            for row in rows - n..rows {
                self.0[row].clear();
            }
            for row in damage.rows {
                self.0[row] = term.screen().line_text(row);
            }
        }
    }
    const PIECES: &[&str] = &[
        "hello",
        "世界",
        "x",
        "\r\n",
        "\r\n",
        "\r\n",
        "\n",
        "\r",
        "\x1b[2;4r",
        "\x1b[r",
        "\x1b[L",
        "\x1b[2M",
        "\x1b[3A",
        "\x1b[2B",
        "\x1b[H",
        "\x1b[2J",
        "\x1b[K",
        "\x1b[1K",
        "\x1bM",
        "\x1b[S",
        "\x1b[2T",
        "\x1b[3@",
        "\x1b[2P",
        "\x1b[4X",
        "\x1b[5;3H",
        "\t",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..200 {
        let mut term = Terminal::new(12, 6);
        let mut mirror = Mirror(vec![String::new(); 6]);
        mirror.apply(&mut term);
        for step in 0..30 {
            let mut batch = String::new();
            for _ in 0..(next() % 8 + 1) {
                batch.push_str(PIECES[(next() % PIECES.len() as u64) as usize]);
            }
            term.feed(batch.as_bytes());
            mirror.apply(&mut term);
            let actual: Vec<String> = (0..6).map(|row| term.screen().line_text(row)).collect();
            assert_eq!(
                mirror.0, actual,
                "第 {round} 轮第 {step} 批之后对不上，这一批是 {batch:?}"
            );
        }
    }
}

/// 光标定位的行、列别拿反：宽屏上列号能到屏宽，不是屏高
#[test]
fn cursor_position_uses_columns_for_the_column() {
    let mut term = Terminal::new(100, 10);
    term.feed(b"\x1b[3;80H");
    assert_eq!(term.screen().cursor, Point::new(2, 79));
    // 越界截到屏内
    term.feed(b"\x1b[99;999H");
    assert_eq!(term.screen().cursor, Point::new(9, 99));
}

#[test]
fn mouse_modes_switch_on_and_off_independently() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(term.modes.mouse, MouseMode::Press);
    assert!(term.modes.mouse_sgr);
    term.feed(b"\x1b[?1003h");
    assert_eq!(term.modes.mouse, MouseMode::Motion);
    // 关的是 1000，不是当前的 1003：不动
    term.feed(b"\x1b[?1000l");
    assert_eq!(term.modes.mouse, MouseMode::Motion);
    term.feed(b"\x1b[?1003l");
    assert_eq!(term.modes.mouse, MouseMode::Off);
    term.feed(b"\x1b[?1004h");
    assert!(term.modes.focus_reporting);
    // 复位回到默认
    term.feed(b"\x1b[?1002h\x1b[!p");
    assert_eq!(term.modes.mouse, MouseMode::Off);
}

/// 内容写成文字再喂给另一台终端，历史、屏幕、颜色、宽字都原样回来；副屏里的东西不存
#[test]
fn a_transcript_replays_into_an_identical_screen() {
    let mut term = Terminal::new(20, 3);
    term.feed(
        "one\r\n\x1b[1;31mtwo\x1b[0m\r\n中文 \x1b[48;2;1;2;3mbg\x1b[0m\r\nfour\r\n\x1b[38;5;200mfive\x1b[0m".as_bytes(),
    );
    assert_eq!(term.scrollback_len(), 2, "有两行滚进了历史");
    let saved = term.transcript(100);
    let mut other = Terminal::new(20, 3);
    other.feed(saved.as_bytes());
    assert_eq!(other.scrollback_len(), 2);
    assert_eq!(other.screen().line_text(0), "中文 bg");
    assert_eq!(other.transcript(100), saved, "再存一次一样");
    let two = other.screen().history_line(0).expect("历史里有");
    assert_eq!(
        two[0].attrs,
        term.screen().history_line(0).expect("有")[0].attrs
    );
    let bg = &other.screen().lines[0][5];
    assert_eq!(bg.attrs.background, Some(ColorSpec::Rgb(0x010203)));
    assert_eq!(
        other.screen().lines[2][0].attrs.foreground,
        Some(ColorSpec::Indexed(200))
    );
    // 只留最后几行；空行、行尾空格去掉；副屏不存
    assert_eq!(term.transcript(2).matches("\r\n").count(), 1);
    assert_eq!(Terminal::new(10, 3).transcript(10), "");
    term.feed(b"\x1b[?1049h\x1b[Hvim here");
    assert!(!term.transcript(100).contains("vim here"));
}

#[test]
fn narrowing_wraps_a_long_line_and_widening_joins_it_back() {
    let mut term = Terminal::new(20, 6);
    term.feed(b"$ npm run build-all");
    term.resize(10, 6);
    assert_eq!(text(&term, 0), "$ npm run");
    assert_eq!(text(&term, 1), "build-all");
    assert_eq!(term.screen().cursor, Point::new(1, 9));
    term.resize(30, 6);
    assert_eq!(text(&term, 0), "$ npm run build-all");
    assert_eq!(text(&term, 1), "");
    assert_eq!(term.screen().cursor, Point::new(0, 19));
}

#[test]
fn reflow_does_not_join_lines_that_ended_with_a_newline() {
    let mut term = Terminal::new(10, 6);
    term.feed(b"abc\r\ndef");
    term.resize(20, 6);
    assert_eq!(text(&term, 0), "abc");
    assert_eq!(text(&term, 1), "def");
}

#[test]
fn reflow_carries_history_along() {
    let mut term = Terminal::new(10, 3);
    term.feed(b"0123456789abc\r\nx\r\ny\r\nz");
    term.resize(20, 3);
    let screen = term.screen();
    let mut all: Vec<String> = (0..screen.history_len())
        .rev()
        .map(|i| {
            screen
                .history_line(i)
                .unwrap()
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    all.extend((0..3).map(|row| screen.line_text(row)));
    assert_eq!(all, ["0123456789abc", "x", "y", "z"]);
}

#[test]
fn dim_attribute_is_kept_on_cells() {
    let mut term = Terminal::new(20, 2);
    term.feed(b"\x1b[2mgrey\x1b[0m");
    assert!(term.screen().lines[0][0].attrs.dim);
}
