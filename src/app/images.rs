//! 终端里的图片：哪些图现在看得见（跟着行滚动），和叠在网格上的那一层。协议的解析在 `vt::image`。

use super::*;

/// 屏幕上现在看得见的一张图：在终端区域里的位置和大小（逻辑像素），和它的像素来源
#[derive(Clone)]
pub struct VisibleImage {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    source: ImageSource,
}

impl PartialEq for VisibleImage {
    /// 来源按图的编号认：同一张图只要位置、大小没变就是没变
    fn eq(&self, other: &Self) -> bool {
        (self.id, self.x, self.y, self.width, self.height)
            == (other.id, other.x, other.y, other.width, other.height)
    }
}

impl VisibleImage {
    pub fn source(&self) -> ImageSource {
        self.source.clone()
    }
}

/// 叠在网格上的一张图：按终端算好的位置和大小摆，位置变了（滚动）跟着变
pub(super) fn terminal_image(images: Signal<Vec<VisibleImage>>, id: u64) -> Element {
    let field = move |read: fn(&VisibleImage) -> f32| {
        move || {
            images
                .get()
                .iter()
                .find(|image| image.id == id)
                .map_or(0.0, read)
        }
    };
    let source = images
        .get_untracked()
        .into_iter()
        .find(|image| image.id == id)
        .map(|image| image.source());
    match source {
        Some(source) => image(source)
            .name(format!("terminal-image-{id}"))
            .fit(ImageFit::Stretch)
            .position(field(|image| image.x), field(|image| image.y))
            .width(field(|image| image.width))
            .height(field(|image| image.height)),
        None => column(),
    }
}

impl Session {
    /// 看得见的图片有变化就更新界面用的那份清单
    pub(super) fn refresh_images(&self, offset: usize) {
        let (cell_width, cell_height) = self.cell.get().unwrap_or((8.0, 16.0));
        let visible: Vec<VisibleImage> = {
            let mut terminal = self.terminal.borrow_mut();
            if terminal.placements().is_empty() {
                Vec::new()
            } else {
                terminal.prune_images();
                let history = terminal.scrollback_len();
                let top = history.saturating_sub(offset) as isize;
                let rows = terminal.size().1 as isize;
                let mut sources = self.image_sources.borrow_mut();
                let alive: std::collections::HashSet<u64> = terminal
                    .placements()
                    .iter()
                    .map(|placement| placement.id)
                    .collect();
                sources.retain(|id, _| alive.contains(id));
                let mut visible = Vec::new();
                for placement in terminal.placements() {
                    let Some(line) = terminal.absolute_line(placement.line) else {
                        continue;
                    };
                    let row = line as isize - top;
                    if terminal.on_alternate || row + placement.rows as isize <= 0 || row >= rows {
                        continue;
                    }
                    let source = sources
                        .entry(placement.id)
                        .or_insert_with(|| match &placement.data.pixels {
                            Pixels::Encoded(bytes) => ImageSource::bytes(bytes.clone()),
                            Pixels::Rgba(bytes) => ImageSource::rgba(
                                placement.data.width,
                                placement.data.height,
                                bytes.clone(),
                            ),
                        })
                        .clone();
                    visible.push(VisibleImage {
                        id: placement.id,
                        x: placement.column as f32 * cell_width,
                        y: row as f32 * cell_height,
                        width: placement.cols as f32 * cell_width,
                        height: placement.rows as f32 * cell_height,
                        source,
                    });
                }
                visible
            }
        };
        if self.images.get_untracked() != visible {
            self.images.set(visible);
        }
    }
}
