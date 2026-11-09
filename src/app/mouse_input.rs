//! 鼠标：程序要鼠标时（vim、htop、tmux）把指针事件编码了写给它，不要时选字、滚轮、右键归终端自己。

use super::*;

impl Session {
    /// 现在鼠标归程序（要上报）吗？按着 Shift 时永远归终端：这是和 xterm 一样的约定，
    /// 程序抓着鼠标的时候，还能用 Shift+拖动选字、复制
    pub(super) fn mouse_reporting(&self, modifiers: Modifiers) -> bool {
        self.terminal.borrow().modes.mouse != MouseMode::Off && !modifiers.has_shift()
    }

    /// 元素坐标（逻辑像素）落在第几行第几列，限制在屏内。格子大小还没量到时是 `None`
    pub(super) fn cell_at(&self, position: [f32; 2]) -> Option<(usize, usize)> {
        let (width, height) = self.cell.get()?;
        let (columns, rows) = self.terminal.borrow().size();
        let column = ((position[0] / width).floor().max(0.0) as usize).min(columns - 1);
        let row = ((position[1] / height).floor().max(0.0) as usize).min(rows - 1);
        Some((row, column))
    }

    /// 编码一次鼠标动作写给程序
    pub(super) fn report_mouse(
        &self,
        button: Option<Button>,
        action: Action,
        cell: (usize, usize),
        modifiers: Modifiers,
    ) {
        let (mode, sgr) = {
            let terminal = self.terminal.borrow();
            (terminal.modes.mouse, terminal.modes.mouse_sgr)
        };
        let report = Report {
            button,
            action,
            column: cell.1,
            row: cell.0,
            shift: modifiers.has_shift(),
            alt: modifiers.has_alt(),
            control: modifiers.has_control(),
        };
        if let Some(bytes) = mouse::encode(&report, mode, sgr) {
            self.write(&bytes);
        }
    }

    pub(super) fn pointer_down(&self, event: &PointerEvent) {
        if self.open_link_at(event) {
            // 这一下是点链接，不开始选字
            event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
            return;
        }
        if !self.mouse_reporting(event.modifiers) {
            return;
        }
        let button = match event.button {
            Some(MouseButton::Left) => Button::Left,
            Some(MouseButton::Middle) => Button::Middle,
            Some(MouseButton::Right) => Button::Right,
            _ => return,
        };
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
        // 这次按下归程序：不选字、不开始拖动；抓住指针，拖出窗口也照样收到移动和松开
        event.prevent_builtin(Builtin::TEXT_SELECT | Builtin::DRAG);
        event.capture_pointer();
        self.grid.set_selection(None);
        self.pressed.set(Some(button));
        self.last_cell.set(Some(cell));
        self.report_mouse(Some(button), Action::Press, cell, event.modifiers);
    }

    pub(super) fn pointer_up(&self, event: &PointerEvent) {
        // 选中后自动复制：这次选字归终端自己（程序没抓着鼠标）时，松手就把选中的字放进剪贴板
        if self.form.copy_on_select.get_untracked()
            && self.pressed.get().is_none()
            && !self.mouse_reporting(event.modifiers)
            && let Some(text) = self.grid.selected_text()
            && !text.is_empty()
        {
            aureole::clipboard::write_text(text);
        }
        // 按下时归了程序，松开就一定要报，不管这时候 Shift 按没按、模式变没变
        let Some(button) = self.pressed.take() else {
            return;
        };
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
        self.report_mouse(Some(button), Action::Release, cell, event.modifiers);
    }

    pub(super) fn pointer_move(&self, event: &PointerEvent) {
        self.pointer_over(self.cell_at(event.position));
        if !self.mouse_reporting(event.modifiers) {
            return;
        }
        let Some(cell) = self.cell_at(event.position) else {
            return;
        };
        if self.last_cell.get() == Some(cell) {
            return;
        }
        self.last_cell.set(Some(cell));
        self.report_mouse(self.pressed.get(), Action::Motion, cell, event.modifiers);
    }

    /// 窗口得到、失去键盘焦点：程序要的话告诉它（vim 靠它在切回来时重读文件）
    pub(super) fn focus_changed(&self, focused: bool) {
        if self.terminal.borrow().modes.focus_reporting {
            self.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    /// 滚轮、触控板。程序要鼠标就上报成滚轮键；否则主屏是翻历史；
    /// 副屏（less、man、vim）没有历史，翻成方向键，和别的终端一样
    pub(super) fn wheel(&self, event: &WheelEvent) {
        let cell_height = self
            .cell
            .get()
            .map_or(view::FONT_SIZE * view::LINE_HEIGHT, |cell| cell.1);
        let lines = match event.delta {
            // 滚轮一格滚几行听设置的
            WheelDelta::Lines(delta) => delta[1] * self.form.wheel_lines(),
            WheelDelta::Pixels(delta) => delta[1] / cell_height,
        };
        // 事件里正数是「内容往上走」（往最新的方向），翻历史的方向正好相反
        let total = self.wheel_rest.get() - lines;
        let whole = total.trunc();
        self.wheel_rest.set(total - whole);
        let steps = whole as isize;
        if steps == 0 {
            return;
        }
        if self.mouse_reporting(event.modifiers) {
            if let Some(cell) = self.cell_at(event.position) {
                let button = if steps > 0 {
                    Button::WheelUp
                } else {
                    Button::WheelDown
                };
                for _ in 0..steps.unsigned_abs().min(12) {
                    self.report_mouse(Some(button), Action::Press, cell, event.modifiers);
                }
            }
            return;
        }
        let (alternate, application) = {
            let terminal = self.terminal.borrow();
            (terminal.on_alternate, terminal.modes.application_cursor)
        };
        if alternate {
            let arrow = match (steps > 0, application) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..steps.unsigned_abs().min(12) {
                self.write(arrow);
            }
        } else {
            self.scroll_back(steps);
        }
    }

    /// 右键。程序抓着鼠标时（按着 Shift 除外）右键归程序；设置里选了「右键粘贴」就直接粘贴；
    /// 都不是就该弹右键菜单，返回 `true`
    pub(super) fn secondary_click(&self, position: [f32; 2], modifiers: Modifiers) -> bool {
        self.remember_link(position);
        if self.mouse_reporting(modifiers) {
            return false;
        }
        if self.form.right_click_paste.get_untracked() {
            self.paste();
            return false;
        }
        true
    }
}
