//! 编译脚本：Windows（MSVC）上把图标和包标识符编进可执行文件。
//!
//! 框架建窗口时从可执行文件里读编号 1 的图标组（标题栏、任务栏显示它），启动时读名为 `AUREOLE_APP_ID` 的包标识符、
//! 声明成进程的应用标识（AppUserModelID：任务栏分组、系统通知认它）。框架的打包工具（`cargo bundle`）在打包时才写这两样；
//! 这里编译时就写好，`cargo run`、`cargo build --release` 出来的程序也有图标和应用标识。打包时打包工具按同样的类型、
//! 编号、语言再写一遍，盖掉这里的（图标的尺寸也和它的一样，不会留下多出来的）。
//!
//! 不用资源编译器（`rc.exe`）：资源文件（.res）的格式很简单，这里直接拼出来，链接器（`link.exe`、`lld-link`）直接收。
//! GNU 工具链的链接器不收 .res，那里不嵌（框架照样能跑，用系统默认的图标）。
//!
//! 图标 `assets/icon.ico` 是从 `assets/icon-windows.png` 缩出来的（16、24、32、48、64、256，每张一个 PNG），
//! 换了图标要两个一起换。

use std::path::PathBuf;

/// 图标
const ICON: &str = "assets/icon.ico";
/// 包标识符资源的名字：框架运行时按这个名字读（`aureole_platform::system::APP_ID_RESOURCE`），打包工具写的也是它
const APP_ID_RESOURCE: &str = "AUREOLE_APP_ID";
/// 资源的语言：不分语言，和打包工具写的一样（同一种类型、编号、语言，打包时才是替换而不是多一份）
const LANGUAGE: u16 = 0;
/// 资源类型：图标的一张、图标组、原始数据
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_RCDATA: u16 = 10;
/// 框架按这个编号读图标组
const ICON_GROUP: u16 = 1;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed={ICON}");
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") != "windows" || target("CARGO_CFG_TARGET_ENV") != "msvc" {
        return;
    }
    let package = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo 没给 CARGO_MANIFEST_DIR"),
    );
    let manifest = std::fs::read_to_string(package.join("Cargo.toml")).expect("读不了 Cargo.toml");
    let identifier = bundle_setting(&manifest, "identifier")
        .expect("Cargo.toml 的 [package.metadata.bundle] 里要写 identifier");
    let icon =
        std::fs::read(package.join(ICON)).unwrap_or_else(|error| panic!("读不了 {ICON}：{error}"));
    let images = icon_images(&icon);

    let mut res = Vec::new();
    // 第一项是空的：.res 文件的约定，链接器靠它认出格式
    resource(&mut res, Id::Number(0), Id::Number(0), 0, 0, &[]);
    for (index, image) in images.iter().enumerate() {
        resource(
            &mut res,
            Id::Number(RT_ICON),
            Id::Number(index as u16 + 1),
            0x1010,
            LANGUAGE,
            image.data,
        );
    }
    resource(
        &mut res,
        Id::Number(RT_GROUP_ICON),
        Id::Number(ICON_GROUP),
        0x1030,
        LANGUAGE,
        &group(&images),
    );
    resource(
        &mut res,
        Id::Number(RT_RCDATA),
        Id::Name(APP_ID_RESOURCE),
        0x0030,
        LANGUAGE,
        identifier.as_bytes(),
    );

    let path =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo 没给 OUT_DIR")).join("nimbus.res");
    std::fs::write(&path, res).unwrap_or_else(|error| panic!("写不了 {}：{error}", path.display()));
    // 只给可执行文件：测试程序不需要图标
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

/// `[package.metadata.bundle]` 里一项字符串设置的值（只认 `键 = "值"` 这种一行的写法）
fn bundle_setting(manifest: &str, key: &str) -> Option<String> {
    let mut in_bundle = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bundle = line == "[package.metadata.bundle]";
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if in_bundle && name.trim() == key {
            return value
                .trim()
                .strip_prefix('"')?
                .split('"')
                .next()
                .map(str::to_owned);
        }
    }
    None
}

/// .ico 里的一张图
struct Image<'a> {
    /// 宽、高（256 写成 0）
    width: u8,
    height: u8,
    /// 图的数据（PNG，或者没有文件头的 BMP）
    data: &'a [u8],
}

/// 拆开 .ico：头（保留的 0、类型 1、张数），每张一项 16 字节（宽、高、色数、保留、面数、位深、长度、在文件里的位置）
fn icon_images(ico: &[u8]) -> Vec<Image<'_>> {
    let word = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]) as usize;
    let dword =
        |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]) as usize;
    assert!(
        ico.len() >= 6 && word(0) == 0 && word(2) == 1,
        "{ICON} 不是 .ico"
    );
    let images: Vec<Image> = (0..word(4))
        .map(|index| {
            let entry = 6 + index * 16;
            let (length, offset) = (dword(entry + 8), dword(entry + 12));
            Image {
                width: ico[entry],
                height: ico[entry + 1],
                data: ico
                    .get(offset..offset + length)
                    .unwrap_or_else(|| panic!("{ICON} 的第 {index} 张不完整")),
            }
        })
        .collect();
    assert!(!images.is_empty(), "{ICON} 里没有图");
    images
}

/// 图标组资源：和 .ico 的目录几乎一样，只是每项最后不是位置，而是那一张的资源编号（从 1 开始）
fn group(images: &[Image]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        // 色数 0、保留 0、1 个面、32 位色
        out.extend_from_slice(&[image.width, image.height, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(image.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(index as u16 + 1).to_le_bytes());
    }
    out
}

/// 资源的类型或名字：编号，或者字符串
enum Id<'a> {
    Number(u16),
    Name(&'a str),
}

impl Id<'_> {
    /// 编号写成 0xFFFF 加编号，字符串写成以 0 结尾的 UTF-16
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Id::Number(number) => {
                out.extend_from_slice(&0xFFFFu16.to_le_bytes());
                out.extend_from_slice(&number.to_le_bytes());
            }
            Id::Name(name) => {
                for unit in name.encode_utf16().chain(Some(0)) {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
            }
        }
    }
}

/// 往 .res 里加一项：数据长度、头长度、类型、名字（补齐到 4 字节）、数据版本、内存标志、语言、版本、特征，
/// 然后是数据（补齐到 4 字节）。内存标志在 32 位以后的 Windows 上没有作用，照资源编译器的习惯写
fn resource(out: &mut Vec<u8>, kind: Id, name: Id, flags: u16, language: u16, data: &[u8]) {
    let mut header = Vec::new();
    kind.write(&mut header);
    name.write(&mut header);
    header.resize(header.len().next_multiple_of(4), 0);
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&flags.to_le_bytes());
    header.extend_from_slice(&language.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(header.len() as u32 + 8).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(4), 0);
}
