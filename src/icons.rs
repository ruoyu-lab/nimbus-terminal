//! 图标。框架不带图标集，图标就是一条矢量路径：24 × 24 的画布，1.6 的圆头描边，
//! 颜色跟着周围文字走。线条风格，和整个界面的轻一致。

use aureole::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Plus,
    Close,
    Search,
    Terminal,
    Sidebar,
    PanelRight,
    Laptop,
    Server,
    Box,
    Folder,
    File,
    Download,
    Upload,
    Refresh,
    ArrowUp,
    ArrowDown,
    Sliders,
    Columns,
    Rows,
    Sun,
    Keyboard,
    Info,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Github,
}

impl Icon {
    fn data(self) -> &'static str {
        match self {
            Icon::Plus => "M12 5v14 M5 12h14",
            Icon::Close => "M6 6l12 12 M18 6L6 18",
            Icon::Search => "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z M20 20l-4-4",
            Icon::Terminal => {
                "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z M7.5 10l3 2-3 2 M13 15h4"
            }
            Icon::Sidebar => {
                "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M9 4v16"
            }
            Icon::PanelRight => {
                "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M15 4v16"
            }
            Icon::Server => {
                "M5 4h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z M5 13h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1z M8 7.5h.01 M8 16.5h.01"
            }
            Icon::Laptop => "M5 6h14a1 1 0 0 1 1 1v9H4V7a1 1 0 0 1 1-1z M2.5 19h19",
            // 集装箱（Feather 的 box）
            Icon::Box => {
                "M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z M3.27 6.96L12 12.01l8.73-5.05 M12 22.08V12"
            }
            Icon::Folder => {
                "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
            }
            Icon::File => "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5",
            Icon::Download => "M12 4v11 M7 10l5 5 5-5 M5 20h14",
            Icon::Upload => "M12 16V5 M7 10l5-5 5 5 M5 20h14",
            Icon::Refresh => {
                "M20 11A8.1 8.1 0 0 0 4.5 9 M4 5v4h4 M4 13a8.1 8.1 0 0 0 15.5 2 M20 19v-4h-4"
            }
            Icon::ArrowUp => "M12 19V5 M5 12l7-7 7 7",
            Icon::ArrowDown => "M12 5v14 M5 12l7 7 7-7",
            Icon::Sliders => {
                "M4 7h9 M17 7h3 M4 17h3 M11 17h9 M13 7a2 2 0 1 0 4 0a2 2 0 1 0-4 0 M7 17a2 2 0 1 0 4 0a2 2 0 1 0-4 0"
            }
            Icon::Columns => {
                "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M12 4v16"
            }
            Icon::Rows => {
                "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M3 12h18"
            }
            Icon::Sun => {
                "M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8z M12 2.5v2 M12 19.5v2 M2.5 12h2 M19.5 12h2 M5.3 5.3l1.4 1.4 M17.3 17.3l1.4 1.4 M18.7 5.3l-1.4 1.4 M6.7 17.3l-1.4 1.4"
            }
            Icon::Keyboard => {
                "M3 7h18a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1z M6 11h.01 M10 11h.01 M14 11h.01 M18 11h.01 M7.5 14.5h9"
            }
            Icon::Info => "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 11v5 M12 8h.01",
            Icon::ChevronDown => "M6 9l6 6 6-6",
            Icon::ChevronRight => "M9 6l6 6-6 6",
            Icon::ChevronLeft => "M15 6l-6 6 6 6",
            // GitHub 的标志（Feather 图标集，MIT）
            Icon::Github => {
                "M9 19 c-5 1.5 -5 -2.5 -7 -3 M16 22 v-3.87 a3.37 3.37 0 0 0 -0.94 -2.61 c3.14 -0.35 6.44 -1.54 6.44 -7 a5.44 5.44 0 0 0 -1.5 -3.75 a5.07 5.07 0 0 0 -0.09 -3.77 c0 0 -1.18 -0.35 -3.91 1.48 a13.38 13.38 0 0 0 -7 0 c-2.73 -1.83 -3.91 -1.48 -3.91 -1.48 a5.07 5.07 0 0 0 -0.09 3.77 a5.44 5.44 0 0 0 -1.5 3.78 c0 5.42 3.3 6.61 6.44 7 a3.37 3.37 0 0 0 -0.94 2.58 v3.87"
            }
        }
    }
}

/// 一个 `size` 大小的图标
pub fn icon(glyph: Icon, size: f32) -> Element {
    let shape = VectorPath::svg(glyph.data()).view_box(0.0, 0.0, 24.0, 24.0);
    path(shape)
        .size(size, size)
        .stroke(Stroke::new(1.6).round())
}
