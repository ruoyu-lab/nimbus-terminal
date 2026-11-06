//! 字节流 → 事件的状态机。
//!
//! 只认转义序列的**语法**（谁开头、参数怎么切、到哪结束），不认它是什么意思（那是
//! [`super::Terminal::apply`]）。分开写的好处是：遇到没实现的序列时能准确地把它整条吃掉，
//! 剩下的字节照常当文字显示——这是「不乱屏」的关键。
//!
//! UTF-8 是增量解码的：一个汉字的三个字节可能分在两次 `feed` 里（pty 就会这么切），
//! 半个字先存在状态里，等下一块补齐。

/// 解析出来的一个动作
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// 要写进格子的字（控制字符、C1 都不从这里走）
    Print(char),
    /// C0 控制字符（`0x00..=0x1f`、`0x7f`）
    Control(u8),
    /// `ESC` + 中间字节 + 终结字节，比如 `ESC M`（反向换行）
    Escape {
        intermediates: Vec<u8>,
        final_byte: u8,
    },
    /// `CSI`：`private` 是 `?` `>` `<` `=` `!` 这类打头的私有标记，参数缺省时是 `-1`。
    /// `subparams` 的第 `i` 位为 1，表示 `params[i]` 前面的分隔符是 `:` 而不是 `;`，
    /// 它是前一个参数的子参数（`4:3` 是弯曲下划线，`38:2::255:0:0` 是直接色）
    Csi {
        private: Option<u8>,
        intermediates: Vec<u8>,
        params: Vec<i32>,
        subparams: u32,
        final_byte: u8,
    },
    /// `OSC`：编号和数据，比如 `OSC 0 ; 标题 BEL`
    Osc { code: i32, text: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    /// `ESC` 之后的中间字节（`ESC ( B` 的 `(`），认完就丢
    EscapeIntermediate,
    CsiParam,
    CsiIntermediate,
    /// 参数写乱了（`CSI 1 ? A`），整条丢到终结字节
    CsiIgnore,
    Osc,
    /// OSC 里看到一个 `ESC`，可能是 `ESC \` 收尾
    OscEscape,
    /// DCS、SOS、PM、APC：内容整条丢掉（做 sixel 时要在这里分叉出去）
    Ignore,
    IgnoreEscape,
}

/// 一条序列最多留多少个参数、多长字符串（防止有人拿 `CSI 1;1;1;...` 撑爆内存）
const MAX_PARAMS: usize = 32;
const MAX_STRING: usize = 4096;

pub struct Parser {
    state: State,
    params: Vec<i32>,
    /// 正在攒的那个参数
    param: i32,
    /// 一个数字都没见到时不要凭空补出个 0
    has_param: bool,
    /// 哪些参数前面是冒号，见 [`Event::Csi`]
    subparams: u32,
    private: Option<u8>,
    intermediates: Vec<u8>,
    string: Vec<u8>,
    /// 攒了一半的 UTF-8
    utf8: [u8; 4],
    utf8_len: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            params: Vec::new(),
            param: 0,
            has_param: false,
            subparams: 0,
            private: None,
            intermediates: Vec::new(),
            string: Vec::new(),
            utf8: [0; 4],
            utf8_len: 0,
        }
    }

    /// 喂一块字节，解出来的动作按顺序追加到 `out`
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for &byte in bytes {
            self.byte(byte, out);
        }
    }

    fn byte(&mut self, byte: u8, out: &mut Vec<Event>) {
        match self.state {
            State::Ground => self.ground(byte, out),
            State::Escape => self.escape(byte, out),
            State::EscapeIntermediate => {
                if (0x20..=0x2f).contains(&byte) {
                    self.push_intermediate(byte);
                } else if (0x30..=0x7e).contains(&byte) {
                    out.push(Event::Escape {
                        intermediates: std::mem::take(&mut self.intermediates),
                        final_byte: byte,
                    });
                    self.state = State::Ground;
                } else {
                    self.control_or_escape(byte, out);
                }
            }
            State::CsiParam => self.csi_param(byte, out),
            State::CsiIntermediate => {
                if (0x20..=0x2f).contains(&byte) {
                    self.push_intermediate(byte);
                } else if (0x30..=0x3f).contains(&byte) {
                    self.state = State::CsiIgnore;
                } else if (0x40..=0x7e).contains(&byte) {
                    self.finish_csi(byte, out);
                } else {
                    self.control_or_escape(byte, out);
                }
            }
            State::CsiIgnore => {
                if (0x40..=0x7e).contains(&byte) {
                    self.state = State::Ground;
                } else {
                    self.control_or_escape(byte, out);
                }
            }
            State::Osc => match byte {
                0x07 => self.finish_osc(out),
                0x1b => self.state = State::OscEscape,
                0x08 => {}
                _ => {
                    if self.string.len() < MAX_STRING {
                        self.string.push(byte);
                    }
                }
            },
            State::OscEscape => {
                self.finish_osc(out);
                if byte == b'\\' {
                    self.state = State::Ground;
                } else {
                    self.escape(byte, out);
                }
            }
            State::Ignore => match byte {
                0x07 => self.state = State::Ground,
                0x18 | 0x9c => self.state = State::Ground,
                0x1b => self.state = State::IgnoreEscape,
                _ => {}
            },
            State::IgnoreEscape => {
                if byte == b'\\' {
                    self.state = State::Ground;
                } else {
                    self.escape(byte, out);
                }
            }
        }
    }

    fn ground(&mut self, byte: u8, out: &mut Vec<Event>) {
        match byte {
            0x1b => self.begin_escape(),
            0x00..=0x1f => {
                self.utf8_len = 0;
                out.push(Event::Control(byte));
            }
            0x7f => {
                self.utf8_len = 0;
                out.push(Event::Control(byte));
            }
            0x20..=0x7e => {
                self.utf8_len = 0;
                out.push(Event::Print(byte as char));
            }
            _ => self.utf8(byte, out),
        }
    }

    /// 攒 UTF-8：凑够前导字节说的长度才解
    fn utf8(&mut self, byte: u8, out: &mut Vec<Event>) {
        // 攒了一半又遇到不是续字节的（新前导字节或者坏字节）：前面那段作废，重新开始
        if self.utf8_len > 0 && byte & 0xc0 != 0x80 {
            self.utf8_len = 0;
        }
        if self.utf8_len == 0 {
            let want = utf8_len(byte);
            if want == 0 {
                return;
            }
            self.utf8[0] = byte;
            self.utf8_len = 1;
        } else {
            self.utf8[self.utf8_len] = byte;
            self.utf8_len += 1;
        }
        if self.utf8_len < utf8_len(self.utf8[0]) {
            return;
        }
        let mut bytes = [0u8; 4];
        bytes[..self.utf8_len].copy_from_slice(&self.utf8[..self.utf8_len]);
        let len = self.utf8_len;
        self.utf8_len = 0;
        // 过长编码、坏组合都会被 `from_utf8` 挡掉
        if let Ok(text) = std::str::from_utf8(&bytes[..len]) {
            for ch in text.chars() {
                self.print_or_c1(ch, out);
            }
        }
    }

    /// 解出来的字符如果是 C1（`U+0080..U+009F`），当成对应的 8 位控制序列
    fn print_or_c1(&mut self, ch: char, out: &mut Vec<Event>) {
        let code = ch as u32;
        if (0x80..=0x9f).contains(&code) {
            match code {
                0x9b => self.begin_escape_csi(),
                0x9d => self.begin_osc(),
                0x90 | 0x98 | 0x9e | 0x9f => self.state = State::Ignore,
                _ => {}
            }
            return;
        }
        out.push(Event::Print(ch));
    }

    fn begin_escape(&mut self) {
        self.state = State::Escape;
        self.params.clear();
        self.param = 0;
        self.has_param = false;
        self.subparams = 0;
        self.private = None;
        self.intermediates.clear();
    }

    fn begin_escape_csi(&mut self) {
        self.begin_escape();
        self.state = State::CsiParam;
    }

    fn begin_osc(&mut self) {
        self.begin_escape();
        self.state = State::Osc;
        self.string.clear();
    }

    fn escape(&mut self, byte: u8, out: &mut Vec<Event>) {
        match byte {
            b'[' => self.begin_escape_csi(),
            b']' => self.begin_osc(),
            b'P' | b'X' | b'^' | b'_' => self.state = State::Ignore,
            0x20..=0x2f => {
                self.push_intermediate(byte);
                self.state = State::EscapeIntermediate;
            }
            0x1b => self.begin_escape(),
            0x30..=0x7e => {
                out.push(Event::Escape {
                    intermediates: std::mem::take(&mut self.intermediates),
                    final_byte: byte,
                });
                self.state = State::Ground;
            }
            _ => self.control_or_escape(byte, out),
        }
    }

    fn csi_param(&mut self, byte: u8, out: &mut Vec<Event>) {
        match byte {
            b'0'..=b'9' => {
                self.param = (self.param * 10 + i32::from(byte - b'0')).min(9999);
                self.has_param = true;
            }
            b';' => self.push_param(),
            b':' => {
                self.push_param();
                // 下一个参数（下标就是现在的个数）是子参数
                if self.params.len() < MAX_PARAMS {
                    self.subparams |= 1 << self.params.len();
                }
            }
            // `CSI ? ...` 这类私有前缀，以及只在开头有意义的 `>` `<` `=` `!`
            b'?' | b'>' | b'<' | b'=' | b'!' if self.params.is_empty() && !self.has_param => {
                self.private = Some(byte)
            }
            0x20..=0x2f => {
                self.push_param();
                self.push_intermediate(byte);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7e => self.finish_csi(byte, out),
            0x3c..=0x3f => self.state = State::CsiIgnore,
            0x1b => self.begin_escape(),
            _ => self.control_or_escape(byte, out),
        }
    }

    fn push_param(&mut self) {
        if self.params.len() < MAX_PARAMS {
            self.params
                .push(if self.has_param { self.param } else { -1 });
        }
        self.param = 0;
        self.has_param = false;
    }

    fn push_intermediate(&mut self, byte: u8) {
        if self.intermediates.len() < 4 {
            self.intermediates.push(byte);
        }
    }

    fn finish_csi(&mut self, final_byte: u8, out: &mut Vec<Event>) {
        if self.has_param && self.params.len() < MAX_PARAMS {
            self.params.push(self.param);
        }
        out.push(Event::Csi {
            private: self.private.take(),
            intermediates: std::mem::take(&mut self.intermediates),
            params: std::mem::take(&mut self.params),
            subparams: std::mem::take(&mut self.subparams),
            final_byte,
        });
        self.param = 0;
        self.has_param = false;
        self.state = State::Ground;
    }

    fn finish_osc(&mut self, out: &mut Vec<Event>) {
        let text = String::from_utf8_lossy(&std::mem::take(&mut self.string)).into_owned();
        let (code, rest) = match text.split_once(';') {
            Some((code, rest)) => (code.parse::<i32>().unwrap_or(-1), rest.to_string()),
            None => (text.parse::<i32>().unwrap_or(-1), String::new()),
        };
        out.push(Event::Osc { code, text: rest });
        self.state = State::Ground;
    }

    /// 序列中间来的 C0 控制字符照样执行（`Ctrl-C` 打断一条写了一半的序列是合法的）
    fn control_or_escape(&mut self, byte: u8, out: &mut Vec<Event>) {
        match byte {
            0x18 | 0x9c => self.state = State::Ground,
            0x1b => self.begin_escape(),
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => out.push(Event::Control(byte)),
            _ => {}
        }
    }
}

/// 前导字节说的这个字符一共几个字节；不是合法前导字节时 1（当成坏字节丢掉）
fn utf8_len(byte: u8) -> usize {
    match byte {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(bytes: &[&[u8]]) -> Vec<Event> {
        let mut parser = Parser::new();
        let mut out = Vec::new();
        for chunk in bytes {
            parser.feed(chunk, &mut out);
        }
        out
    }

    #[test]
    fn csi_with_parameters_and_private_flag() {
        assert_eq!(
            events(&["\x1b[?1049h".as_bytes()]),
            vec![Event::Csi {
                private: Some(b'?'),
                intermediates: vec![],
                params: vec![1049],
                subparams: 0,
                final_byte: b'h'
            }]
        );
    }

    #[test]
    fn omitted_parameters_report_minus_one() {
        // `CSI ;H`：第一个参数缺省，语义上是 1，交给上层补
        assert_eq!(
            events(&["\x1b[;3H".as_bytes()]),
            vec![Event::Csi {
                private: None,
                intermediates: vec![],
                params: vec![-1, 3],
                subparams: 0,
                final_byte: b'H'
            }]
        );
    }

    #[test]
    fn utf8_split_across_reads() {
        let bytes = "好".as_bytes();
        assert_eq!(
            events(&[&bytes[..1], &bytes[1..]]),
            vec![Event::Print('好')]
        );
        assert_eq!(
            events(&[&bytes[..2], &bytes[2..]]),
            vec![Event::Print('好')]
        );
    }

    /// 半个字后面来了普通字节：残段丢掉，那个字节照常显示（不能把后面的内容一起吃掉）
    #[test]
    fn incomplete_sequence_resyncs_on_next_printable() {
        let bytes = "好".as_bytes();
        assert_eq!(events(&[&bytes[..1], b"a"]), vec![Event::Print('a')]);
    }

    #[test]
    fn bad_lead_byte_is_dropped_not_printed() {
        // 0x80 是孤零零的续字节：不能当成字符打出来
        assert_eq!(events(&[&[0x80, b'b']]), vec![Event::Print('b')]);
    }

    #[test]
    fn osc_title_ends_with_bell() {
        assert_eq!(
            events(&["\x1b]0;我的终端\x07".as_bytes()]),
            vec![Event::Osc {
                code: 0,
                text: "我的终端".into()
            }]
        );
    }

    /// `4:3`（弯曲下划线）、`38:2::r:g:b`：冒号后面的是子参数，要能和分号区分开
    #[test]
    fn colon_marks_subparameters() {
        assert_eq!(
            events(&["\x1b[4:3;38:2::1:2:3m".as_bytes()]),
            vec![Event::Csi {
                private: None,
                intermediates: vec![],
                params: vec![4, 3, 38, 2, -1, 1, 2, 3],
                // 第 1、3、4、5、6、7 个参数前面是冒号
                subparams: 0b1111_1010,
                final_byte: b'm'
            }]
        );
    }

    #[test]
    fn dcs_is_swallowed_whole() {
        // 没实现的 DCS（这里是 DECUDAK）整条吃掉，后面的字照样是字
        assert_eq!(
            events(&["\x1bP1$z\x1b\\x".as_bytes()]),
            vec![Event::Print('x')]
        );
    }

    #[test]
    fn control_inside_csi_still_executes() {
        let out = events(&["\x1b[3\x07m".as_bytes()]);
        assert!(out.contains(&Event::Control(7)), "{out:?}");
        assert!(out.contains(&Event::Csi {
            private: None,
            intermediates: vec![],
            params: vec![3],
            subparams: 0,
            final_byte: b'm'
        }));
    }
}
