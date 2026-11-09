//! 终端里的图片：iTerm2 的内联图片（`OSC 1337;File=`）、Kitty 的图片协议（`APC G`）、Sixel（`DCS q`）。
//!
//! 这一层只管「协议 → 一张图加它放在哪」：图片的像素（PNG、JPEG 这类编码好的，或者解好的 RGBA）和位置
//! （哪一行、哪一列、占几行几列）。真正画出来是界面层的事，仿真层不认识框架。
//!
//! 位置用行的编号记（见 [`super::marks`]）：图片跟着它所在的那一行一起滚进历史。

use std::collections::HashMap;
use std::sync::Arc;

use super::Terminal;

/// 一张图的像素
#[derive(Debug)]
pub enum Pixels {
    /// PNG、JPEG、GIF、WebP 这类编码好的，交给图片解码
    Encoded(Arc<[u8]>),
    /// 8 位 RGBA，一个像素四个字节，没有预乘
    Rgba(Arc<[u8]>),
}

#[derive(Debug)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

impl ImageData {
    fn bytes(&self) -> usize {
        match &self.pixels {
            Pixels::Encoded(bytes) | Pixels::Rgba(bytes) => bytes.len(),
        }
    }
}

/// 一张图放在屏幕上的位置
#[derive(Clone, Debug)]
pub struct Placement {
    pub id: u64,
    /// 左上角所在行的编号
    pub line: u64,
    pub column: usize,
    /// 占几列几行
    pub cols: usize,
    pub rows: usize,
    pub data: Arc<ImageData>,
}

/// 一张图的像素最多这么多（防止一张几万 × 几万的图撑爆内存）
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// 放着的图片加起来最多占这么多字节，再多挤掉最早的
const MAX_BYTES: usize = 128 * 1024 * 1024;
/// 同时放着最多这么多张
const MAX_PLACEMENTS: usize = 200;
/// 一张图最多占这么多行
const MAX_ROWS: usize = 400;

/// 要放多大：格子数、像素、百分比或者自动
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Extent {
    #[default]
    Auto,
    Cells(usize),
    Pixels(f32),
    /// 占终端宽（高）的百分之几
    Percent(f32),
}

/// Kitty 协议里攒了一半（分块传输）的一张图
#[derive(Debug, Default)]
pub struct KittyPending {
    control: HashMap<String, String>,
    data: Vec<u8>,
}

/// 图片相关的状态
#[derive(Debug, Default)]
pub struct Images {
    pub placements: Vec<Placement>,
    next_id: u64,
    /// 一格有多少逻辑像素（宽、高）：算图占几行几列用
    pub cell: (f32, f32),
    /// Kitty 传过来存着的图（按它的编号），`a=p` 时拿出来放
    kitty_stored: HashMap<u32, Arc<ImageData>>,
    kitty_pending: Option<KittyPending>,
}

impl Images {
    pub fn new() -> Self {
        Self {
            cell: (8.0, 16.0),
            ..Default::default()
        }
    }
}

impl Terminal {
    /// 告诉终端一格有多大（逻辑像素），图片按它换算成占几行几列
    pub fn set_cell_pixels(&mut self, width: f32, height: f32) {
        if width > 0.5 && height > 0.5 {
            self.images.cell = (width, height);
        }
    }

    pub fn placements(&self) -> &[Placement] {
        &self.images.placements
    }

    /// 去掉已经滚出历史的图
    pub fn prune_images(&mut self) {
        let alive: Vec<bool> = self
            .images
            .placements
            .iter()
            .map(|placement| {
                self.absolute_line(placement.line)
                    .is_some_and(|line| line + placement.rows > 0)
            })
            .collect();
        let mut keep = alive.into_iter();
        self.images.placements.retain(|_| keep.next().unwrap_or(false));
    }

    /// 清屏（`ED 2`、`ED 3`）、整机复位：图片一起清掉
    pub(super) fn clear_images(&mut self) {
        self.images.placements.clear();
    }

    /// 把一张图放在光标处，光标挪到图的下面。`move_cursor` 为假时光标不动（Kitty 的 `C=1`）
    fn place_image(
        &mut self,
        data: Arc<ImageData>,
        width: Extent,
        height: Extent,
        move_cursor: bool,
    ) {
        // 副屏（vim、less）里图片没法跟着内容滚，不支持
        if self.on_alternate || data.width == 0 || data.height == 0 {
            return;
        }
        let (columns, screen_rows) = self.size();
        let column = self.screen().cursor.column;
        let available = columns.saturating_sub(column).max(1);
        let (cell_width, cell_height) = self.images.cell;
        let (pixel_width, pixel_height) = (data.width as f32, data.height as f32);
        // 宽高都没指定：原始大小；只指定一边：另一边按宽高比算
        let ratio = pixel_height / pixel_width;
        let (mut cols, mut rows) = match (width, height) {
            (Extent::Auto, Extent::Auto) => (
                (pixel_width / cell_width).ceil().max(1.0),
                (pixel_height / cell_height).ceil().max(1.0),
            ),
            (width, Extent::Auto) => {
                let cols = resolve(width, cell_width, columns);
                (cols, (cols * cell_width * ratio / cell_height).ceil().max(1.0))
            }
            (Extent::Auto, height) => {
                let rows = resolve(height, cell_height, screen_rows);
                (
                    (rows * cell_height / ratio / cell_width).ceil().max(1.0),
                    rows,
                )
            }
            (width, height) => (
                resolve(width, cell_width, columns),
                resolve(height, cell_height, screen_rows),
            ),
        };
        // 比剩下的宽度还宽：等比缩小到放得下
        if cols > available as f32 {
            rows = (rows * available as f32 / cols).ceil().max(1.0);
            cols = available as f32;
        }
        let cols = (cols as usize).clamp(1, available);
        let rows = (rows as usize).clamp(1, MAX_ROWS);

        let placement = Placement {
            id: self.images.next_id,
            line: self.cursor_line_id_for_images(),
            column,
            cols,
            rows,
            data,
        };
        self.images.next_id += 1;
        self.images.placements.push(placement);
        self.trim_images();
        if move_cursor {
            // 图占了这些行：光标跳到图的下一行行首（滚动时图跟着行走）
            for _ in 0..rows {
                self.screen_mut().line_feed(1);
            }
            self.screen_mut().carriage_return();
        }
    }

    fn cursor_line_id_for_images(&self) -> u64 {
        self.primary.archived_total() + self.primary.cursor.row as u64
    }

    /// 放着的图太多（条数或者字节数）就挤掉最早的
    fn trim_images(&mut self) {
        let mut total: usize = self
            .images
            .placements
            .iter()
            .map(|placement| placement.data.bytes())
            .sum();
        while self.images.placements.len() > MAX_PLACEMENTS
            || (total > MAX_BYTES && self.images.placements.len() > 1)
        {
            let removed = self.images.placements.remove(0);
            total -= removed.data.bytes();
        }
    }

    // ---------- iTerm2：OSC 1337 ; File=参数:base64 ----------

    pub(super) fn iterm_image(&mut self, text: &str) {
        let Some(rest) = text.strip_prefix("File=") else {
            return;
        };
        let Some((arguments, payload)) = rest.split_once(':') else {
            return;
        };
        let mut inline = false;
        let (mut width, mut height) = (Extent::Auto, Extent::Auto);
        for argument in arguments.split(';') {
            let Some((name, value)) = argument.split_once('=') else {
                continue;
            };
            match name {
                "inline" => inline = value == "1",
                "width" => width = parse_extent(value),
                "height" => height = parse_extent(value),
                _ => {}
            }
        }
        // 不是内联的（是下载文件）不处理
        if !inline {
            return;
        }
        let Some(bytes) = decode_base64(payload.as_bytes()) else {
            return;
        };
        let Some((pixel_width, pixel_height)) = image_size(&bytes) else {
            return;
        };
        let data = Arc::new(ImageData {
            width: pixel_width,
            height: pixel_height,
            pixels: Pixels::Encoded(bytes.into()),
        });
        self.place_image(data, width, height, true);
    }

    // ---------- Sixel：DCS 参数 q 数据 ST ----------

    pub(super) fn dcs(&mut self, content: &[u8]) {
        // 参数是数字和分号，后面跟 `q` 才是 Sixel
        let Some(start) = content.iter().position(|byte| !byte.is_ascii_digit() && *byte != b';')
        else {
            return;
        };
        if content[start] != b'q' {
            return;
        }
        let Some((width, height, pixels)) = decode_sixel(&content[start + 1..]) else {
            return;
        };
        let data = Arc::new(ImageData {
            width,
            height,
            pixels: Pixels::Rgba(pixels.into()),
        });
        self.place_image(data, Extent::Auto, Extent::Auto, true);
    }

    // ---------- Kitty：APC G 控制;数据 ST ----------

    pub(super) fn apc(&mut self, content: &[u8]) {
        let Some(rest) = content.strip_prefix(b"G") else {
            return;
        };
        let (control_text, payload) = match rest.iter().position(|byte| *byte == b';') {
            Some(index) => (&rest[..index], &rest[index + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        let mut control = parse_control(control_text);
        // 分块传输：`m=1` 还有后续；后续的块只带 `m` 和数据，控制字段沿用第一块的
        let more = control.get("m").is_some_and(|value| value == "1");
        if let Some(mut pending) = self.images.kitty_pending.take() {
            pending.data.extend_from_slice(payload);
            if pending.data.len() > 48 * 1024 * 1024 {
                return;
            }
            if more {
                self.images.kitty_pending = Some(pending);
                return;
            }
            control = pending.control;
            self.kitty_command(&control, &pending.data);
            return;
        }
        if more {
            self.images.kitty_pending = Some(KittyPending {
                control,
                data: payload.to_vec(),
            });
            return;
        }
        self.kitty_command(&control, payload);
    }

    fn kitty_command(&mut self, control: &HashMap<String, String>, payload: &[u8]) {
        let number = |key: &str| control.get(key).and_then(|value| value.parse::<u32>().ok());
        let id = number("i").or_else(|| number("I"));
        let quiet = number("q").unwrap_or(0);
        let action = control.get("a").map_or("t", String::as_str);
        let reply = |terminal: &mut Terminal, result: Result<(), &str>| {
            let Some(id) = id else { return };
            match result {
                Ok(()) if quiet == 0 => terminal
                    .output
                    .extend_from_slice(format!("\x1b_Gi={id};OK\x1b\\").as_bytes()),
                Err(error) if quiet < 2 => terminal
                    .output
                    .extend_from_slice(format!("\x1b_Gi={id};{error}\x1b\\").as_bytes()),
                _ => {}
            }
        };
        match action {
            "d" => {
                match control.get("d").map_or("a", String::as_str) {
                    "a" | "A" => self.images.placements.clear(),
                    "i" | "I" => {
                        // 按图的编号删：放着的位置里我们没记 Kitty 的编号，只能清存着的
                        if let Some(id) = id {
                            self.images.kitty_stored.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
            "q" | "t" | "T" => {
                if control.get("o").is_some_and(|value| value == "z") {
                    reply(self, Err("EINVAL:压缩的数据不支持"));
                    return;
                }
                if control.get("t").is_some_and(|value| value != "d") {
                    reply(self, Err("EINVAL:只支持直接传输"));
                    return;
                }
                let Some(bytes) = decode_base64(payload) else {
                    reply(self, Err("EINVAL:数据不是 base64"));
                    return;
                };
                let Some(data) = kitty_image(control, bytes) else {
                    reply(self, Err("EINVAL:图片不能用"));
                    return;
                };
                let data = Arc::new(data);
                if action != "q" {
                    if let Some(id) = id {
                        self.images.kitty_stored.insert(id, data.clone());
                    }
                    if action == "T" {
                        let (width, height) = kitty_extent(control);
                        self.place_image(data, width, height, number("C") != Some(1));
                    }
                }
                reply(self, Ok(()));
            }
            "p" => {
                let Some(data) = id.and_then(|id| self.images.kitty_stored.get(&id).cloned())
                else {
                    reply(self, Err("ENOENT:没有这张图"));
                    return;
                };
                let (width, height) = kitty_extent(control);
                self.place_image(data, width, height, number("C") != Some(1));
                reply(self, Ok(()));
            }
            _ => {}
        }
    }
}

fn kitty_extent(control: &HashMap<String, String>) -> (Extent, Extent) {
    let cells = |key: &str| {
        control
            .get(key)
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|count| *count > 0)
            .map_or(Extent::Auto, Extent::Cells)
    };
    (cells("c"), cells("r"))
}

/// 把 Kitty 传来的字节变成图：`f=100` 是 PNG，`f=24`、`f=32` 是裸的 RGB、RGBA（要带 `s`、`v` 宽高）
fn kitty_image(control: &HashMap<String, String>, bytes: Vec<u8>) -> Option<ImageData> {
    let format = control.get("f").map_or("32", String::as_str);
    match format {
        "100" => {
            let (width, height) = image_size(&bytes)?;
            Some(ImageData {
                width,
                height,
                pixels: Pixels::Encoded(bytes.into()),
            })
        }
        "24" | "32" => {
            let width = control.get("s")?.parse::<u32>().ok()?;
            let height = control.get("v")?.parse::<u32>().ok()?;
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
                return None;
            }
            let count = width as usize * height as usize;
            let rgba = if format == "32" {
                if bytes.len() != count * 4 {
                    return None;
                }
                bytes
            } else {
                if bytes.len() != count * 3 {
                    return None;
                }
                let mut rgba = Vec::with_capacity(count * 4);
                for pixel in bytes.chunks_exact(3) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
                rgba
            };
            Some(ImageData {
                width,
                height,
                pixels: Pixels::Rgba(rgba.into()),
            })
        }
        _ => None,
    }
}

fn parse_control(text: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(text)
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// iTerm2 的宽高写法：`N` 是格数，`Npx` 像素，`N%` 百分比，`auto`
fn parse_extent(text: &str) -> Extent {
    if let Some(number) = text.strip_suffix("px") {
        return number.parse().map_or(Extent::Auto, Extent::Pixels);
    }
    if let Some(number) = text.strip_suffix('%') {
        return number.parse().map_or(Extent::Auto, Extent::Percent);
    }
    text.parse::<usize>().map_or(Extent::Auto, Extent::Cells)
}

/// 一个方向上放多大（格数）。`unit` 是一格多少像素，`total` 是这个方向一共多少格
fn resolve(extent: Extent, unit: f32, total: usize) -> f32 {
    match extent {
        Extent::Auto => 1.0,
        Extent::Cells(count) => count.max(1) as f32,
        Extent::Pixels(pixels) => (pixels / unit).ceil().max(1.0),
        Extent::Percent(percent) => (total as f32 * percent / 100.0).round().max(1.0),
    }
}

// ---------- base64 ----------

/// 标准 base64，空白忽略，末尾的 `=` 可有可无。写错了是 `None`
pub fn decode_base64(text: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in text {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

// ---------- 从文件头读图片的宽高 ----------

/// PNG、JPEG、GIF、WebP 的像素宽高（只读文件头，不解码）。认不出来、尺寸不合理是 `None`
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let (width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        (be32(16)?, be32(20)?)
    } else if bytes.starts_with(b"GIF8") {
        let le16 = |at: usize| Some(u32::from(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?)));
        (le16(6)?, le16(8)?)
    } else if bytes.starts_with(b"\xff\xd8") {
        jpeg_size(bytes)?
    } else if bytes.len() > 30 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp_size(bytes)?
    } else {
        return None;
    };
    (width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS)
        .then_some((width, height))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < bytes.len() {
        if bytes[at] != 0xff {
            at += 1;
            continue;
        }
        let marker = bytes[at + 1];
        // 没有长度的标记
        if marker == 0xff || marker == 0x01 || (0xd0..=0xd9).contains(&marker) {
            at += if marker == 0xff { 1 } else { 2 };
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        // SOF0..SOF15（除了 DHT、JPG、DAC）带着尺寸
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let height = u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]));
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            let width = u32::from(u16::from_le_bytes(bytes.get(26..28)?.try_into().ok()?) & 0x3fff);
            let height = u32::from(u16::from_le_bytes(bytes.get(28..30)?.try_into().ok()?) & 0x3fff);
            Some((width, height))
        }
        b"VP8L" => {
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => {
            let le24 = |at: usize| -> Option<u32> {
                let part = bytes.get(at..at + 3)?;
                Some(u32::from(part[0]) | u32::from(part[1]) << 8 | u32::from(part[2]) << 16)
            };
            Some((le24(24)? + 1, le24(27)? + 1))
        }
        _ => None,
    }
}

// ---------- Sixel ----------

/// VT340 的默认 16 色
const SIXEL_PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (51, 51, 204),
    (204, 33, 33),
    (51, 204, 51),
    (204, 51, 204),
    (51, 204, 204),
    (204, 204, 51),
    (135, 135, 135),
    (66, 66, 66),
    (84, 84, 153),
    (153, 66, 66),
    (84, 153, 84),
    (153, 84, 153),
    (84, 153, 153),
    (153, 153, 84),
    (204, 204, 204),
];

/// HLS（色相 0–360，亮度、饱和度 0–100）→ RGB
fn hls_to_rgb(hue: f32, lightness: f32, saturation: f32) -> (u8, u8, u8) {
    let (l, s) = (lightness / 100.0, saturation / 100.0);
    // Sixel 的色相 0 是蓝色，比通常的 HLS 转了 240 度
    let h = ((hue + 240.0) % 360.0) / 360.0;
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let value = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    if s == 0.0 {
        let gray = (l * 255.0).round() as u8;
        return (gray, gray, gray);
    }
    (channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// 解 Sixel 的数据（`q` 后面的部分）：宽、高、RGBA（没画到的像素是透明的）。出错、太大是 `None`
fn decode_sixel(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    const MAX_SIDE: usize = 8192;
    let mut palette = [(0u8, 0u8, 0u8); 256];
    palette[..16].copy_from_slice(&SIXEL_PALETTE);
    let (mut width, mut height) = (0usize, 0usize);
    // 每个像素一个 u32：0 是没画，否则是 0xFF000000 | RGB。按行存，行宽是 `stride`
    let mut stride = 0usize;
    let mut pixels: Vec<u32> = Vec::new();
    let (mut x, mut band) = (0usize, 0usize);
    let mut color = 0usize;
    let mut at = 0;
    let number = |at: &mut usize| -> Option<usize> {
        let start = *at;
        while *at < data.len() && data[*at].is_ascii_digit() {
            *at += 1;
        }
        (*at > start).then(|| std::str::from_utf8(&data[start..*at]).ok()?.parse().ok())?
    };
    while at < data.len() {
        let byte = data[at];
        at += 1;
        match byte {
            // 光栅属性 "Pan;Pad;Ph;Pv：只用来知道大小的下限，数据本身会撑开
            b'"' => {
                let mut values = Vec::new();
                loop {
                    match number(&mut at) {
                        Some(value) => values.push(value),
                        None => break,
                    }
                    if data.get(at) == Some(&b';') {
                        at += 1;
                    } else {
                        break;
                    }
                }
                if let (Some(&w), Some(&h)) = (values.get(2), values.get(3)) {
                    width = width.max(w.min(MAX_SIDE));
                    height = height.max(h.min(MAX_SIDE));
                }
            }
            b'#' => {
                let index = number(&mut at)?.min(255);
                if data.get(at) == Some(&b';') {
                    at += 1;
                    let kind = number(&mut at)?;
                    let mut parts = [0usize; 3];
                    for part in &mut parts {
                        if data.get(at) == Some(&b';') {
                            at += 1;
                        }
                        *part = number(&mut at).unwrap_or(0);
                    }
                    palette[index] = if kind == 2 {
                        let scale = |value: usize| (value.min(100) as f32 * 255.0 / 100.0).round() as u8;
                        (scale(parts[0]), scale(parts[1]), scale(parts[2]))
                    } else {
                        hls_to_rgb(parts[0] as f32, parts[1] as f32, parts[2] as f32)
                    };
                }
                color = index;
            }
            b'$' => x = 0,
            b'-' => {
                x = 0;
                band += 1;
            }
            b'!' | b'?'..=b'~' => {
                let (count, sixel) = if byte == b'!' {
                    let count = number(&mut at)?;
                    let sixel = *data.get(at)?;
                    at += 1;
                    (count, sixel)
                } else {
                    (1, byte)
                };
                if !(b'?'..=b'~').contains(&sixel) {
                    continue;
                }
                let bits = sixel - b'?';
                let (red, green, blue) = palette[color];
                let rgb = 0xff00_0000 | u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue);
                if x + count > MAX_SIDE || (band + 1) * 6 > MAX_SIDE {
                    return None;
                }
                // 宽度、高度不够就扩（先扩宽：每一行搬到新的行距里；再扩高：补零行）
                let needed_width = x + count;
                let needed_height = (band + 1) * 6;
                if needed_width > stride {
                    let rows = if stride == 0 { 0 } else { pixels.len() / stride };
                    let mut grown = vec![0u32; needed_width * rows];
                    for row in 0..rows {
                        grown[row * needed_width..row * needed_width + stride]
                            .copy_from_slice(&pixels[row * stride..(row + 1) * stride]);
                    }
                    pixels = grown;
                    stride = needed_width;
                }
                if pixels.len() / stride < needed_height {
                    pixels.resize(stride * needed_height, 0);
                }
                for bit in 0..6 {
                    if bits & (1 << bit) != 0 {
                        let row = band * 6 + bit;
                        for column in x..x + count {
                            pixels[row * stride + column] = rgb;
                        }
                    }
                }
                x += count;
                width = width.max(x);
                height = height.max(band * 6 + 6);
            }
            _ => {}
        }
    }
    if width == 0 || height == 0 || stride == 0 {
        return None;
    }
    let (width, height) = (width.min(stride), height.min(pixels.len() / stride));
    if (width * height) as u64 > MAX_PIXELS {
        return None;
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        for column in 0..width {
            let pixel = pixels[row * stride + column];
            if pixel == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                rgba.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8, 255]);
            }
        }
    }
    Some((width as u32, height as u32, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let value = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (index, byte)| acc | u32::from(*byte) << (16 - 8 * index));
            for index in 0..4 {
                if index <= chunk.len() {
                    text.push(TABLE[(value >> (18 - 6 * index) & 63) as usize] as char);
                } else {
                    text.push('=');
                }
            }
        }
        text
    }

    /// 只有文件头的 PNG：够读出宽高
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn base64_roundtrip_and_errors() {
        for sample in [&b""[..], b"a", b"ab", b"abc", b"hello world!", &[0, 255, 128, 7]] {
            assert_eq!(decode_base64(base64(sample).as_bytes()).as_deref(), Some(sample));
        }
        assert_eq!(decode_base64(b"aGVs\nbG8=").as_deref(), Some(&b"hello"[..]), "空白忽略");
        assert!(decode_base64(b"a$b").is_none());
    }

    #[test]
    fn reads_sizes_from_file_headers() {
        assert_eq!(image_size(&png_header(640, 480)), Some((640, 480)));
        assert_eq!(image_size(b"GIF89a\x10\x00\x20\x00rest"), Some((16, 32)));
        // JPEG：SOI，一个 APP0（长度 4），然后 SOF0（高 100，宽 200）
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0, 0xff, 0xc0, 0x00, 0x0b, 8, 0x00, 100, 0x00, 200, 1, 1, 0x11,
            0,
        ];
        assert_eq!(image_size(&jpeg), Some((200, 100)));
        assert_eq!(image_size(b"not an image at all, just text"), None);
        assert_eq!(image_size(&png_header(0, 5)), None, "零尺寸");
        assert_eq!(image_size(&png_header(100_000, 100_000)), None, "大得离谱");
    }

    fn terminal() -> Terminal {
        let mut terminal = Terminal::new(40, 10);
        terminal.set_cell_pixels(10.0, 20.0);
        terminal
    }

    /// iTerm2：宽高按格数、像素、百分比、自动；光标挪到图下面的行首
    #[test]
    fn iterm_inline_images() {
        let mut term = terminal();
        let png = base64(&png_header(100, 40));
        term.feed(format!("\x1b]1337;File=inline=1;width=10;height=3:{png}\x07").as_bytes());
        let placement = &term.placements()[0];
        assert_eq!((placement.cols, placement.rows), (10, 3));
        assert_eq!((placement.column, term.screen().cursor.row, term.screen().cursor.column), (0, 3, 0));
        // 只给宽：高按宽高比算（20 列 = 200px 宽，高 80px = 4 行）
        term.feed(format!("\x1b]1337;File=inline=1;width=20:{png}\x07").as_bytes());
        assert_eq!((term.placements()[1].cols, term.placements()[1].rows), (20, 4));
        // 都不给：原始大小，100×40 px = 10 × 2 格
        term.feed(format!("\x1b]1337;File=inline=1:{png}\x07").as_bytes());
        assert_eq!((term.placements()[2].cols, term.placements()[2].rows), (10, 2));
        // 百分比、像素
        term.feed(format!("\x1b]1337;File=inline=1;width=50%;height=60px:{png}\x07").as_bytes());
        assert_eq!((term.placements()[3].cols, term.placements()[3].rows), (20, 3));
        // 不是内联的（下载）、不是图片、base64 坏了：不放
        let before = term.placements().len();
        term.feed(format!("\x1b]1337;File=name=a.png:{png}\x07").as_bytes());
        term.feed(b"\x1b]1337;File=inline=1:aGVsbG8=\x07");
        term.feed(b"\x1b]1337;File=inline=1:@@@\x07");
        assert_eq!(term.placements().len(), before);
    }

    /// 比剩下的宽度还宽的图等比缩小；占的行数封顶
    #[test]
    fn wide_images_shrink_to_fit() {
        let mut term = terminal();
        let png = base64(&png_header(1000, 200));
        term.feed(format!("\x1b]1337;File=inline=1:{png}\x07").as_bytes());
        let placement = &term.placements()[0];
        assert_eq!(placement.cols, 40, "不超过终端宽");
        assert_eq!(placement.rows, 4, "高跟着等比缩小（原来 10 行，宽缩到 40/100，高也是 4 行）");
    }

    /// 图片跟着它所在的行滚进历史；滚出历史（挤掉了）就清理；清屏时一起清
    #[test]
    fn images_scroll_with_their_lines_and_clear_with_the_screen() {
        let mut term = Terminal::new(20, 5);
        term.set_cell_pixels(10.0, 20.0);
        let png = base64(&png_header(20, 20));
        term.feed(format!("top\r\n\x1b]1337;File=inline=1:{png}\x07").as_bytes());
        let line = term.placements()[0].line;
        let before = term.absolute_line(line).expect("在");
        term.feed(b"1\r\n2\r\n3\r\n4\r\n5\r\n");
        assert!(term.absolute_line(line).is_some());
        assert_eq!(term.absolute_line(line), Some(before), "滚了以后编号还对应同一行");
        term.prune_images();
        assert_eq!(term.placements().len(), 1);
        term.feed(b"\x1b[2J");
        assert!(term.placements().is_empty(), "清屏清掉图片");
        // 副屏里不放
        term.feed(b"\x1b[?1049h");
        term.feed(format!("\x1b]1337;File=inline=1:{png}\x07").as_bytes());
        assert!(term.placements().is_empty());
    }

    #[test]
    fn kitty_direct_transmission_replies_and_deletes() {
        let mut term = terminal();
        let pixels = base64(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
        // a=T：传输并显示；带编号时回 OK
        term.feed(format!("\x1b_Ga=T,f=32,s=2,v=2,i=7;{pixels}\x1b\\").as_bytes());
        assert_eq!(term.placements().len(), 1);
        assert_eq!((term.placements()[0].data.width, term.placements()[0].data.height), (2, 2));
        assert_eq!(term.take_output(), b"\x1b_Gi=7;OK\x1b\\");
        // 不带编号：不回
        term.feed(format!("\x1b_Ga=T,f=32,s=2,v=2;{pixels}\x1b\\").as_bytes());
        assert!(term.take_output().is_empty());
        // q=2：什么都不回
        term.feed(format!("\x1b_Ga=T,f=32,s=2,v=2,i=8,q=2;{pixels}\x1b\\").as_bytes());
        assert!(term.take_output().is_empty());
        // 先传后放（a=t 然后 a=p），按列数放
        term.feed(format!("\x1b_Ga=t,f=32,s=2,v=2,i=9;{pixels}\x1b\\").as_bytes());
        assert_eq!(term.placements().len(), 3, "a=t 只存不放");
        term.take_output();
        term.feed(b"\x1b_Ga=p,i=9,c=6,r=2\x1b\\");
        assert_eq!((term.placements()[3].cols, term.placements()[3].rows), (6, 2));
        // 放不存在的：报错
        term.feed(b"\x1b_Ga=p,i=99\x1b\\");
        assert!(String::from_utf8_lossy(&term.take_output()).contains("ENOENT"));
        // 删光
        term.feed(b"\x1b_Ga=d,d=a\x1b\\");
        assert!(term.placements().is_empty());
    }

    #[test]
    fn kitty_chunks_png_and_bad_input() {
        let mut term = terminal();
        let png = base64(&png_header(30, 40));
        let (first, second) = png.split_at(8);
        term.feed(format!("\x1b_Ga=T,f=100,i=1,m=1;{first}\x1b\\").as_bytes());
        assert!(term.placements().is_empty(), "还没传完");
        term.feed(format!("\x1b_Gm=0;{second}\x1b\\").as_bytes());
        assert_eq!(term.placements().len(), 1);
        assert_eq!((term.placements()[0].cols, term.placements()[0].rows), (3, 2));
        term.take_output();
        // C=1：放了以后光标不动
        let row = term.screen().cursor.row;
        term.feed(format!("\x1b_Ga=T,f=100,C=1;{png}\x1b\\").as_bytes());
        assert_eq!(term.screen().cursor.row, row);
        // 压缩的、长度对不上的、不认识的格式：不放，带编号时报错
        term.feed(b"\x1b_Ga=T,f=32,s=2,v=2,i=3,o=z;AAAA\x1b\\");
        assert!(String::from_utf8_lossy(&term.take_output()).contains("EINVAL"));
        term.feed(b"\x1b_Ga=T,f=32,s=2,v=2,i=4;AAAA\x1b\\");
        assert!(String::from_utf8_lossy(&term.take_output()).contains("EINVAL"));
        assert_eq!(term.placements().len(), 2);
        // 查询（kitten icat 先问一声）：能用就回 OK，不放
        term.feed(format!("\x1b_Ga=q,f=100,i=31;{png}\x1b\\").as_bytes());
        assert_eq!(term.take_output(), b"\x1b_Gi=31;OK\x1b\\");
        assert_eq!(term.placements().len(), 2);
    }

    #[test]
    fn sixel_decodes_colors_and_repeats() {
        let mut term = terminal();
        // 红色（#1 = RGB 100,0,0），一条 4 宽的竖条（~ 是六个点全亮），再换行，蓝色 2 宽
        term.feed(b"\x1bPq#1;2;100;0;0#2;2;0;0;100#1!4~-#2!2~\x1b\\");
        let placement = &term.placements()[0];
        let Pixels::Rgba(pixels) = &placement.data.pixels else {
            panic!("Sixel 解出来是 RGBA");
        };
        assert_eq!((placement.data.width, placement.data.height), (4, 12));
        let at = |x: usize, y: usize| pixels[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4].to_vec();
        assert_eq!(at(0, 0), [255, 0, 0, 255]);
        assert_eq!(at(3, 5), [255, 0, 0, 255]);
        assert_eq!(at(1, 6), [0, 0, 255, 255]);
        assert_eq!(at(3, 6), [0, 0, 0, 0], "没画到的是透明的");
        // 数据里什么都没画：不放
        let before = term.placements().len();
        term.feed(b"\x1bPq\x1b\\");
        assert_eq!(term.placements().len(), before);
        // 别的 DCS 不当 Sixel
        term.feed(b"\x1bP1$rabc\x1b\\");
        assert_eq!(term.placements().len(), before);
    }

    #[test]
    fn too_many_images_evict_the_oldest() {
        let mut term = Terminal::new(40, 5);
        let png = base64(&png_header(10, 10));
        for _ in 0..(MAX_PLACEMENTS + 5) {
            term.feed(format!("\x1b]1337;File=inline=1:{png}\x07").as_bytes());
        }
        assert_eq!(term.placements().len(), MAX_PLACEMENTS);
    }
}
