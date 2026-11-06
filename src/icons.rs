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
    Bell,
    Laptop,
    Server,
    Cube,
    Folder,
    Sliders,
    Activity,
    Columns,
    Rows,
    Prompt,
}

impl Icon {
    fn data(self) -> &'static str {
        match self {
            Icon::Plus => "M12 5v14 M5 12h14",
            Icon::Close => "M6 6l12 12 M18 6L6 18",
            Icon::Search => "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z M20 20l-4-4",
            Icon::Terminal => "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z M7.5 10l3 2-3 2 M13 15h4",
            Icon::Sidebar => "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M9 4v16",
            Icon::Bell => "M6 17v-6a6 6 0 0 1 12 0v6l2 2H4z M10 21a2 2 0 0 0 4 0",
            Icon::Laptop => "M5 6h14a1 1 0 0 1 1 1v9H4V7a1 1 0 0 1 1-1z M2.5 19h19",
            Icon::Server => "M5 4h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z M5 13h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1z M8 7.5h.01 M8 16.5h.01",
            Icon::Cube => "M12 3l8 4.5v9L12 21l-8-4.5v-9z M4 7.5l8 4.5 8-4.5 M12 12v9",
            Icon::Folder => "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
            Icon::Sliders => "M4 7h9 M17 7h3 M4 17h3 M11 17h9 M13 7a2 2 0 1 0 4 0a2 2 0 1 0-4 0 M7 17a2 2 0 1 0 4 0a2 2 0 1 0-4 0",
            Icon::Activity => "M3 12h4l3-8 4 16 3-8h4",
            Icon::Columns => "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M12 4v16",
            Icon::Rows => "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M3 12h18",
            Icon::Prompt => "M5 8l4 4-4 4 M12 16h7",
        }
    }
}

/// 一个 `size` 大小的图标
pub fn icon(glyph: Icon, size: f32) -> Element {
    let shape = VectorPath::svg(glyph.data()).view_box(0.0, 0.0, 24.0, 24.0);
    path(shape).size(size, size).stroke(Stroke::new(1.6).round())
}
