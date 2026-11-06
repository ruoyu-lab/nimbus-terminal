//! 鼠标上报：把「在第几行第几列按下了哪个键」编码成程序（vim、htop、tmux、mc）读得懂的字节。
//!
//! 程序用 DECSET 告诉终端要不要鼠标、要多详细：
//!
//! | 模式 | 报什么 |
//! |---|---|
//! | `?1000` | 按下、松开、滚轮 |
//! | `?1002` | 再加上按着键时的移动（拖动） |
//! | `?1003` | 再加上没按键时的移动 |
//! | `?1006` | 换成 SGR 写法 |
//!
//! 默认写法（X10 的老格式）把坐标塞进一个字节，最多到第 223 列、第 223 行，松开时不说是哪个键；
//! SGR 写法没有这两个毛病，现在的程序基本都会要它。这一层是纯函数，不认识窗口也不认识屏幕。

/// 要上报到什么程度
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMode {
    /// 不上报：鼠标归终端自己用（选择、复制）
    #[default]
    Off,
    /// `?1000`
    Press,
    /// `?1002`
    Drag,
    /// `?1003`
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press,
    Release,
    Motion,
}

/// 一次要上报的鼠标动作。行列从 0 起算
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// 按着的键；没按键的移动是 `None`
    pub button: Option<Button>,
    pub action: Action,
    pub column: usize,
    pub row: usize,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// 这种动作在这个模式下要不要报
pub fn wants(mode: MouseMode, action: Action, button: Option<Button>) -> bool {
    match (mode, action) {
        (MouseMode::Off, _) => false,
        (_, Action::Press | Action::Release) => true,
        (MouseMode::Press, Action::Motion) => false,
        (MouseMode::Drag, Action::Motion) => button.is_some(),
        (MouseMode::Motion, Action::Motion) => true,
    }
}

/// 编码。`sgr` 是 `?1006` 开没开。模式不要这个动作、或者坐标在老格式里放不下时是 `None`
pub fn encode(report: &Report, mode: MouseMode, sgr: bool) -> Option<Vec<u8>> {
    if !wants(mode, report.action, report.button) {
        return None;
    }
    let is_wheel = matches!(report.button, Some(Button::WheelUp | Button::WheelDown));
    // 滚轮只有「按下」，没有松开
    if is_wheel && report.action != Action::Press {
        return None;
    }
    let mut code: u32 = match report.button {
        Some(Button::Left) => 0,
        Some(Button::Middle) => 1,
        Some(Button::Right) => 2,
        Some(Button::WheelUp) => 64,
        Some(Button::WheelDown) => 65,
        // 没按键的移动：老格式里「松开」也是 3，靠下面的 +32 区分
        None => 3,
    };
    // 老格式松开时不说是哪个键，一律 3
    if report.action == Action::Release && !sgr {
        code = 3;
    }
    code +=
        4 * u32::from(report.shift) + 8 * u32::from(report.alt) + 16 * u32::from(report.control);
    if report.action == Action::Motion {
        code += 32;
    }
    let (column, row) = (report.column + 1, report.row + 1);
    if sgr {
        let end = if report.action == Action::Release {
            'm'
        } else {
            'M'
        };
        Some(format!("\x1b[<{code};{column};{row}{end}").into_bytes())
    } else {
        // 老格式：每个数加 32 变成可见字符放进一个字节，超过 255 放不下
        let byte = |value: usize| u8::try_from(value + 32).ok();
        Some(vec![
            0x1b,
            b'[',
            b'M',
            byte(code as usize)?,
            byte(column)?,
            byte(row)?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(button: Option<Button>, action: Action, column: usize, row: usize) -> Report {
        Report {
            button,
            action,
            column,
            row,
            shift: false,
            alt: false,
            control: false,
        }
    }

    #[test]
    fn sgr_press_and_release_keep_the_button() {
        let press = report(Some(Button::Left), Action::Press, 9, 4);
        assert_eq!(
            encode(&press, MouseMode::Press, true),
            Some(b"\x1b[<0;10;5M".to_vec())
        );
        let release = report(Some(Button::Left), Action::Release, 9, 4);
        assert_eq!(
            encode(&release, MouseMode::Press, true),
            Some(b"\x1b[<0;10;5m".to_vec())
        );
    }

    /// 老格式：松开不说是哪个键，坐标和键码都加 32
    #[test]
    fn legacy_release_has_no_button() {
        let press = report(Some(Button::Right), Action::Press, 0, 0);
        assert_eq!(
            encode(&press, MouseMode::Press, false),
            Some(vec![0x1b, b'[', b'M', 32 + 2, 33, 33])
        );
        let release = report(Some(Button::Right), Action::Release, 0, 0);
        assert_eq!(
            encode(&release, MouseMode::Press, false),
            Some(vec![0x1b, b'[', b'M', 32 + 3, 33, 33])
        );
    }

    #[test]
    fn legacy_format_cannot_reach_far_columns() {
        let far = report(Some(Button::Left), Action::Press, 300, 0);
        assert_eq!(encode(&far, MouseMode::Press, false), None);
        assert!(
            encode(&far, MouseMode::Press, true).is_some(),
            "SGR 没有这个限制"
        );
    }

    #[test]
    fn modifiers_and_motion_add_to_the_code() {
        let mut drag = report(Some(Button::Left), Action::Motion, 1, 1);
        drag.shift = true;
        drag.control = true;
        // 0 + 4（Shift）+ 16（Ctrl）+ 32（移动）
        assert_eq!(
            encode(&drag, MouseMode::Drag, true),
            Some(b"\x1b[<52;2;2M".to_vec())
        );
    }

    #[test]
    fn wheel_is_press_only() {
        let up = report(Some(Button::WheelUp), Action::Press, 0, 0);
        assert_eq!(
            encode(&up, MouseMode::Press, true),
            Some(b"\x1b[<64;1;1M".to_vec())
        );
        let down = report(Some(Button::WheelDown), Action::Press, 0, 0);
        assert_eq!(
            encode(&down, MouseMode::Press, true),
            Some(b"\x1b[<65;1;1M".to_vec())
        );
        assert_eq!(
            encode(
                &report(Some(Button::WheelUp), Action::Release, 0, 0),
                MouseMode::Press,
                true
            ),
            None
        );
    }

    /// 各个模式要不要报移动：1000 不报，1002 只报按着键的，1003 全报
    #[test]
    fn modes_filter_motion() {
        let held = report(Some(Button::Left), Action::Motion, 0, 0);
        let free = report(None, Action::Motion, 0, 0);
        assert!(encode(&held, MouseMode::Press, true).is_none());
        assert!(encode(&held, MouseMode::Drag, true).is_some());
        assert!(encode(&free, MouseMode::Drag, true).is_none());
        assert_eq!(
            encode(&free, MouseMode::Motion, true),
            Some(b"\x1b[<35;1;1M".to_vec())
        );
        assert!(encode(&held, MouseMode::Off, true).is_none());
    }
}
