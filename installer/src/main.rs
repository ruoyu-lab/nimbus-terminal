//! Nimbus 在 Windows 上的安装程序（打包工具接在它后面的是终端本身，见 `scripts\bundle-windows.ps1`）。
//!
//! 解压、校验、出错回滚、升级、卸载、开始菜单和桌面的快捷方式、「应用和功能」里的登记都是框架的安装引擎做的
//! （`aureole_installer::Setup`）；这里只画界面（[`ui`]）、补一个步骤（[`FullVersion`]）。
//! 卸载也是这个程序：装好以后它就是安装目录里的 `uninstall.exe`。
//!
//! 命令行（框架认的）：`--silent` 不开窗口，`--dir 目录` 指定目录，`--no-desktop-shortcut`、`--no-menu-shortcut`。

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ui;

use aureole::TitleBar;
use aureole_installer::Setup;

/// 终端的完整版本号（两个包共用工作区的版本）。框架给系统看的版本号去掉了 `-beta` 这种后缀，界面上写这个
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> std::process::ExitCode {
    Setup::new()
        .window(|window| {
            window
                .size(ui::WIDTH, ui::HEIGHT)
                .title_bar(TitleBar::Hidden)
        })
        .step(|| FullVersion)
        .run(ui::build)
}

/// 「应用和功能」里写完整的版本号：引擎登记的是给系统看的 `0.2.1`，这里改成 `0.2.1-beta`。
/// 卸载、回滚时登记由引擎整个删掉或者还原，这里不用管
struct FullVersion;

impl aureole_installer::Step for FullVersion {
    fn name(&self) -> &str {
        "登记版本号"
    }

    fn apply(&mut self, cx: &aureole_installer::StepContext) -> Result<(), String> {
        #[cfg(windows)]
        registry::set_display_version(&cx.manifest.identifier, VERSION);
        #[cfg(not(windows))]
        let _ = cx;
        Ok(())
    }
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// 改「应用和功能」里这个应用的版本号。登记不在（预览、没登记）就不建：只改已有的
    pub fn set_display_version(identifier: &str, version: &str) {
        let path = wide(&format!(
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{identifier}"
        ));
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: 路径以 0 结尾；打开成功时 `key` 是新开的键
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if opened != ERROR_SUCCESS {
            return;
        }
        let (name, value) = (wide("DisplayVersion"), wide(version));
        // SAFETY: 键开着；值是以 0 结尾的 UTF-16，长度按字节算、含结尾的 0
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
            RegCloseKey(key);
        }
    }
}
