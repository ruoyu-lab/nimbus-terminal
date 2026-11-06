//! 界面的颜色，浅色和深色两套。中性的灰阶，整个界面没有彩色：层次靠几档灰和大面积的留白，
//! 选中、悬停都用「前景色叠一层透明度」，在任何底上都是同一个灰度关系。
//!
//! 终端自己的颜色跟着这里走（`view::colors`）：界面是深色，终端就是深色，不单独配。
//!
//! 现在是深色还是浅色存在 [`dark`] 这个信号里（设置里的「外观」、系统的外观决定它）。
//! 颜色都是函数，调用时按当时的深浅取值；界面在深浅变了的时候整个重新建一遍（见 `chrome::build`），
//! 所以建元素时直接调用就行，不用每处都包成响应式的闭包。

use std::cell::Cell;
use std::time::{Duration, Instant};

use aureole::prelude::*;

/// 浅色的墨色：文字、投影、悬停和选中的底色都从它来
pub const LIGHT_INK: Color = Color::hex(0x1f2328);
/// 深色的「墨色」：浅灰的字
pub const DARK_INK: Color = Color::hex(0xdde0e6);
/// 浅色的主卡片（终端所在的那块）：白
pub const LIGHT_CARD: Color = Color::hex(0xffffff);
/// 深色的主卡片
pub const DARK_CARD: Color = Color::hex(0x1a1d23);

thread_local! {
    static DARK: Cell<Option<Signal<bool>>> = const { Cell::new(None) };
}

/// 现在是不是深色。信号不挂在任何界面上：窗口里的东西卸载不能把它释放掉
pub fn dark() -> Signal<bool> {
    DARK.with(|slot| {
        if let Some(signal) = slot.get() {
            return signal;
        }
        let created = aureole::detached(|| signal(false));
        slot.set(Some(created));
        created
    })
}

thread_local! {
    /// 最近一次深浅色真的变了是什么时候
    static SWITCHED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// 换深浅色。真的变了才算，并记下时间（见 [`just_switched`]）。第一次定下来（启动时）不算「换」
pub fn set_dark(value: bool) {
    let initial = DARK.with(|slot| slot.get().is_none());
    let signal = dark();
    if !initial && signal.get_untracked() != value {
        SWITCHED.with(|at| at.set(Some(Instant::now())));
    }
    signal.set(value);
}

/// 刚换过深浅色：界面整个重建了，这时挂上的东西是「换了身衣服」，不是新出现的，不该再播入场动画
pub fn just_switched() -> bool {
    SWITCHED.with(|at| {
        at.get()
            .is_some_and(|at| at.elapsed() < Duration::from_millis(500))
    })
}

/// 现在是深色吗（在响应式的地方读会跟着变）
pub fn is_dark() -> bool {
    dark().get()
}

fn pick(light: Color, dark_color: Color) -> Color {
    if is_dark() { dark_color } else { light }
}

/// 墨色：文字、悬停和选中的底色
pub fn ink() -> Color {
    pick(LIGHT_INK, DARK_INK)
}

/// 侧栏和标题栏的底：比卡片稍深（浅色）或稍浅（深色）的一档
pub fn sidebar_bg() -> Color {
    pick(Color::hex(0xf3f3f4), Color::hex(0x1e2025))
}

/// 主卡片（终端所在的那块）
pub fn card_bg() -> Color {
    pick(LIGHT_CARD, DARK_CARD)
}

/// 细线、卡片边框
pub fn line() -> Color {
    pick(Color::hex(0xe3e4e6), Color::hex(0x2d3037))
}

/// 选中的标签的底：浅色里和下面的卡片一样是白的，深色里比标签条和卡片都亮一档，
/// 不然选中的标签比标签条还暗，像开了个洞，分不出哪个是当前的
pub fn tab_selected_bg() -> Color {
    pick(LIGHT_CARD, Color::hex(0x394049))
}

/// 正文
pub fn fg() -> Color {
    ink()
}

pub fn text_2() -> Color {
    pick(Color::hex(0x555b62), Color::hex(0xb3b9c1))
}

pub fn text_3() -> Color {
    pick(Color::hex(0x838990), Color::hex(0x8a9099))
}

/// 小标记（后台有新输出的圆点、角标）：不抢眼但看得清
pub fn mark() -> Color {
    text_2()
}

/// 开关打开时、主按钮的底：浅色里是墨色，深色里是一档中灰（浅灰底配白色的滑块看不清）
pub fn accent() -> Color {
    pick(LIGHT_INK, Color::hex(0x7c8696))
}

/// 开关关着时的槽：深色里是一档深灰（框架默认的浅灰槽配白色滑块在深底上整个发白，看不出是开是关）
pub fn toggle_off() -> Color {
    pick(Color::hex(0xd5dae0), Color::hex(0x3a3f48))
}

/// 模态框后面压暗的一层
pub fn scrim() -> Color {
    pick(LIGHT_INK.with_alpha(0.14), Color::BLACK.with_alpha(0.45))
}

fn shadow_color() -> Color {
    if is_dark() { Color::BLACK } else { LIGHT_INK }
}

/// 卡片浮在底色上的投影：一层贴得近的接触阴影，一层铺得开的柔和阴影。
/// 用投影表现「浮起来」，不用边框线
pub fn card_shadow() -> Shadows {
    let (near, far) = if is_dark() {
        (0.35, 0.45)
    } else {
        (0.07, 0.10)
    };
    Shadows::new([
        Shadow::new(0.0, 1.0, 2.0, shadow_color().with_alpha(near)),
        Shadow::new(0.0, 10.0, 30.0, shadow_color().with_alpha(far)).spread(-4.0),
    ])
}

/// 浮在最上面的面板（命令面板）的投影：比卡片更深更开
pub fn popup_shadow() -> Shadows {
    let (near, far) = if is_dark() { (0.4, 0.6) } else { (0.08, 0.18) };
    Shadows::new([
        Shadow::new(0.0, 2.0, 6.0, shadow_color().with_alpha(near)),
        Shadow::new(0.0, 18.0, 48.0, shadow_color().with_alpha(far)).spread(-6.0),
    ])
}

/// 悬停时的一层灰：要看得出来，所以比「淡淡一层」深
pub fn hover() -> Color {
    ink().with_alpha(if is_dark() { 0.07 } else { 0.10 })
}

/// 选中项的底：比悬停再深一档，悬停和选中能分得开
pub fn selected() -> Color {
    ink().with_alpha(if is_dark() { 0.12 } else { 0.16 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 的对比度：两个颜色亮度之比，越大越清楚（正文至少 4.5，好读的 7 以上）
    fn contrast(a: Color, b: Color) -> f32 {
        let luminance = |color: Color| {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.03928 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.0[0])
                + 0.7152 * channel(color.0[1])
                + 0.0722 * channel(color.0[2])
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// 深色里字、标签、终端的颜色都读得清：这是个下限，防止以后调色调回难读的
    #[test]
    fn the_dark_theme_is_readable() {
        set_dark(true);
        let (card, bar) = (card_bg(), sidebar_bg());
        assert!(
            contrast(fg(), card) >= 11.0,
            "正文和终端：{}",
            contrast(fg(), card)
        );
        assert!(
            contrast(text_2(), bar) >= 7.0,
            "标签条上没选中的标签：{}",
            contrast(text_2(), bar)
        );
        assert!(
            contrast(text_3(), bar) >= 4.5,
            "标签条上的次要文字：{}",
            contrast(text_3(), bar)
        );
        assert!(
            contrast(text_3(), card) >= 4.5,
            "卡片里的次要文字：{}",
            contrast(text_3(), card)
        );
        assert!(contrast(fg(), tab_selected_bg()) >= 7.0, "选中的标签上的字");
        assert!(
            contrast(tab_selected_bg(), bar) >= 1.4,
            "选中的标签要和标签条分得开：{}",
            contrast(tab_selected_bg(), bar)
        );
        // 终端的 16 色：除了黑（本来就是深色），都能读
        let colors = crate::view::colors(true);
        for index in 1..16 {
            let ratio = contrast(colors.ansi(index), colors.background);
            let needed = if index == 8 { 4.5 } else { 6.0 };
            assert!(ratio >= needed, "ANSI {index}：{ratio}");
        }
        assert!(
            contrast(colors.ansi(0), colors.background) >= 2.0,
            "黑也要看得见"
        );
        set_dark(false);
        assert!(
            contrast(fg(), card_bg()) >= 12.0,
            "浅色：{}",
            contrast(fg(), card_bg())
        );
        assert!(contrast(text_3(), card_bg()) >= 3.0);
    }
}
