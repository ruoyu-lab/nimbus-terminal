//! 常驻通道：一个长期开着的远端 `sh`，之后每次操作只是往它的标准输入写一段脚本、读一段带结束标记的输出。
//!
//! 为什么要它：每个操作各起一个 `ssh` 进程，远端要把用户的登录 shell 再启动一遍
//! （`.bashrc` 里有 nvm、conda 的话一次几百毫秒），每点一下付一次。通道里那个 `sh` 只启动一次，
//! 之后每次操作就是一个网络往返。实测（本机临时 sshd）借已有的连接跑一条命令是 11 毫秒左右。
//!
//! 协议：每个请求把脚本包进子 shell（`cd` 之类不会留在通道里），跑完在标准输出和标准错误各打一条结束标记，
//! 标准输出那条还带着退出码。标记里有一个本通道随机的口令和请求序号，两头夹着 NUL：
//! 远端的数据（文件名之类）造不出这个字节序列，所以不能提前「结束」一个请求或者冒充下一个请求的输出。
//! 请求一次只做一个（互斥），所以不会错位。
//!
//! 出了岔子（超时、连接断了、读写失败）这条通道就作废、杀掉进程，下一个请求调用的人换一条新的。
//! 这一层不认识 `ssh`：给它一个 [`Command`] 就行，所以协议能用本机的 `sh` 直接测。

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// 一个请求的结果
#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// 请求没做成的原因。两种都让这条通道作废
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// 通道断了（进程退出、读写失败）
    Dead,
    /// 超时没有结束标记
    Timeout,
}

enum Message {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Closed,
}

struct Inner {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Message>,
    token: String,
    sequence: u64,
    out: Vec<u8>,
    err: Vec<u8>,
    dead: bool,
}

pub struct Channel {
    inner: Mutex<Inner>,
}

/// 通道自己的口令：每条通道不一样，猜不到
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    format!("{:016x}", hasher.finish())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 一个请求发给 `sh` 的文字：脚本放进子 shell，标准输入接到 `/dev/null`（脚本里读标准输入的命令
/// 不能把后面的协议行当成输入吃掉），跑完打两条结束标记
fn wire(script: &str, token: &str, sequence: u64) -> String {
    let mark = format!("@@NIMBUS-{token}-{sequence}@@");
    format!(
        "( {script}\n) </dev/null\n__nimbus_status=$?\nprintf '\\000{mark}%d\\000' \"$__nimbus_status\"\nprintf '\\000{mark}\\000' >&2\n"
    )
}

impl Channel {
    /// 起一条通道：跑 `command`（它的标准输入输出会被接管）。起来之后先做一次空请求，把远端 shell 启动时
    /// 可能打出来的东西（有的 `.bashrc` 会往标准输出打字）读掉丢掉，免得混进第一个请求的结果
    pub fn start(mut command: Command) -> Result<Self, Error> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| Error::Dead)?;
        let stdin = child.stdin.take();
        let (sender, messages) = mpsc::channel();
        let reader = |mut stream: Box<dyn Read + Send>,
                      wrap: fn(Vec<u8>) -> Message,
                      sender: mpsc::Sender<Message>| {
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 64 * 1024];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            if sender.send(wrap(buffer[..count].to_vec())).is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = sender.send(Message::Closed);
            });
        };
        if let Some(stdout) = child.stdout.take() {
            reader(Box::new(stdout), Message::Out, sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            reader(Box::new(stderr), Message::Err, sender);
        }
        let channel = Self {
            inner: Mutex::new(Inner {
                child,
                stdin,
                messages,
                token: token(),
                sequence: 0,
                out: Vec::new(),
                err: Vec::new(),
                dead: false,
            }),
        };
        channel.request("true", Duration::from_secs(15))?;
        Ok(channel)
    }

    /// 在通道里跑一段 `sh` 脚本。同一时刻只有一个请求在跑，别的等着
    pub fn request(&self, script: &str, timeout: Duration) -> Result<Output, Error> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.dead {
            return Err(Error::Dead);
        }
        inner.sequence += 1;
        let mark = format!("@@NIMBUS-{}-{}@@", inner.token, inner.sequence);
        let out_prefix = format!("\0{mark}").into_bytes();
        let err_marker = format!("\0{mark}\0").into_bytes();
        let text = wire(script, &inner.token, inner.sequence);
        let written = inner.stdin.as_mut().is_some_and(|stdin| {
            stdin
                .write_all(text.as_bytes())
                .and_then(|()| stdin.flush())
                .is_ok()
        });
        if !written {
            inner.kill();
            return Err(Error::Dead);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let out_end = find(&inner.out, &out_prefix).and_then(|position| {
                let after = position + out_prefix.len();
                let close = inner.out[after..].iter().position(|byte| *byte == 0)?;
                let status = std::str::from_utf8(&inner.out[after..after + close])
                    .ok()?
                    .parse::<i32>()
                    .ok()?;
                Some((position, status))
            });
            let err_end = find(&inner.err, &err_marker);
            if let (Some((out_position, status)), Some(err_position)) = (out_end, err_end) {
                let stdout = inner.out[..out_position].to_vec();
                let stderr = String::from_utf8_lossy(&inner.err[..err_position]).into_owned();
                inner.out.clear();
                inner.err.clear();
                return Ok(Output {
                    success: status == 0,
                    stdout,
                    stderr,
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match inner.messages.recv_timeout(remaining) {
                Ok(Message::Out(bytes)) => inner.out.extend(bytes),
                Ok(Message::Err(bytes)) => inner.err.extend(bytes),
                Ok(Message::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    inner.kill();
                    return Err(Error::Dead);
                }
                Err(RecvTimeoutError::Timeout) => {
                    inner.kill();
                    return Err(Error::Timeout);
                }
            }
        }
    }
}

impl Inner {
    /// 作废：杀掉进程，之后的请求都是 `Dead`
    fn kill(&mut self) {
        self.dead = true;
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(10);

    /// 本机的 `sh` 当「远端」
    fn local() -> Channel {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sh"]);
        Channel::start(command).expect("起得来")
    }

    #[test]
    fn a_request_returns_stdout_stderr_and_the_exit_status() {
        let channel = local();
        let ok = channel
            .request("echo hello; echo oops >&2", LONG)
            .expect("跑得了");
        assert_eq!(
            ok,
            Output {
                success: true,
                stdout: b"hello\n".to_vec(),
                stderr: "oops\n".into()
            }
        );
        let failed = channel
            .request("echo partial; exit 3", LONG)
            .expect("跑得了");
        assert!(!failed.success);
        assert_eq!(failed.stdout, b"partial\n");
    }

    /// 一条通道连续跑很多个请求，每个都对得上（不错位）
    #[test]
    fn many_requests_in_a_row_stay_aligned() {
        let channel = local();
        for number in 0..200 {
            let output = channel
                .request(&format!("echo {number}; echo e{number} >&2"), LONG)
                .expect("跑得了");
            assert_eq!(output.stdout, format!("{number}\n").into_bytes());
            assert_eq!(output.stderr, format!("e{number}\n"));
        }
    }

    /// 每个请求在子 shell 里：`cd`、变量不会留到下一个请求
    #[test]
    fn requests_do_not_leak_state_into_each_other() {
        let channel = local();
        channel
            .request("cd /; FOO=bar; export FOO", LONG)
            .expect("跑得了");
        let output = channel
            .request("pwd; echo \"[$FOO]\"", LONG)
            .expect("跑得了");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!text.starts_with("/\n"), "cd 没有留下来：{text}");
        assert!(text.contains("[]"), "变量没有留下来：{text}");
    }

    /// 脚本里读标准输入的命令吃不到协议行（标准输入接到 /dev/null）
    #[test]
    fn a_script_that_reads_stdin_does_not_eat_the_protocol() {
        let channel = local();
        let output = channel.request("cat; echo after", LONG).expect("跑得了");
        assert_eq!(output.stdout, b"after\n");
        assert!(
            channel
                .request("echo still-fine", LONG)
                .expect("还能用")
                .success
        );
    }

    /// 输出里带 NUL、换行、伪造的结束标记：都当作数据，不会提前结束，也不会冒充别的请求
    #[test]
    fn hostile_output_cannot_end_or_forge_a_request() {
        let channel = local();
        let output = channel
            .request(
                "printf 'a\\000b\\n@@NIMBUS-0000-1@@0\\000\\000@@NIMBUS-x-9@@\\n'; echo done",
                LONG,
            )
            .expect("跑得了");
        assert_eq!(
            output.stdout,
            b"a\0b\n@@NIMBUS-0000-1@@0\0\0@@NIMBUS-x-9@@\ndone\n"
        );
        assert!(channel.request("echo next", LONG).expect("还能用").success);
    }

    /// 大输出（几 MB）不卡、不丢
    #[test]
    fn large_output_comes_back_whole() {
        let channel = local();
        let output = channel
            .request("yes 0123456789abcdefghij | head -c 5000000", LONG)
            .expect("跑得了");
        assert_eq!(output.stdout.len(), 5_000_000);
        assert!(channel.request("echo ok", LONG).expect("还能用").success);
    }

    /// 远端 shell 启动时往标准输出打的字（坏的 .bashrc）被起通道时那次空请求吃掉，不混进第一个请求
    #[test]
    fn startup_noise_does_not_leak_into_the_first_request() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "echo NOISE-ON-STDOUT; echo NOISE-ON-STDERR >&2; exec sh",
        ]);
        let channel = Channel::start(command).expect("起得来");
        let output = channel.request("echo clean", LONG).expect("跑得了");
        assert_eq!(output.stdout, b"clean\n");
        assert_eq!(output.stderr, "");
    }

    /// 超时：这条通道作废（不然下一个请求会读到上一个请求迟到的输出），之后是 Dead
    #[test]
    fn a_timeout_kills_the_channel() {
        let channel = local();
        assert_eq!(
            channel.request("sleep 5", Duration::from_millis(200)),
            Err(Error::Timeout)
        );
        assert_eq!(channel.request("echo hi", LONG), Err(Error::Dead));
    }

    /// 远端进程没了：Dead
    #[test]
    fn a_dead_process_is_reported() {
        let channel = local();
        assert_eq!(channel.request("kill -9 $$", LONG), Err(Error::Dead));
        assert_eq!(channel.request("echo hi", LONG), Err(Error::Dead));
        let mut command = Command::new("sh");
        command.args(["-c", "exit 1"]);
        assert!(matches!(Channel::start(command), Err(Error::Dead)));
        assert!(matches!(
            Channel::start(Command::new("/nonexistent/program")),
            Err(Error::Dead)
        ));
    }

    /// 多个线程同时请求：互斥，各拿各的结果
    #[test]
    fn concurrent_requests_do_not_mix() {
        let channel = std::sync::Arc::new(local());
        let handles: Vec<_> = (0..8)
            .map(|number| {
                let channel = channel.clone();
                std::thread::spawn(move || {
                    for round in 0..25 {
                        let tag = format!("{number}-{round}");
                        let output = channel
                            .request(&format!("echo {tag}"), LONG)
                            .expect("跑得了");
                        assert_eq!(output.stdout, format!("{tag}\n").into_bytes());
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("线程没有 panic");
        }
    }
}
