//! 远端操作的分发：界面只认识 [`Target`]，这里决定走 SSH（借终端的复用连接）还是 Docker（本机的 `docker` 命令）。
//! [`run`] 是两边共用的「跑一个外部命令」：收好输出、到点杀掉。

use std::path::{Path, PathBuf};

use crate::files::Listing;
use crate::monitor::{Failure, Reading};
use crate::target::Target;

/// 跑完一个外部命令的结果
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ran {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// 跑一个命令：`input` 写进它的标准输入，标准输出和错误输出各一个线程同时读
/// （输出大了，只等进程退出再读会把管道塞满、双方一起卡死），`timeout` 到了就杀掉
pub(crate) fn run(
    mut command: std::process::Command,
    input: Option<Vec<u8>>,
    timeout: Option<std::time::Duration>,
) -> Result<Ran, Failure> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::time::Instant;

    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failure::Failed(format!("起不了命令：{error}")))?;
    let writer = input.zip(child.stdin.take()).map(|(input, mut stdin)| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let reader = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = child.stdout.take().map(|stream| reader(Box::new(stream)));
    let stderr = child.stderr.take().map(|stream| reader(Box::new(stream)));
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if deadline.is_none_or(|deadline| Instant::now() < deadline) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Failed("命令没有及时回应".into()));
            }
            Err(error) => return Err(Failure::Failed(error.to_string())),
        }
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let stdout = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    Ok(Ran {
        success: status.success(),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// 界面用的远端操作。平时按目标走 SSH 或 Docker；测试里换成假的，不用真的连
pub trait Remote: Send + Sync {
    fn list(
        &self,
        target: &Target,
        control: Option<&Path>,
        dir: Option<&str>,
    ) -> Result<Listing, Failure>;
    fn download(
        &self,
        target: &Target,
        control: Option<&Path>,
        remote: &str,
        directory: bool,
        into: &Path,
    ) -> Result<PathBuf, Failure>;
    fn upload(
        &self,
        target: &Target,
        control: Option<&Path>,
        local: &Path,
        remote_dir: &str,
    ) -> Result<(), Failure>;
}

/// 真的远端：SSH 借终端的复用连接（套接字不在就是没连上），Docker 用本机的 `docker` 命令
pub struct OverRemote;

impl OverRemote {
    fn ssh(host: &str, control: Option<&Path>) -> Result<crate::ssh::Transport, Failure> {
        Ok(crate::ssh::Transport::Ssh {
            host: host.to_string(),
            control: control.ok_or(Failure::NotConnected)?.to_path_buf(),
        })
    }
}

impl Remote for OverRemote {
    fn list(
        &self,
        target: &Target,
        control: Option<&Path>,
        dir: Option<&str>,
    ) -> Result<Listing, Failure> {
        match target {
            Target::Ssh { host } => crate::ssh::list_via(&Self::ssh(host, control)?, dir),
            Target::Docker { container } => crate::docker::list(container, dir),
            Target::Local => Err(Failure::NotConnected),
        }
    }

    fn download(
        &self,
        target: &Target,
        control: Option<&Path>,
        remote: &str,
        directory: bool,
        into: &Path,
    ) -> Result<PathBuf, Failure> {
        match target {
            Target::Ssh { host } => {
                crate::ssh::download_via(&Self::ssh(host, control)?, remote, directory, into)
            }
            Target::Docker { container } => {
                crate::docker::download(container, remote, directory, into)
            }
            Target::Local => Err(Failure::NotConnected),
        }
    }

    fn upload(
        &self,
        target: &Target,
        control: Option<&Path>,
        local: &Path,
        remote_dir: &str,
    ) -> Result<(), Failure> {
        match target {
            Target::Ssh { host } => {
                crate::ssh::upload_via(&Self::ssh(host, control)?, local, remote_dir)
            }
            Target::Docker { container } => crate::docker::upload(container, local, remote_dir),
            Target::Local => Err(Failure::NotConnected),
        }
    }
}

/// 取一次监控读数：SSH 在远端读 `/proc`，Docker 用 `docker stats` 和 `docker inspect`
pub fn poll(target: &Target, control: Option<&Path>) -> Result<Reading, Failure> {
    match target {
        Target::Ssh { host } => crate::monitor::parse(&crate::ssh::poll(
            host,
            control.ok_or(Failure::NotConnected)?,
        )?),
        Target::Docker { container } => crate::docker::poll(container),
        Target::Local => Err(Failure::NotConnected),
    }
}
