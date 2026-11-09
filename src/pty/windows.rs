//! Windows 的 pty 后端：ConPTY（`CreatePseudoConsole`）。**现在只有骨架，还没有实现。**
//!
//! 要做的事（接口都在 [`super::PtyBackend`]，上层不用改）：
//! - `spawn`：`CreatePseudoConsole` 建伪控制台，用 `CreateProcessW` 加 `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` 起子进程，
//!   两根管道一根读一根写；读线程把输出塞进 [`super::ByteQueue`]（和 `posix` 一样的读写分线程），子进程退出时报结束
//! - `write`、`resize`：写管道、`ResizePseudoConsole`
//! - `is_busy`：问控制台里有几个进程挂着（`GetConsoleProcessList`），比 shell 多就是前台有程序在跑
//! - `working_directory`：读子进程的当前目录（`NtQueryInformationProcess` 读 PEB），读不到返回 `None` 也行
//! - 起进程要带 `Config::environment`、`working_directory`，`COLORFGBG` 这类提示也要给
//!
//! 别的要一起看的：[`crate::shell`] 里 PowerShell 的命令标记脚本、`docs/windows.md` 里的清单。

use std::io;

use super::{ByteQueue, Config, Pty};

/// 还没实现：明说，别编出一个能过但跑不对的东西
pub fn spawn(_config: Config) -> io::Result<(Pty, ByteQueue)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows 的 ConPTY 后端还没写（见 src/pty/windows.rs 和 docs/windows.md）",
    ))
}
