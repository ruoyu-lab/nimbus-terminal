//! 快捷键，和只用快捷键触发的几个动作（字号、查找）。macOS 用 ⌘，别的平台用 Ctrl+Shift；
//! 带不带 Shift 分成两个动作的那几个（分屏、查找上一处下一处）在 Windows、Linux 上另有按法。
//! 界面上的提示怎么写也在这里（[`Shortcut`]），和按键的处理放在一起，改一处就两边都对。

use super::*;

impl Workspace {
    /// 快捷键。macOS 用 ⌘，别的平台用 Ctrl+Shift（Ctrl 单独要留给 shell）。收下了返回 `true`
    pub fn shortcut(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if modifiers.has_alt() {
            // macOS 上 Option 加什么都交给终端（当 Meta 用）；Windows、Linux 上 Alt+Shift+加号、减号是分屏
            return !Shortcut::MAC && self.alt_shortcut(event);
        }
        // Ctrl+Tab / Ctrl+Shift+Tab：各平台都一样
        if modifiers.has_control()
            && !modifiers.has_super()
            && matches!(event.key, Key::Named(NamedKey::Tab))
        {
            self.cycle(if modifiers.has_shift() { -1 } else { 1 });
            return true;
        }
        // F3 / Shift+F3：查找条开着时找上一处 / 下一处（Windows 的习惯）；没开着时照常交给终端里的程序
        if !Shortcut::MAC
            && matches!(event.key, Key::Named(NamedKey::F3))
            && !modifiers.has_control()
            && !modifiers.has_super()
            && self
                .focused_session()
                .is_some_and(|session| session.find_open().get_untracked())
        {
            self.find_step(!modifiers.has_shift());
            return true;
        }
        let command = if modifiers.has_super() {
            !modifiers.has_control()
        } else {
            modifiers.has_control() && modifiers.has_shift()
        };
        if !command {
            return false;
        }
        // Shift 是不是另外按的：⌘ 的一套里 ⌘D 和 ⌘⇧D 是两个动作；Ctrl+Shift 的一套里 Shift 本来就按着，
        // 分不出来，都算不带 Shift 的那个（另一个换了按法：Alt+Shift+减号、Shift+F3）
        let shifted = modifiers.has_shift() && (Shortcut::MAC || modifiers.has_super());
        // ⌘↑ / ⌘↓：跳到上一条 / 下一条命令（要 shell 集成发的命令标记；没有时让按键照常交给终端）
        if let Key::Named(named @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) = event.key {
            return self
                .focused_session()
                .is_some_and(|session| session.jump_prompt(named == NamedKey::ArrowUp));
        }
        let Key::Character(text) = &event.base else {
            return false;
        };
        match text.as_str() {
            "t" | "T" => self.new_tab(),
            "w" | "W" => self.close_active(),
            "b" | "B" => self.toggle_sidebar(),
            // ⌘D 左右分，⌘⇧D 上下分（iTerm 的习惯）
            "d" | "D" => self.split(if shifted { Axis::Rows } else { Axis::Columns }),
            // ⌘+ / ⌘- / ⌘0：字号（直接改设置里的字号，和设置页是同一个）
            "=" | "+" => self.zoom_font(1.0),
            "-" | "_" => self.zoom_font(-1.0),
            "0" => self.reset_font_size(),
            "f" | "F" => self.find_in_terminal(),
            "g" | "G" => self.find_step(!shifted),
            "k" | "K" => self.palette_open.update(|open| *open = !*open),
            "," => self.toggle_settings(),
            // ⌘⇧[ / ⌘⇧]
            "[" | "{" => self.cycle(-1),
            "]" | "}" => self.cycle(1),
            // ⌘1..⌘8 是第几个，⌘9 是最后一个（浏览器的习惯）
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => {
                self.select_index(digit.parse::<usize>().unwrap_or(1) - 1)
            }
            "9" => self.select_index(self.bar.get_untracked().len().saturating_sub(1)),
            _ => return false,
        }
        true
    }

    /// Windows、Linux 上按着 Alt 的快捷键：Alt+Shift+加号左右分、Alt+Shift+减号上下分（Windows Terminal 的按法）。
    /// 按着 Ctrl 的不算（AltGr 报上来就是 Ctrl+Alt，那是在打字）
    fn alt_shortcut(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if !modifiers.has_shift() || modifiers.has_control() || modifiers.has_super() {
            return false;
        }
        let Key::Character(text) = &event.base else {
            return false;
        };
        let axis = match text.as_str() {
            "=" | "+" => Axis::Columns,
            "-" | "_" => Axis::Rows,
            _ => return false,
        };
        self.split(axis);
        true
    }

    /// 字号加减 `delta`，限制在设置允许的范围里
    pub fn zoom_font(&self, delta: f64) {
        let (low, high) = crate::settings::FONT_SIZE;
        self.form
            .font_size
            .update(|size| *size = (*size + delta).clamp(low, high));
    }

    /// 字号回到默认
    pub fn reset_font_size(&self) {
        self.form
            .font_size
            .set(crate::settings::Settings::default().font_size);
    }

    /// ⌘F：在聚焦的终端里打开查找条
    pub fn find_in_terminal(&self) {
        if let Some(session) = self.focused_session() {
            session.open_find();
        }
    }

    /// ⌘G、⌘⇧G：跳到上一处（更早的）、下一处
    pub fn find_step(&self, older: bool) {
        if let Some(session) = self.focused_session() {
            session.find_step(older);
        }
    }
}

/// 界面上提到的快捷键（按钮的提示、侧栏、命令面板、设置页的列表）在这个平台上怎么写：
/// macOS 是 ⌘、⇧ 这些符号，Windows、Linux 是 `Ctrl+Shift+T` 这样的字。
/// 按键本身在 [`Workspace::shortcut`] 处理，终端里的复制、粘贴在 `app::input`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    NewTab,
    /// 关掉聚焦的终端（拆分过的）或标签；设置页里是返回
    Close,
    SplitColumns,
    SplitRows,
    Palette,
    Find,
    /// 查找：上一处（更早的）
    FindOlder,
    /// 查找：下一处
    FindNewer,
    PreviousCommand,
    NextCommand,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Sidebar,
    Settings,
    /// 切到第 1 – 8 个标签
    TabNumber,
    /// 切到最后一个标签
    LastTab,
    PreviousTab,
    NextTab,
    /// Ctrl+Tab：各平台都一样
    CycleTabs,
    Copy,
    Paste,
    SelectAll,
}

impl Shortcut {
    /// 快捷键按 macOS 的一套（⌘）；否则是 Windows、Linux 的一套
    pub const MAC: bool = cfg!(target_os = "macos");

    /// 这个平台上按哪几个键
    pub fn keys(self) -> &'static str {
        if Self::MAC { self.mac() } else { self.others() }
    }

    fn mac(self) -> &'static str {
        match self {
            Self::NewTab => "⌘T",
            Self::Close => "⌘W",
            Self::SplitColumns => "⌘D",
            Self::SplitRows => "⌘⇧D",
            Self::Palette => "⌘K",
            Self::Find => "⌘F",
            Self::FindOlder => "⌘G",
            Self::FindNewer => "⌘⇧G",
            Self::PreviousCommand => "⌘↑",
            Self::NextCommand => "⌘↓",
            Self::ZoomIn => "⌘+",
            Self::ZoomOut => "⌘-",
            Self::ZoomReset => "⌘0",
            Self::Sidebar => "⌘B",
            Self::Settings => "⌘,",
            Self::TabNumber => "⌘1 – ⌘8",
            Self::LastTab => "⌘9",
            Self::PreviousTab => "⌘⇧[",
            Self::NextTab => "⌘⇧]",
            Self::CycleTabs => "Ctrl+Tab",
            Self::Copy => "⌘C",
            Self::Paste => "⌘V",
            Self::SelectAll => "⌘A",
        }
    }

    /// Windows、Linux：Ctrl+Shift 加 macOS 上 ⌘ 加的那个键；分屏、查找上一处下一处另有按法
    /// （Ctrl+Shift+D、Ctrl+Shift+G 也还在，是左右分、上一处）
    fn others(self) -> &'static str {
        match self {
            Self::NewTab => "Ctrl+Shift+T",
            Self::Close => "Ctrl+Shift+W",
            Self::SplitColumns => "Alt+Shift+=",
            Self::SplitRows => "Alt+Shift+-",
            Self::Palette => "Ctrl+Shift+K",
            Self::Find => "Ctrl+Shift+F",
            Self::FindOlder => "F3",
            Self::FindNewer => "Shift+F3",
            Self::PreviousCommand => "Ctrl+Shift+↑",
            Self::NextCommand => "Ctrl+Shift+↓",
            Self::ZoomIn => "Ctrl+Shift+=",
            Self::ZoomOut => "Ctrl+Shift+-",
            Self::ZoomReset => "Ctrl+Shift+0",
            Self::Sidebar => "Ctrl+Shift+B",
            Self::Settings => "Ctrl+Shift+,",
            Self::TabNumber => "Ctrl+Shift+1 – 8",
            Self::LastTab => "Ctrl+Shift+9",
            Self::PreviousTab => "Ctrl+Shift+[",
            Self::NextTab => "Ctrl+Shift+]",
            Self::CycleTabs => "Ctrl+Tab",
            Self::Copy => "Ctrl+Shift+C",
            Self::Paste => "Ctrl+Shift+V",
            Self::SelectAll => "Ctrl+Shift+A",
        }
    }
}
