//! 内联图片。

use super::*;

/// 内联图片叠在网格上：位置和大小按格子算，跟着行滚；滚出屏幕就撤下来，回看时又出现
#[test]
fn inline_images_sit_on_their_cells_and_scroll_with_them() {
    // 1×1 的 PNG：真能解码
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let setup = format!("line0\r\n\x1b]1337;File=inline=1;width=10;height=3:{PNG}\x07done\r\n");
    let (harness, _recorder, grid, cell, session) =
        mounted_with(&setup, crate::settings::Settings::default());
    harness.frame();
    let image = harness
        .window_bounds("terminal-image-0")
        .expect("图片在元素树里");
    assert!((image.x - grid.x).abs() < 0.5, "左对齐网格");
    assert!(
        (image.y - (grid.y + cell.1)).abs() < 0.5,
        "在第 1 行（第 0 行写了 line0）"
    );
    assert!((image.width - 10.0 * cell.0).abs() < 0.5);
    assert!((image.height - 3.0 * cell.1).abs() < 0.5);
    // 再写够把屏幕写满再多一行：整屏往上滚一行，图跟着往上一格
    let screen_rows = session.grid_size_for_test()[1];
    session.feed_for_test(&lines(screen_rows - 5));
    harness.frame();
    let scrolled = harness
        .window_bounds("terminal-image-0")
        .expect("还在屏幕上")
        .y;
    assert!((image.y - scrolled - cell.1).abs() < 0.5, "往上移了一格");
    // 滚到屏幕外：撤下来；往回翻又回来
    session.feed_for_test(&lines(40));
    harness.frame();
    assert!(
        harness.window_bounds("terminal-image-0").is_none(),
        "滚出屏幕了"
    );
    session.scroll_back(1000);
    harness.frame();
    assert!(
        harness.window_bounds("terminal-image-0").is_some(),
        "往回翻看得见"
    );
    // 清屏：图片清掉
    session.snap_to_bottom();
    session.feed_for_test("\x1b[2J");
    harness.frame();
    assert!(harness.window_bounds("terminal-image-0").is_none());
}
