//! 图片的编码：base64，和从文件头读出 PNG、JPEG、GIF、WebP 的宽高（不解码）。

use super::MAX_PIXELS;

/// 标准 base64，空白忽略，末尾的 `=` 可有可无。写错了是 `None`
pub(super) fn decode_base64(text: &[u8]) -> Option<Vec<u8>> {
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

/// PNG、JPEG、GIF、WebP 的像素宽高（只读文件头，不解码）。认不出来、尺寸不合理是 `None`
pub(super) fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let (width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        (be32(16)?, be32(20)?)
    } else if bytes.starts_with(b"GIF8") {
        let le16 = |at: usize| {
            Some(u32::from(u16::from_le_bytes(
                bytes.get(at..at + 2)?.try_into().ok()?,
            )))
        };
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
            let height =
                u32::from(u16::from_le_bytes(bytes.get(28..30)?.try_into().ok()?) & 0x3fff);
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
