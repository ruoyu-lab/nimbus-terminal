# Windows 支持：现状和清单

现在的状态：**能日常用**。在 Windows 11（26300，系统自带的 ConPTY）上实测：本机终端、分屏、标签、设置、恢复现场，
PowerShell 的命令标记，服务器的监控和文件页（密钥登录），点击和拖进来的 Windows 路径，发布版（不开控制台窗口、带图标）都可以用。
`cargo test` 在 Windows 上全部通过（Git Bash 和 PowerShell 里都是）。

## 和 macOS 的差别

| 功能 | Windows 上 |
|---|---|
| 新建会话 | 加号旁边的小箭头选 shell：命令提示符、Windows PowerShell、PowerShell 7（没装的灰着写「未安装」），装了 WSL 的发行版时下面再列每个发行版；命令面板里也有。WSL 的发行版还列在侧栏的「WSL」里（见下面「WSL」）。标签记住自己的 shell，拆分、重启都跟着走；默认标题就是 shell 的名字 |
| 默认 shell | 按 PowerShell 7 → Windows PowerShell → 命令提示符取第一个装了的（`pty::default_shell`）；设置里「指定程序」可以换 |
| 快捷键 | Ctrl+Shift 代替 ⌘，例外：左右、上下分屏是 Alt+Shift+= / Alt+Shift+-，查找的下一处、上一处是 F3 / Shift+F3（查找框开着时），复制 Ctrl+Shift+C 或 Ctrl+Insert，粘贴 Ctrl+Shift+V 或 Shift+Insert，全选 Ctrl+Shift+A。单独的 Ctrl+C、Ctrl+A 照常给 shell。界面上的提示按平台写（`tabs::Shortcut`） |
| 命令标记（shell 集成） | Windows PowerShell 5.1、PowerShell 7 自动加载 `assets/shell/nimbus.ps1`（`-NoExit -Command` 在配置文件之后读进来，不受执行策略限制，不改用户的任何文件）：发 OSC 133 的 A、B、C、D 和 OSC 7。命令提示符没有（cmd 发不出 C、D） |
| 当前目录 | PowerShell 靠 OSC 7（它的 `cd` 不改进程目录）；命令提示符读进程的 PEB |
| 服务器的监控、文件页 | Windows 自带的 OpenSSH 没有连接复用（`ControlMaster`）：面板另开只用密钥的连接（`BatchMode=yes`），只能输密码的服务器提示「需要密钥登录」，不反复重试 |
| 路径 | 认 `C:\…`、`C:/…`、`.\…`、`..\…`、`~\…`、`\\服务器\共享\…`；拖进来的路径按这个终端起的 shell 引（PowerShell 单引号、cmd 双引号，能两边都对时用双引号）；文件页的上传下载换成 `C:/…` 交给 `sftp`；远端文件名里 Windows 不能有的字换成 `_`，跑不出下载目录 |
| 字体 | 系统等宽（Cascadia Mono，Windows 10 上退到 Consolas）、Cascadia Mono、Consolas、Courier New |
| 图片 | iTerm2 内联图片、Kitty、Sixel 都能用：Kitty（APC）、Sixel（DCS）要靠随包的新版 ConPTY（见下面「随包的 ConPTY」），系统自带的会把它们整段吃掉 |
| 窗口 | 窗口按钮按 Windows 的样子自己画；侧栏透出系统材质（Mica，框架 `c2728f4` 起开着材质也不会多出系统的窗口按钮）；外框的底色盖得比 macOS 薄（深色 66%、浅色的白 19%），材质透得出来、字也清楚。Mica 只带桌面壁纸的颜色、不透出后面的窗口，窗口不在前台、或者系统设置里关了「透明效果」时是纯色。窗口位置按显示器记（Win32 枚举显示器） |
| 打包 | `scripts\bundle-windows.ps1`：默认出 NSIS 安装程序（先要 `cargo install cargo-packager --version 0.11.8 --locked`，第一次还会下载 NSIS）；`-Format installer` 用框架自己的安装程序，不用下载。编译时（`build.rs`）就把图标和应用标识写进可执行文件 |
| 通知 | 应用标识 `com.ruoyu-lab.nimbus` 在启动时声明；Windows 要先登记过这个标识才显示通知，安装程序建开始菜单快捷方式时会登记（实测快捷方式带着这个标识；通知弹不弹还没在安装版里看过） |
| 安装程序（`-Format installer`） | 界面是自己画的（`installer/`，见下面「安装程序」）：一页、正中一个按钮，样子参考比邻（bilin-desk）的安装程序；解压、回滚、升级、卸载、快捷方式、「应用和功能」的登记用框架的安装引擎。实测（Windows 11）：装给当前用户，在 `%LOCALAPPDATA%\Programs\Nimbus`，开始菜单、桌面各一个快捷方式，「应用和功能」里一条（版本号是完整的 `0.2.1-beta`）；`--silent` 一秒装完。从 0.2.1-beta 覆盖升级：多出 `conpty\`，卸载信息还是一条，快捷方式不变；程序开着时升级会拒绝（「正被使用」），旧版不受影响。卸载把文件、快捷方式、卸载信息都清掉，不碰配置目录。**框架的问题**：打开卸载程序又关掉（没卸），安装目录里的 `uninstall.exe` 也会被删掉，之后「应用和功能」里卸载不了（框架的 `delete_self_later` 不管卸没卸都删），要在框架里改 |
| Docker 目标 | 代码在 Windows 上编译、子进程不弹窗口；这台机器没装 Docker，没验证 |

## WSL

装了的发行版自动列出来（`wsl` 模块）：

- **侧栏**：「目标」下面一节「WSL」，和「容器」一个样子。点发行版：它已经有会话就切过去，没有就开一个；右键「新建会话」再开一个；
  它的会话列在它下面，不混在本机的会话里。WSL 的会话和容器的一样不归文件夹（从新建的下拉开的也是）。
  侧栏挂上时（启动、从设置页回来）重新列一遍，新装的发行版也出来。新建的下拉（「WSL」那一节）、命令面板里也能开。
- **列出来**：读注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`，不调 `wsl.exe`（那样会把 WSL 的虚拟机拉起来）。
  默认的发行版排最前；Docker Desktop、Rancher Desktop 自己用的（`docker-desktop*`、`rancher-desktop*`）不列。
- **起**：标签里记成 `wsl:发行版`，起的是 `wsl.exe -d 发行版 --exec /bin/sh -c 引导脚本`。不用 `--cd`：目录不在了它直接报错退出，
  引导脚本自己进，进不去就报一句「进不了这个目录」、留在主目录。用户的登录 shell 按 `/etc/passwd` 找，照登录 shell 起。
  还没建过 Linux 用户的发行版（注册表里 `RunOOBE` 是 1）不带引导：`--exec` 会跳过建用户那一步直接用 root，所以让 `wsl.exe` 先问用户名和密码。
- **shell 集成**：集成脚本的目录经 `WSLENV`（`/pu`）换成 `/mnt/c/…` 传进去。bash 用 `--init-file nimbus.bash`（脚本先照登录 shell 的顺序跑
  `/etc/profile` 和 `~/.bash_profile`／`~/.bash_login`／`~/.profile`），zsh 用和 macOS 一样的 `ZDOTDIR` 垫片，别的 shell 照常起、没有命令标记。
  在 WSL 里（`$WSL_DISTRO_NAME`）脚本每个提示符多报一次 OSC 7；macOS 上不报，那里终端自己问进程。
  bash 脚本只认画完提示符之后的第一条命令（`PROMPT_COMMAND` 最后放一个 `__nimbus_arm`）：Ubuntu 的 systemd 往 `PROMPT_COMMAND`
  里加了报 OSC 3008 的命令，以前会被当成用户的命令，命令标记全乱。
- **目录**：WSL 里报的 `/home/me` 在 Windows 这边写成 `\\wsl.localhost\发行版\home\me`，`/mnt/c/x` 写回 `C:\x`；
  拆分、恢复从这里起，Ctrl 点击 `/…`、`~/…`（发行版里默认用户的主目录，按 `/etc/passwd`）、相对路径都换过去打开。
  `wsl.exe` 这个进程的目录和 WSL 里的 shell 无关，不用；没有 shell 集成时拆分从主目录起。
  报目录的主机要是这台机器（WSL 默认用 Windows 的计算机名），WSL 里自己改过主机名的话目录跟不上。
- **拖文件**：`C:\x` 写成 `/mnt/c/x`，这个发行版自己的共享路径写成 Linux 的路径，按 `sh` 的规矩引；换不过去的（别的共享路径）原样引好。
  `/mnt` 是 WSL 默认的挂载点，`wsl.conf` 里改过 `automount.root` 的话要自己改路径。
- **忙不忙**：WSL 里的进程 Windows 这边看不到，只看命令标记（有一条命令开始了、还没结束）；没有 shell 集成时当它不忙，关的时候不问。
- **环境变量**：`COLORTERM`、`TERM_PROGRAM`、`COLORFGBG` 经 `WSLENV` 传进去（`TERM` WSL 自己设成 `xterm-256color`）；用户原来的 `WSLENV` 接在前面。
- 实测：Ubuntu（WSL 2.7.11，内核 6.18）里的 bash。zsh 的那段只读过代码，没在 WSL 里跑过。

## 安装程序

`installer/` 是工作区里的第二个包（`nimbus-installer`），`Cargo.toml` 的 `[package.metadata.bundle.installer]` 让打包工具用它当安装程序，
后面接上终端本身。装好以后它也是安装目录里的 `uninstall.exe`。

- **界面**（`installer/src/ui.rs`）：560 × 360 的固定窗口，不要系统的标题栏，右上角自己画最小化、关闭，整个窗口按住能拖。
  正中是图标、「Nimbus」和版本号、一句介绍，下面一个按钮「立即安装」，再下面一行小字「安装到 …  更改」和「在桌面上创建快捷方式」。
  装的时候按钮换成一根细进度条和百分比，装完图标换成对勾、按钮换成「打开 Nimbus」。目录里装过时写「会从 0.2.0 更新到 …」、按钮叫「立即更新」。
  失败、取消都能「再试一次」；Nimbus 开着时说「关掉它以后再试一次」。卸载是同一个样子，说清楚设置会留着。
  颜色是终端的那几档灰，深浅跟着系统。
- **引擎**：框架的 `aureole_installer::Setup`。只多一个步骤（`FullVersion`）：框架给系统登记的版本号去掉了 `-beta`，这里改回完整的。
- **看样子**：`NIMBUS_INSTALLER_SHOTS=目录 cargo test -p nimbus-installer screenshots`，每种状态浅色、深色各存一张图。
- 命令行和框架的一样：`--silent`、`--dir 目录`、`--no-desktop-shortcut`、`--no-menu-shortcut`。

## 随包的 ConPTY

系统自带的 ConPTY（kernel32 里的那份）会把 Kitty 图片（APC）、Sixel（DCS）整段吃掉。安装包带着微软开源的新版：
NuGet 包 `Microsoft.Windows.Console.ConPTY`（MIT，源码在 microsoft/terminal）里的 `conpty.dll` 和它起的 `OpenConsole.exe`，
装在程序旁边的 `conpty\` 里，再加一份许可证。

- **取来**：`scripts\fetch-conpty.ps1` 下载固定的版本（现在是 1.25.260930003），核对包的 SHA-256 和两个文件的微软签名，放进
  `vendor\conpty\`（不进 Git）。`bundle-windows.ps1` 打包前会先跑它；`Cargo.toml` 里 `[package.metadata.bundle]` 的 `resources`
  指着这个目录，两种安装程序都把它装在 exe 旁边。免安装的 zip 也要带上这个 `conpty\` 文件夹。
  换版本时两个文件要来自同一个包：它们之间的协议没有版本检查，不配套会出怪问题。
- **用哪一份**（`pty::windows::Api`，第一次起伪控制台时选定）：环境变量 `NIMBUS_CONPTY`（`system` 是一定用系统的，别的当目录）→
  程序旁边的 `conpty\` → 调试版再看源码目录里的 `vendor\conpty\`（`cargo run`、`cargo test` 用它）→ 都没有就用系统自带的，照样能用，只是没有那两种图片。
  DLL 用完整路径载，它依赖的只在它自己的目录和 System32 里找。
- **字宽**：新版默认按字素簇算字宽，emoji 组合（👨‍👩‍👧）算 2 格，仿真层一个字一个字地算是 6 格，程序问到的光标位置（PowerShell 编辑命令行时就靠它）
  和屏幕上的对不上。所以建伪控制台时给 `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`，两边都按 `wcswidth` 算；系统自带的不认这个标志，不给它。
- **进程**：`OpenConsole.exe` 是 Nimbus 的子进程（系统的那份是 `conhost.exe`），没有窗口；关会话时跟着退出。
- 实测：两份各跑一遍全部测试都过；真程序里 PowerShell 7 输出的 Sixel、Kitty 图片都显示出来，换成系统的就没了。

## ConPTY 后端的几处实测结论

- **透传**：ConPTY 会重画屏幕。命令标记（OSC 133）、OSC 7、ConEmu 的 OSC 9;9 原样过来；
  超链接（OSC 8）被改写成 `8;id=…;网址`、结束的那条挪到后面，仿真层照样认得；图片见上表。两份 ConPTY 都是这样。
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
- 装了 WSL 发行版的机器上，`wsl_*` 的测试用第一个发行版真的起 bash（加 `-u root`，不管建没建过用户）：命令标记、OSC 7、`cd`、起始目录、
  进不去的目录、不带集成；会话层的测试再点一下 `\\wsl.localhost` 下面的文件。没装就跳过。
- 两份 ConPTY 都要测：先跑 `scripts\fetch-conpty.ps1`，`cargo test` 用的是随包的那份（Kitty、Sixel 透传、字宽的测试只在它上面跑，
  没取来就跳过）；再 `NIMBUS_CONPTY=system cargo test` 用系统自带的跑一遍。系统的那份吃掉 Kitty、Sixel 有专门的测试，哪天系统放过去了它会失败，提醒改文档。
- `.gitattributes` 让 `assets/shell/` 里的 bash、zsh 脚本一直是 LF：它们原样嵌进程序、在 Linux 里跑，Windows 上默认的 CRLF 检出会让 bash 报错。
- 好几个工作树共用一个 `target` 时，`cargo test` 可能跑的是别的工作树刚编出来的测试程序（同一个路径、同一个哈希），看一下测试总数对不对。
