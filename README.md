<div align="center">

<img src="assets/icon.png" width="128" height="128" alt="Nimbus">

# Nimbus

**一个干净、好看、能记住你工作现场的终端。**

用 [Aureole](https://github.com/ruoyu-lab/aureole) 界面框架从零写成：自研 pty 和 xterm 仿真，没有 WebView，没有 Electron。

[![License: MIT](https://img.shields.io/badge/license-MIT-2f6fed.svg)](LICENSE)
![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey.svg)
![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)
[![Built with Aureole](https://img.shields.io/badge/built%20with-Aureole-7aa2ff.svg)](https://github.com/ruoyu-lab/aureole)

[特性](#特性) · [截图](#截图) · [安装](#安装) · [快捷键](#快捷键) · [配置](#配置) · [开发](#开发) · [English](README.en.md)

</div>

<div align="center">
  <img src="docs/screenshots/light.png" width="49%" alt="浅色外观">
  <img src="docs/screenshots/dark.png" width="49%" alt="深色外观">
</div>

## 特性

**终端本身**

- 自研 pty 与 xterm 仿真：256 色和真彩、鼠标上报、括号粘贴、备用屏（vim、less、htop 都能用）、滚动历史、中日韩宽字符
- 左右、上下分屏，分隔条可拖，拆出来的终端从当前目录起
- 右键菜单：复制、粘贴、全选、清屏、拆分、关闭分屏；点在链接上还有打开、复制
- 终端内查找（⌘F）：在屏幕和滚动历史里找，命中高亮，回车和 ⌘G 跳转
- 字号 ⌘+ / ⌘- / ⌘0；光标形状和闪烁、字体、行距、字体粗细、响铃（提示音或画面闪一下）
- **链接**：网址常显链接色，⌘ 点击打开，悬停有下划线和手指光标；文件路径悬停时才着色，⌘ 点击用默认应用打开（脚本只在访达里选中，不会直接运行）；支持 OSC 8 带文字的链接
- **图片**：iTerm2 内联图片（`imgcat`）、Kitty 图片协议、Sixel，跟着行一起滚动
- **文件拖进终端**：从访达拖来的文件，路径（自动引好）写在光标处
- 没开括号粘贴时，粘贴多行文字先确认

**命令标记（shell 集成，OSC 133）**

- 本机的 zsh 自动启用，不改你的任何配置文件；脚本也在配置目录的 `shell/` 里，bash 可以自己 `source`
- ⌘↑ / ⌘↓ 在命令之间跳，选中那条命令（提示符加输出），⌘C 直接复制；命令面板里「复制上一条命令的输出」
- 命令跑完（10 秒以上）、程序要求通知（OSC 9、777）、前台程序响铃，窗口在后台时发系统通知（macOS 要打包成 .app 才发得出），带命令和退出码
- 跑过的命令存下来，命令面板里打字就能搜，选中写在提示符处；重启后恢复的历史里，旧命令也能跳

**服务器和容器**

- 侧栏的「远程」：读 `~/.ssh/config` 里的主机，连上去就是一个终端；Docker 容器同样
- 连着服务器时右侧自动出现面板：**监控**（CPU、内存、磁盘、负载）和**文件**（浏览、上传、下载、把路径拖进终端）
- 只用本地终端的人看不到这些，界面里没有多余的东西

**工作区**

- 标签条可拖动换位，拖动时只左右移动、松手平滑落位
- 侧栏的「目标」：新建文件夹，或者**直接把访达里的目录拖进来**，之后在里面开的终端都从那个目录起
- 命令面板（⌘K）：搜索会话、执行命令
- 有程序在跑时关闭先确认

**记住你的现场**

- 重启后恢复标签、分屏排布、每个终端所在的目录，连**终端里的内容**（带颜色）一起带回来
- 窗口的大小和位置**按显示器分别记**，缩放比例、分辨率变了也不会把窗口丢到屏幕外面
- 设置、文件夹、侧栏宽度都会存下来

**外观**

- 浅色、深色、跟随系统，整个界面和终端一起换
- 设置页里每一项都是真的，改了马上生效

## 截图

<div align="center">
  <img src="docs/screenshots/split.png" width="49%" alt="分屏">
  <img src="docs/screenshots/settings.png" width="49%" alt="设置">
</div>

> 截图是离屏渲染的：左上角的红绿灯是按 macOS 的样子画上去的，也没有系统的窗口材质；真机上侧栏是半透明的毛玻璃。

## 安装

正式发布的版本目前只有 **macOS**（pty 用的是 `forkpty`，Linux 同一套接口但没有在桌面上测过）。
Windows 上已经能日常用（ConPTY；命令提示符、Windows PowerShell、PowerShell 7 都能选），但界面框架现在依赖的是私有仓库，
外面暂时编译不了；和 macOS 的差别见 [docs/windows.md](docs/windows.md)。

### 下载

到 [Releases](https://github.com/ruoyu-lab/nimbus-terminal/releases) 下载 `Nimbus-<版本>-macos-arm64.zip`，解压后把 `Nimbus.app` 拖进「应用程序」。

- 只有 Apple 芯片（arm64）的包，最低 macOS 12
- 没有开发者签名和公证，第一次打开会被系统拦下：右键 `Nimbus.app` → 打开；或者在终端里执行 `xattr -dr com.apple.quarantine /Applications/Nimbus.app`

### 从源码构建

需要 Rust 1.90 或更新（edition 2024）。

```bash
git clone https://github.com/ruoyu-lab/nimbus-terminal.git
cd nimbus-terminal

# 直接跑
cargo run --release

# 或者打包成 .app（target/bundle/Nimbus.app，带图标，用临时签名）
scripts/bundle-macos.sh

# 再压成发布用的 zip（target/release-assets/）
scripts/release-macos.sh
```

界面框架 Aureole 是 Git 依赖，`cargo` 会自己拉取，不需要另外装。

> **注意**：Aureole 目前还是私有仓库，没有访问权限的话 `cargo` 拉不下来、编译不了。框架公开之前，这个项目只有能访问它的人才能构建。

## 快捷键

| 操作 | 快捷键 |
| --- | --- |
| 新建会话 | `⌘T` |
| 关闭当前终端（拆分过的）/ 标签；设置页里是返回 | `⌘W` |
| 左右分屏 / 上下分屏 | `⌘D` / `⌘⇧D` |
| 命令面板 | `⌘K` |
| 在终端中查找 / 上一处 / 下一处 | `⌘F` / `⌘G` / `⌘⇧G` |
| 放大 / 缩小 / 还原字号 | `⌘+` / `⌘-` / `⌘0` |
| 跳到上一条 / 下一条命令（要命令标记） | `⌘↑` / `⌘↓` |
| 收起 / 展开侧栏 | `⌘B` |
| 打开 / 退出设置 | `⌘,` |
| 切到第 1–8 个标签 / 最后一个 | `⌘1`–`⌘8` / `⌘9` |
| 上一个 / 下一个标签 | `⌘⇧[` `⌘⇧]`，`Ctrl+Tab` |
| 粘贴 | `⌘V` |

## 配置

配置放在 `~/Library/Application Support/Nimbus/`（用环境变量 `NIMBUS_HOME` 可以改位置）：

| 文件 | 内容 |
| --- | --- |
| `settings.json` | 设置页里的所有选项 |
| `state.json` | 侧栏的文件夹、开着的标签和分屏排布、侧栏宽度 |
| `window.json` | 窗口的位置和大小（每块显示器、每种缩放各一份） |
| `output-N.ansi` | 各个终端里的内容（带颜色、链接和命令标记） |
| `history.json` | 跑过的命令（只记单行，最多 2000 条；命令里的内容是明文，别把密码敲在命令行里） |
| `shell/` | shell 集成脚本，启动时按需重写，不用改 |
| `crash.log` | 出过 panic 时的原因和调用栈 |

设置页的「关于」里点「恢复默认」可以清掉所有设置。

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test
```

测试里有几类需要真实环境：起真 shell 的（要能 `forkpty`）、离屏截图的（要有 GPU 适配器）。界面部分用框架的 `Harness` 离屏测试，不需要窗口。

分层，从下往上：

| 层 | 职责 |
| --- | --- |
| `pty` | 起子进程、搬字节、改窗口大小（跨平台的接缝是 `PtyBackend`：类 Unix 是 `posix`，Windows 是 `windows`（ConPTY）） |
| `vt` | 把字节流解析成屏幕模型：解析器、屏幕、键盘和鼠标的编码，加查找（`search`）、命令标记（`marks`）、图片（`image`：iTerm2、Kitty、Sixel） |
| `view` | 把屏幕模型同步到框架的字符网格，管终端配色 |
| `links` | 在一行文字里认出网址和文件路径 |
| `app` | 一个终端会话，按功能分成子模块：`mouse_input`、`input`、`scroll`、`selection`、`find`、`hyperlinks`、`commands`、`images`、`alerts`、`element`（界面）、`launch`（怎么起） |
| `pane` / `tabs` | 分屏的树；窗口的状态，按功能分成子模块：`persistence`、`folders`、`remote`、`panes`、`panel`（监控和文件）、`history`、`alerts`、`shortcuts` |
| `ssh` / `docker` / `channel` / `files` / `monitor` / `remote` | 服务器和容器：连接、常驻通道、文件页、监控 |
| `shell` | shell 集成：脚本和怎么让 zsh 自动加载 |
| `store` / `settings` / `display` | 落盘、设置项、窗口位置按显示器分别记 |
| `chrome` | 界面：标题栏、侧栏、卡片、右侧面板、查找条、命令面板、设置页 |

测试跟着模块走：`app/tests/` 和 `tabs/tests/` 里按功能分文件，共用的辅助函数在各自的 `mod.rs`。

重新生成 README 里的截图：

```bash
README_SHOTS=$PWD/docs/screenshots cargo test screenshots_for_readme -- --ignored
```

欢迎提 issue 和 PR，先看看 [CONTRIBUTING.md](CONTRIBUTING.md)。改动记录在 [CHANGELOG.md](CHANGELOG.md)。

## 致谢

- [Aureole](https://github.com/ruoyu-lab/aureole)：界面框架，字符网格、布局、动画、输入、离屏测试都来自它
- [Feather](https://feathericons.com/)（MIT）：GitHub 图标

## 许可证

[MIT](LICENSE) © 2026 ruoyu-lab
