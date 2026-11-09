//! 终端文字里的链接：指针停上去的高亮、⌘ 点击打开、右键菜单里的打开和复制。怎么在文字里认出链接是 `links`。

use super::*;

impl Session {
    /// 指针在这一格上，停着的是哪个链接：网址，或者本机上真的存在的文件路径
    pub(super) fn link_under_pointer(&self, offset: usize) -> Option<view::Hover> {
        let (row, column) = self.pointer_cell.get()?;
        let found = {
            let terminal = self.terminal.borrow();
            view::source_line(&terminal, row, offset)
                .and_then(|line| links::link_at(line, column, terminal.link_uris()))
        };
        let (link, start, end) = found?;
        let hover = view::Hover { row, start, end };
        // 还停在同一个上：不再问文件系统
        if self.hover.get() == Some(hover) {
            return Some(hover);
        }
        match link {
            links::Link::Url(_) => Some(hover),
            links::Link::Path { path, .. } => {
                (self.local.get() && self.resolve_path(&path).is_some()).then_some(hover)
            }
        }
    }

    /// 指针停着的链接变了就记下来：画面要加下划线，光标要变手指
    pub(super) fn update_hover(&self, offset: usize) {
        let hover = self.link_under_pointer(offset);
        if hover != self.hover.get() {
            self.hover.set(hover);
            self.hover_dirty.set(true);
            self.hovering_link.set(hover.is_some());
        }
    }

    /// 指针移到了 `cell` 上（离开终端是 `None`）
    pub(super) fn pointer_over(&self, cell: Option<(usize, usize)>) {
        if self.pointer_cell.replace(cell) != cell {
            self.update_hover(self.offset.get());
            if self.hover_dirty.get() {
                self.flush();
            }
        }
    }

    /// 标成本机的终端（别的是服务器、容器）
    pub fn set_local(&self, local: bool) {
        self.local.set(local);
    }

    /// ⌘（别的平台是 Ctrl）加点击：点在链接上就打开。收下了返回 `true`
    pub(super) fn open_link_at(&self, event: &PointerEvent) -> bool {
        let modifier = if cfg!(target_os = "macos") {
            event.modifiers.has_super()
        } else {
            event.modifiers.has_control()
        };
        if !modifier || !matches!(event.button, Some(MouseButton::Left)) {
            return false;
        }
        let Some((row, column)) = self.cell_at(event.position) else {
            return false;
        };
        let link = {
            let terminal = self.terminal.borrow();
            view::source_line(&terminal, row, self.offset.get())
                .and_then(|line| links::link_at(line, column, terminal.link_uris()))
        };
        let Some((link, ..)) = link else {
            return false;
        };
        self.open_link(&link)
    }

    pub(super) fn open_link(&self, link: &links::Link) -> bool {
        match link {
            links::Link::Url(url) => aureole::open_url(url),
            links::Link::Path { path, .. } => {
                if !self.local.get() {
                    return false;
                }
                let Some(resolved) = self.resolve_path(path) else {
                    return false;
                };
                // 目录在访达里打开；文件只有常见的文档、图片、代码才直接用默认应用打开，
                // 别的（脚本、应用、安装包……）只在访达里选中，不会被点一下就运行起来
                if resolved.is_dir() || links::is_plain_document(&resolved) {
                    aureole::open_path(&resolved)
                } else {
                    aureole::reveal_path(&resolved)
                }
            }
        }
    }

    /// 终端里写的路径对应本机的哪个已存在的文件：`~` 展开，相对路径从 shell 现在的目录算起
    pub(super) fn resolve_path(&self, path: &str) -> Option<std::path::PathBuf> {
        let candidate = if let Some(rest) = path.strip_prefix("~/") {
            crate::store::user_home()?.join(rest)
        } else if path.starts_with('/') {
            std::path::PathBuf::from(path)
        } else {
            self.working_directory()?.join(path)
        };
        candidate.canonicalize().ok()
    }

    /// 记下 `position`（网格里的位置）那一格上的链接：网址，或者本机上真有的文件路径
    pub(super) fn remember_link(&self, position: [f32; 2]) {
        let link = self.cell_at(position).and_then(|(row, column)| {
            let terminal = self.terminal.borrow();
            view::source_line(&terminal, row, self.offset.get())
                .and_then(|line| links::link_at(line, column, terminal.link_uris()))
        });
        let link = link.map(|(link, ..)| link).filter(|link| match link {
            links::Link::Url(_) => true,
            links::Link::Path { path, .. } => self.local.get() && self.resolve_path(path).is_some(),
        });
        *self.context_link.borrow_mut() = link;
    }

    /// 右键点中的链接（右键菜单现看）
    pub fn context_link(&self) -> Option<links::Link> {
        self.context_link.borrow().clone()
    }

    /// 打开链接（菜单里的「打开」）
    pub fn open_link_now(&self, link: &links::Link) -> bool {
        self.open_link(link)
    }
}
