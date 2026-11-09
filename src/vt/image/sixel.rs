//! Sixel：把 `DCS q …` 里的数据解成一张 RGBA 的图。

use super::MAX_PIXELS;

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
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
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
pub(super) fn decode_sixel(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
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
                while let Some(value) = number(&mut at) {
                    values.push(value);
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
                        let scale =
                            |value: usize| (value.min(100) as f32 * 255.0 / 100.0).round() as u8;
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
                let rgb =
                    0xff00_0000 | u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue);
                if x + count > MAX_SIDE || (band + 1) * 6 > MAX_SIDE {
                    return None;
                }
                // 宽度、高度不够就扩（先扩宽：每一行搬到新的行距里；再扩高：补零行）
                let needed_width = x + count;
                let needed_height = (band + 1) * 6;
                if needed_width > stride {
                    let rows = pixels.len().checked_div(stride).unwrap_or(0);
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
                rgba.extend_from_slice(&[
                    (pixel >> 16) as u8,
                    (pixel >> 8) as u8,
                    pixel as u8,
                    255,
                ]);
            }
        }
    }
    Some((width as u32, height as u32, rgba))
}
