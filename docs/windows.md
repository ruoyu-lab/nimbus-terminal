# Windows 支持：现状和清单

现在的状态：**能在 Windows 上编译（`cargo check --all-targets --target x86_64-pc-windows-msvc` 零警告），但还不能用**——
终端起不了 shell，因为 ConPTY 后端还没写。界面、设置、标签、查找这些不碰系统的部分是同一套代码。

## 必须做

1. **ConPTY 后端**：`src/pty/windows.rs`，骨架和要做的事写在文件头。接口是 `pty::PtyBackend`（写、改大小、问目录、问忙不忙），
   上层（`app`、`tabs`）不用改。
2. **默认 shell**：`pty::default_shell` 已经按平台选（`%COMSPEC%`，没有就 `powershell.exe`）；
   `Config::shell` 里「认识的 shell 带登录参数」那段要补上 `pwsh`、`powershell`、`cmd` 的写法。
3. **命令标记（shell 集成）**：`src/shell.rs` 只给 zsh 自动加载。PowerShell 要一份 `assets/shell/nimbus.ps1`
   （在 `prompt` 函数里发 OSC 133 的 A、B、C、D），再在 `shell::environment` 里让 `pwsh` 起来时加载它。
4. **快捷键**：Windows 和 Linux 用 Ctrl+Shift 代替 ⌘（`tabs::shortcuts` 已经按这个写，设置页的说明也有），要在真机上过一遍。
5. **窗口外观**：`native::keep_window_still_unless_asked`、`set_application_icon` 在非 macOS 上是空的；
   系统材质、标题栏按钮的位置（`TitleBarInfo`）要在 Windows 上对一遍。
6. **显示器信息**：`display::displays` 在非 macOS 上返回空（窗口位置交给系统，只记大小），要接 Win32 的显示器枚举才能记住窗口在哪块屏。

## 会受影响的功能

| 功能 | Windows 上的情况 |
|---|---|
| 本机终端 | 等 ConPTY |
| 监控、文件页（SSH） | 依赖 OpenSSH 的 `ControlMaster`，Windows 自带的 OpenSSH 不支持；`ssh::control_directory` 在非 Unix 上返回 `None`，所以现在是「终端能连、监控和文件页没有」。要支持得每次请求另开一条连接，或者换成 Rust 的 SSH 库 |
| Docker 目标 | 走 `docker exec -it`，在 ConPTY 好了之后应该直接能用，没验证 |
| 后台通知 | 要有应用标识（AppUserModelID），从安装程序建的开始菜单快捷方式启动才行 |
| 点击路径 | `~`、反斜杠路径、盘符都要在 `links` 和 `app::hyperlinks::resolve_path` 里补 |
| 打包 | `scripts/` 里只有 macOS 的；要加安装程序（MSIX 或 NSIS） |

## 测试

- 只在 Unix 上跑的测试已经用 `#[cfg(unix)]` 标好（真 shell、符号链接、sftp 实验室）。
- CI 里有 Windows 的编译检查（`cargo check --all-targets`），ConPTY 写好后再加 `cargo test`。
- 框架 Aureole 的 Windows 渲染只做过编译检查，字形回退和系统材质没在真机上验证过。
