//! 按键 → 要写给 shell 的字节。
//!
//! 键盘上的一个键在终端里就是几个字节：`Ctrl-C` 是 `0x03`，方向键是 `\x1b[A`（应用光标模式里是
//! `\x1bOA`），按住了修饰键再加参数（`\x1b[1;5C` 是 Ctrl+→）。按 Command 的组合键不属于终端，
//! 返回 `None` 让上层（复制、粘贴、新建标签）去处理。

use aureole::prelude::*;

/// 编码要看的那几个模式（从 [`super::Modes`] 抄一份，省得这一层依赖整个终端）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// DECCKM：方向键发 `\x1bOA` 这套
    pub application_cursor: bool,
}

/// 这个按键写出来的字节；`None` 是「不归终端管」
pub fn encode(event: &KeyEvent, keymap: Keymap) -> Option<Vec<u8>> {
    // Command（macOS）/ Windows 键 / Super：留给应用和菜单栏
    if event.modifiers.has_super() {
        return None;
    }
    let modifiers = event.modifiers;
    let alt = modifiers.has_alt();
    let ctrl = modifiers.has_control();
    let shift = modifiers.has_shift();
    match &event.key {
        Key::Named(named) => named_key(*named, keymap, modifiers, ctrl, shift),
        Key::Character(text) => {
            if ctrl
                && !alt
                && let Some(byte) = control_code(text.as_str())
            {
                return Some(vec![byte]);
            }
            // 组字过程中（输入法正选字）按键归输入法，不发任何东西
            if event.composing {
                return None;
            }
            if alt {
                // Option 当 Meta：ESC 前缀 + 不算修饰键时的那个字符
                let base = ascii_of(&event.base)?;
                return Some(vec![0x1b, base]);
            }
            let text = event.typed_text()?;
            Some(text.as_bytes().to_vec())
        }
        _ => None,
    }
}

/// `Control` 加一个键的 ASCII：`Ctrl-A` 是 1，`Ctrl-[` 是 27。认不出来时 `None`
fn control_code(text: &str) -> Option<u8> {
    let ch = text.chars().next()?;
    let code = match ch {
        'a'..='z' => ch as u8 - b'a' + 1,
        'A'..='Z' => ch as u8 - b'A' + 1,
        // Ctrl+2 在多数终端里等价于 Ctrl+Space（NUL），Ctrl+3..7 跟着标点走
        '2' | ' ' => 0,
        '@' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' | '/' => 0x1f,
        // xterm 的 Ctrl+8、Ctrl+9、Ctrl+? 都是 DEL
        '8' | '9' | '?' => 0x7f,
        _ => return None,
    };
    Some(code)
}

fn named_key(named: NamedKey, keymap: Keymap, modifiers: Modifiers, ctrl: bool, shift: bool) -> Option<Vec<u8>> {
    // 方向键和这几个「有转义码形状」的键：没有修饰键时是短形式，有了就带参数
    let modified = |code: &str| csi(code, modifiers);
    let cursor = |last: u8| {
        if modifiers.is_empty() {
            let prefix: &[u8] = if keymap.application_cursor { b"\x1bO" } else { b"\x1b[" };
            let mut bytes = prefix.to_vec();
            bytes.push(last);
            bytes
        } else {
            modified(&(last as char).to_string())
        }
    };
    let bytes = match named {
        NamedKey::Enter => b"\r".to_vec(),
        NamedKey::Tab if shift => b"\x1b[Z".to_vec(),
        NamedKey::Tab => b"\t".to_vec(),
        NamedKey::Backspace if ctrl => vec![0x08],
        NamedKey::Backspace => vec![0x7f],
        NamedKey::Escape => vec![0x1b],
        NamedKey::Space if ctrl => vec![0],
        NamedKey::Space => b" ".to_vec(),
        NamedKey::ArrowUp => cursor(b'A'),
        NamedKey::ArrowDown => cursor(b'B'),
        NamedKey::ArrowRight => cursor(b'C'),
        NamedKey::ArrowLeft => cursor(b'D'),
        NamedKey::Home => {
            if modifiers.is_empty() && keymap.application_cursor {
                b"\x1bOH".to_vec()
            } else {
                modified("H")
            }
        }
        NamedKey::End => {
            if modifiers.is_empty() && keymap.application_cursor {
                b"\x1bOF".to_vec()
            } else {
                modified("F")
            }
        }
        NamedKey::Insert => tilde(2, modifiers),
        NamedKey::Delete => tilde(3, modifiers),
        NamedKey::PageUp => tilde(5, modifiers),
        NamedKey::PageDown => tilde(6, modifiers),
        NamedKey::F1 => function_key(b'P', modifiers),
        NamedKey::F2 => function_key(b'Q', modifiers),
        NamedKey::F3 => function_key(b'R', modifiers),
        NamedKey::F4 => function_key(b'S', modifiers),
        NamedKey::F5 => tilde(15, modifiers),
        NamedKey::F6 => tilde(17, modifiers),
        NamedKey::F7 => tilde(18, modifiers),
        NamedKey::F8 => tilde(19, modifiers),
        NamedKey::F9 => tilde(20, modifiers),
        NamedKey::F10 => tilde(21, modifiers),
        NamedKey::F11 => tilde(23, modifiers),
        NamedKey::F12 => tilde(24, modifiers),
        // 数字键盘、媒体键这些：不归终端管
        _ => return None,
    };
    // Option / Alt 加回车、退格、Tab、Esc：照 Meta 的习惯在前面加一个 ESC（Alt+退格是按词删除）
    let meta = modifiers.has_alt() && matches!(named, NamedKey::Enter | NamedKey::Backspace | NamedKey::Tab | NamedKey::Escape);
    if meta {
        let mut prefixed = vec![0x1b];
        prefixed.extend(bytes);
        return Some(prefixed);
    }
    Some(bytes)
}

/// xterm 的修饰键参数：1 起算，Shift +1、Alt +2、Ctrl +4
fn modifier_code(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.has_shift()) + 2 * u8::from(modifiers.has_alt()) + 4 * u8::from(modifiers.has_control())
}

/// `\x1b[1;{修饰键}final`
fn csi(final_char: &str, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.is_empty() {
        let mut bytes = b"\x1b[".to_vec();
        bytes.extend_from_slice(final_char.as_bytes());
        bytes
    } else {
        format!("\x1b[1;{}{}", modifier_code(modifiers), final_char).into_bytes()
    }
}

/// `\x1b[{n}~`（Delete、PgUp、功能键的那一类）
fn tilde(code: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.is_empty() {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{}~", modifier_code(modifiers)).into_bytes()
    }
}

/// F1..F4 是 SS3（`\x1bOP`），带修饰键走 `\x1b[1;n{P}`
fn function_key(final_char: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.is_empty() {
        vec![0x1b, b'O', final_char]
    } else {
        modified_final(final_char, modifiers)
    }
}

fn modified_final(final_char: u8, modifiers: Modifiers) -> Vec<u8> {
    format!("\x1b[1;{}{}", modifier_code(modifiers), final_char as char).into_bytes()
}

fn ascii_of(key: &Key) -> Option<u8> {
    let Key::Character(text) = key else { return None };
    let ch = text.chars().next()?;
    ch.is_ascii().then_some(ch as u8)
}

/// 粘进去的文字：开了括号粘贴时前后加标记。
///
/// - 控制字符（除了制表符和换行）必须去掉：否则剪贴板里带一段 `\x1b[201~` 就能自己结束粘贴、
///   往 shell 里塞命令，带一个 `\x03` 就等于替用户按了 Ctrl-C
/// - 没开括号粘贴时，换行要换成回车（`\r`，也就是按回车键发的那个字节），`\r\n` 只算一个；
///   开了的话原样交给 shell，由它决定换行怎么处理
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    let mut previous_was_cr = false;
    for ch in text.chars() {
        let is_cr = ch == '\r';
        match ch {
            '\t' => bytes.push(b'\t'),
            '\n' | '\r' if bracketed => bytes.push(if is_cr { b'\r' } else { b'\n' }),
            '\r' => bytes.push(b'\r'),
            // `\r\n` 的 `\n` 已经随 `\r` 算过了
            '\n' if previous_was_cr => {}
            '\n' => bytes.push(b'\r'),
            ch if ch.is_control() => {}
            _ => bytes.extend_from_slice(ch.encode_utf8(&mut [0u8; 4]).as_bytes()),
        }
        previous_was_cr = is_cr;
    }
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_code_matches_xterm() {
        assert_eq!(modifier_code(Modifiers::empty()), 1);
        assert_eq!(modifier_code(Modifiers::SHIFT), 2);
        assert_eq!(modifier_code(Modifiers::ALT), 3);
        assert_eq!(modifier_code(Modifiers::CONTROL), 5);
        assert_eq!(modifier_code(Modifiers::CONTROL | Modifiers::SHIFT), 6);
    }

    #[test]
    fn control_letters_and_punctuation() {
        assert_eq!(control_code("c"), Some(0x03));
        assert_eq!(control_code("["), Some(0x1b));
        assert_eq!(control_code(" "), Some(0));
        assert_eq!(control_code("中"), None);
    }

    #[test]
    fn paste_never_carries_escape_bytes() {
        let bytes = paste_bytes("a\x1b[201~rm -rf /", true);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("\x1b[200~") && text.ends_with("\x1b[201~"));
        // 中间那段伪造的结束标记被打散了
        assert!(!text[6..text.len() - 6].contains('\x1b'));
    }

    #[test]
    fn plain_paste_turns_newlines_into_carriage_returns() {
        assert_eq!(paste_bytes("a\nb\r\nc\rd", false), b"a\rb\rc\rd".to_vec());
    }

    #[test]
    fn bracketed_paste_keeps_the_text_but_drops_control_bytes() {
        let bytes = paste_bytes("ls\n\x03\x7f\ttab", true);
        assert_eq!(bytes, b"\x1b[200~ls\n\ttab\x1b[201~".to_vec());
    }
}
