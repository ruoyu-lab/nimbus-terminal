//! 快捷键，和只用快捷键触发的几个动作（字号、查找）。macOS 用 ⌘，别的平台用 Ctrl+Shift。

use super::*;

impl Workspace {
    /// 快捷键。macOS 用 ⌘，别的平台用 Ctrl+Shift（Ctrl 单独要留给 shell）。收下了返回 `true`
    pub fn shortcut(&self, event: &KeyEvent) -> bool {
        let modifiers = event.modifiers;
        if modifiers.has_alt() {
            return false;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab：各平台都一样
        if modifiers.has_control()
            && !modifiers.has_super()
            && matches!(event.key, Key::Named(NamedKey::Tab))
        {
            self.cycle(if modifiers.has_shift() { -1 } else { 1 });
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
            "d" | "D" => self.split(if modifiers.has_shift() {
                Axis::Rows
            } else {
                Axis::Columns
            }),
            // ⌘+ / ⌘- / ⌘0：字号（直接改设置里的字号，和设置页是同一个）
            "=" | "+" => self.zoom_font(1.0),
            "-" | "_" => self.zoom_font(-1.0),
            "0" => self.reset_font_size(),
            "f" | "F" => self.find_in_terminal(),
            "g" | "G" => self.find_step(!modifiers.has_shift()),
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
