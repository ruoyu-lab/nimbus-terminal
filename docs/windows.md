# Windows 支持：现状和清单

现在的状态：**本机终端能用**。ConPTY 后端（`src/pty/windows.rs`）在 Windows 11（26300）上用系统自带的 ConPTY 实测过：
起 shell、读写、改大小、退出码、关标签时结束子进程、当前目录、「下面有没有程序在跑」都有测试。
还没打包，下面「必须做」里剩下的几项做完之前，日常用起来会有不顺手的地方。

## ConPTY 后端的几处实测结论

- **透传**：系统自带的 ConPTY 会重画屏幕，但命令标记（OSC 133）原样过来；超链接（OSC 8）被改写成 `8;id=…;网址`、
  结束的那条挪到后面，仿真层照样认得（`marks_and_links_pass_through_the_pseudoconsole`）。
  内联图片（Sixel、iTerm2、Kitty）还没验证。透传不够用时的退路是像 WezTerm 那样随包带一份新版的 `conpty.dll` 和 `OpenConsole.exe`。
- **启动时的请求**：ConPTY 一起来就发 `CSI ?9001h`（win32 输入模式）和 `CSI ?1004h`（焦点事件）；前一个仿真层不认、忽略，按键照常走 VT。
- **Ctrl+C**：「忽略 Ctrl+C」这个标志会从父进程一路继承下来。Nimbus 自己是被一个忽略它的程序起的时候（一些启动器、跑测试的那一串），
  伪控制台里按 Ctrl+C 就停不下任何程序，所以每次起子进程之前先把自己的恢复成默认（`restore_ctrl_c`）。
- **当前目录**：读子进程的 PEB（`working_directory`）。cmd 的 `cd` 会改进程目录，读得到；
  **PowerShell 的 `cd`（`Set-Location`）不改进程目录**，读到的一直是启动时的目录，要靠 shell 集成发 OSC 7 报（见下面第 3 项）。
- **忙不忙**：Windows 没有前台进程组。原来设想的 `GetConsoleProcessList` 只能问调用者自己所在的控制台，用不上；
  现在是拍进程快照，看 shell 有没有子进程（比创建时间，防进程号重用）。从 shell 里打开、没等它结束的窗口程序（记事本之类）也会算「忙」。
  这个检查每个会话每秒跑一次，在主线程上。

## 必须做

1. ~~**ConPTY 后端**~~：已完成。
2. **默认 shell**：`pty::default_shell` 现在读 `%COMSPEC%`（一般是 cmd），没有才用 `powershell.exe`；
   应该按 pwsh → powershell → cmd 的顺序选。`Config::shell` 里「认识的 shell 带登录参数」那段要补上 `pwsh`、`powershell`（`-NoLogo`）。
3. **命令标记和当前目录（shell 集成）**：`src/shell.rs` 只给 zsh 自动加载。PowerShell 要一份 `assets/shell/nimbus.ps1`
   （在 `prompt` 函数里发 OSC 133 的 A、B、D，PSReadLine 回车时发 C，再发 OSC 7 报当前目录），在 `shell::environment` 里让它起来时加载；
   仿真层还要支持 OSC 7（现在整条丢掉）。
4. **控制台窗口**：`main.rs` 没有 `windows_subsystem = "windows"`，发布版会多开一个黑色的控制台窗口；加上以后，
   起 `ssh`、`docker`、`sftp`（`std::process::Command`）要带 `CREATE_NO_WINDOW`，不然每次闪一个黑窗。
5. **快捷键**：Windows 和 Linux 用 Ctrl+Shift 代替 ⌘（`tabs::shortcuts`）。Shift 永远按着，靠 Shift 区分的两组撞了：
   Ctrl+Shift+D 只能上下分（没有左右分），Ctrl+Shift+G 只能往一个方向跳。复制没有绑键盘（macOS 上的 ⌘C 要确认是谁接的）。
   侧栏、菜单里的提示还写着 ⌘T、⌘K。
6. **字体**：设置里的 Menlo、Monaco 在 Windows 上没有，要换成 Cascadia Mono、Consolas。
7. **窗口外观**：`native::keep_window_still_unless_asked`、`set_application_icon` 在非 macOS 上是空的；
   框架那边标题栏、改大小、输入的 Windows 改进发布以后，升级 `aureole` 的 `rev`。
8. **显示器信息**：`display::displays` 在非 macOS 上返回空（窗口位置交给系统，只记大小），要接 Win32 的显示器枚举才能记住窗口在哪块屏。

## 会受影响的功能

| 功能 | Windows 上的情况 |
|---|---|
| 本机终端 | 能用（ConPTY） |
| 监控、文件页（SSH） | 依赖 OpenSSH 的 `ControlMaster`，Windows 自带的 OpenSSH 不支持；`ssh::control_directory` 在非 Unix 上返回 `None`，所以现在是「终端能连、监控和文件页没有」。要支持得给常驻通道单开一条连接（只能用密钥登录），或者换成 Rust 的 SSH 库 |
| Docker 目标 | 走 `docker exec -it`，ConPTY 好了以后应该直接能用，没验证 |
| 后台通知 | 要有应用标识（AppUserModelID），从安装程序建的开始菜单快捷方式启动才行（框架在做由 `cargo bundle` 写进可执行文件） |
| 点击路径、拖文件进终端 | `C:\`、`.\`、`~\` 这类路径在 `links` 和 `app::hyperlinks::resolve_path` 里认不出来；拖进来的路径按 `sh` 的规矩加单引号，cmd 不认 |
| 文件页的上传、下载 | `files::sftp_get`、`sftp_put` 要求本地路径以 `/` 开头，Windows 上一律被拒 |
| 打包 | `scripts/` 里只有 macOS 的；框架的 `cargo bundle` 能出 NSIS 安装程序，要在 `Cargo.toml` 里写 `[package.metadata.bundle]` |

## 测试

- 只在 Unix 上跑的测试大多已经用 `#[cfg(unix)]` 标好（真 shell、符号链接、sftp 实验室）。
- 在 Windows 上 `cargo test` 还有 19 个失败，都不是 ConPTY 的：调用 `sh` 的（`channel`、`monitor`、`files` 里的，PATH 里有 Git 的 `sh` 时能过）、
  按 macOS 写的（⌘ 点链接、`/tmp` 路径）、依赖 `ControlMaster` 的、上面说的路径和字体。
- CI 里有 Windows 的编译检查（`cargo check --all-targets`），这些测试修好以后再加 `cargo test`。CI 的 clippy 只在 macOS 上跑，管不到 `pty/windows.rs`。
- 框架 Aureole 的 Windows 渲染：字形回退、系统材质还要在真机上看。
