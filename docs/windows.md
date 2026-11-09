# Windows 支持：现状和清单

现在的状态：**能日常用**。在 Windows 11（26300，系统自带的 ConPTY）上实测：本机终端、分屏、标签、设置、恢复现场，
PowerShell 的命令标记，服务器的监控和文件页（密钥登录），点击和拖进来的 Windows 路径，发布版（不开控制台窗口、带图标）都可以用。
`cargo test` 在 Windows 上全部通过（Git Bash 和 PowerShell 里都是）。

## 和 macOS 的差别

| 功能 | Windows 上 |
|---|---|
| 新建会话 | 加号旁边的小箭头选 shell：命令提示符、Windows PowerShell、PowerShell 7（没装的灰着写「未安装」）；命令面板里也有。标签记住自己的 shell，拆分、重启都跟着走；默认标题就是 shell 的名字 |
| 默认 shell | 按 PowerShell 7 → Windows PowerShell → 命令提示符取第一个装了的（`pty::default_shell`）；设置里「指定程序」可以换 |
| 快捷键 | Ctrl+Shift 代替 ⌘，例外：左右、上下分屏是 Alt+Shift+= / Alt+Shift+-，查找的下一处、上一处是 F3 / Shift+F3（查找框开着时），复制 Ctrl+Shift+C 或 Ctrl+Insert，粘贴 Ctrl+Shift+V 或 Shift+Insert，全选 Ctrl+Shift+A。单独的 Ctrl+C、Ctrl+A 照常给 shell。界面上的提示按平台写（`tabs::Shortcut`） |
| 命令标记（shell 集成） | Windows PowerShell 5.1、PowerShell 7 自动加载 `assets/shell/nimbus.ps1`（`-NoExit -Command` 在配置文件之后读进来，不受执行策略限制，不改用户的任何文件）：发 OSC 133 的 A、B、C、D 和 OSC 7。命令提示符没有（cmd 发不出 C、D） |
| 当前目录 | PowerShell 靠 OSC 7（它的 `cd` 不改进程目录）；命令提示符读进程的 PEB |
| 服务器的监控、文件页 | Windows 自带的 OpenSSH 没有连接复用（`ControlMaster`）：面板另开只用密钥的连接（`BatchMode=yes`），只能输密码的服务器提示「需要密钥登录」，不反复重试 |
| 路径 | 认 `C:\…`、`C:/…`、`.\…`、`..\…`、`~\…`、`\\服务器\共享\…`；拖进来的路径按这个终端起的 shell 引（PowerShell 单引号、cmd 双引号，能两边都对时用双引号）；文件页的上传下载换成 `C:/…` 交给 `sftp`；远端文件名里 Windows 不能有的字换成 `_`，跑不出下载目录 |
| 字体 | 系统等宽（Cascadia Mono，Windows 10 上退到 Consolas）、Cascadia Mono、Consolas、Courier New |
| 图片 | iTerm2 内联图片能用；Kitty（APC）、Sixel（DCS）被系统的 ConPTY 整段吃掉（测试标了 `#[ignore]`）。要支持得像 WezTerm 那样随包带新版的 `conpty.dll` 和 `OpenConsole.exe` |
| 窗口 | 窗口按钮按 Windows 的样子自己画；侧栏透出系统材质（Mica，框架 `c2728f4` 起开着材质也不会多出系统的窗口按钮）；外框的底色盖得比 macOS 薄（深色 66%、浅色的白 19%），材质透得出来、字也清楚。Mica 只带桌面壁纸的颜色、不透出后面的窗口，窗口不在前台、或者系统设置里关了「透明效果」时是纯色。窗口位置按显示器记（Win32 枚举显示器） |
| 打包 | `scripts\bundle-windows.ps1`：默认出 NSIS 安装程序（先要 `cargo install cargo-packager --version 0.11.8 --locked`，第一次还会下载 NSIS）；`-Format installer` 用框架自己的安装程序，不用下载。编译时（`build.rs`）就把图标和应用标识写进可执行文件 |
| 通知 | 应用标识 `com.ruoyu-lab.nimbus` 在启动时声明；Windows 要先登记过这个标识才显示通知，安装程序建开始菜单快捷方式时会登记 |
| Docker 目标 | 代码在 Windows 上编译、子进程不弹窗口；这台机器没装 Docker，没验证 |

## ConPTY 后端的几处实测结论

- **透传**：系统自带的 ConPTY 会重画屏幕。命令标记（OSC 133）、OSC 7、ConEmu 的 OSC 9;9 原样过来；
  超链接（OSC 8）被改写成 `8;id=…;网址`、结束的那条挪到后面，仿真层照样认得；图片见上表。
- **启动时的请求**：ConPTY 一起来就发 `CSI ?9001h`（win32 输入模式）和 `CSI ?1004h`（焦点事件）；前一个仿真层不认、忽略，按键照常走 VT。
- **Ctrl+C**：「忽略 Ctrl+C」这个标志会从父进程一路继承下来。Nimbus 自己是被一个忽略它的程序起的时候（一些启动器、跑测试的那一串），
  伪控制台里按 Ctrl+C 就停不下任何程序，所以每次起子进程之前先把自己的恢复成默认（`restore_ctrl_c`）。
- **忙不忙**：Windows 没有前台进程组，`GetConsoleProcessList` 只能问调用者自己所在的控制台，用不上；
  现在是拍进程快照，看 shell 有没有子进程（比创建时间，防进程号重用）。从 shell 里打开、没等它结束的窗口程序（记事本之类）也会算「忙」。
  这个检查每个会话每秒跑一次，在主线程上。

## 还没做、要在框架那边做的

1. **框架依赖是私有仓库**：现在用的是 aureole-framework（私有）master 的 `7f997ec`，外部的人和 CI 都拉不到。
   框架出了带 Windows 改进的公开版本以后换回公开的依赖。在那之前 CI 的两个任务都会在第一步失败。
2. **混合缩放的多显示器**：窗口位置按显示器记，但窗口库按建窗口时那块显示器的缩放换算存下的位置；
   主屏 150%、副屏 100% 时在副屏上关掉、再打开，要看会不会开到一半在屏幕外面。
3. **macOS、Linux 没编译过**：这一轮的改动在 Windows 上做的，类 Unix 的分支只读过代码，要等 macOS 的 CI（依赖换回公开的以后）。
   CI 的 clippy 只在 macOS 上跑，管不到 `cfg(windows)` 的代码（`pty/windows.rs` 等），本机也没装 clippy。

## 测试

- 只在 Unix 上跑的测试用 `#[cfg(unix)]` 标好（真 shell、`sh` 跑的协议测试、符号链接、sftp 实验室）。
- Windows 上的真 shell 测试：`pty::windows` 的测试起 cmd、Windows PowerShell 5.1（大小、环境变量、中文、目录、Ctrl-C、命令标记和退出码、OSC 7、图片透传）。
  装了 PowerShell 7 的机器上，集成脚本的两个测试再用它跑一遍（`pwsh_integration_*`；实测 7.6.9，winget 装的商店版，
  程序是 `WindowsApps` 里的别名，`CreateProcessW` 起得来）；没装就跳过。
- 好几个工作树共用一个 `target` 时，`cargo test` 可能跑的是别的工作树刚编出来的测试程序（同一个路径、同一个哈希），看一下测试总数对不对。
