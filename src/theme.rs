//! 界面的颜色。浅色，中性的灰白底，整个界面没有彩色：层次靠几档灰和大面积的留白，
//! 选中、悬停都用「墨色叠一层透明度」，在任何底上都是同一个灰度关系。

use aureole::prelude::*;

/// 墨色：文字、投影、悬停和选中的底色都从它来
pub const INK: Color = Color::hex(0x1f2328);

/// 侧栏和标题栏的底：比卡片稍深一点的灰白
pub const SIDEBAR_BG: Color = Color::hex(0xf3f3f4);
/// 主卡片（终端所在的那块）：白
pub const CARD_BG: Color = Color::hex(0xffffff);
/// 细线、卡片边框
pub const LINE: Color = Color::hex(0xe3e4e6);

pub const TEXT: Color = INK;
pub const TEXT_2: Color = Color::hex(0x555b62);
pub const TEXT_3: Color = Color::hex(0x838990);

/// 小标记（后台有新输出的圆点、角标）：深灰，不抢眼但看得清
pub const MARK: Color = Color::hex(0x555b62);

/// 卡片浮在底色上的投影：一层贴得近的接触阴影，一层铺得开的柔和阴影。
/// 用投影表现「浮起来」，不用边框线
pub fn card_shadow() -> Shadows {
    Shadows::new([Shadow::new(0.0, 1.0, 2.0, INK.with_alpha(0.07)), Shadow::new(0.0, 10.0, 30.0, INK.with_alpha(0.10)).spread(-4.0)])
}

/// 浮在最上面的面板（命令面板）的投影：比卡片更深更开
pub fn popup_shadow() -> Shadows {
    Shadows::new([Shadow::new(0.0, 2.0, 6.0, INK.with_alpha(0.08)), Shadow::new(0.0, 18.0, 48.0, INK.with_alpha(0.18)).spread(-6.0)])
}

/// 悬停时的一层灰：要看得出来，所以比「淡淡一层」深
pub fn hover() -> Color {
    INK.with_alpha(0.10)
}

/// 选中项的底：比悬停再深一档，悬停和选中能分得开
pub fn selected() -> Color {
    INK.with_alpha(0.16)
}
