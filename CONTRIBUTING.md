# 参与贡献

谢谢你愿意改进 Nimbus。小到错别字、大到新功能，都欢迎。

## 先聊再做

比较大的改动（新功能、改界面、动架构）请先开一个 [issue](https://github.com/ruoyu-lab/nimbus-terminal/issues/new/choose) 说说想法，免得白做。小的修复直接发 PR 就行。

## 本地开发

需要 macOS 和 Rust 1.90+。

```bash
git clone https://github.com/ruoyu-lab/nimbus-terminal.git
cd nimbus-terminal
cargo run          # 开发时依赖也编成优化的（见 Cargo.toml），第一次编译会久一点
cargo test
```

试用时想用一份干净的配置，指定一个临时目录，不会动到你自己的设置：

```bash
NIMBUS_HOME=/tmp/nimbus-dev cargo run
```

## 提 PR 之前

请确认下面三条都通过：

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test
```

- **加了行为就加测试。** 界面用框架的 `Harness` 离屏测试（`src/tabs/tests.rs` 里有很多例子，点击、拖动、截图像素都能测），不需要真的开窗口。
- **修 bug 先写能复现它的测试**，再改代码，让测试从失败变成通过。
- `unsafe` 必须有 `// SAFETY:` 注释说明为什么成立（clippy 会检查）。
- 注释写「为什么」，不写「做了什么」。项目里的注释用中文，保持一致就好。
- 不要为了「兼容」加没人会走到的分支，也不要留死代码。

## 项目结构

分层和各模块的职责见 [README 的「开发」一节](README.md#开发)。几个约定：

- 状态都在 `Workspace`（`src/tabs.rs`），界面（`src/chrome`）只读信号、调方法。
- 会话、标签这类比界面活得久的东西，里面的信号要放在不挂界面的作用域里（`aureole::detached`），否则界面一卸载信号就被释放，之后再读会 panic。
- 颜色都从 `src/theme.rs` 取（函数，不是常量），深浅色变了整个界面会重建。

## 提交信息

用一句话说清楚改了什么、为什么。中文英文都行。

## 许可

提交代码即表示你同意按 [MIT](LICENSE) 授权。
