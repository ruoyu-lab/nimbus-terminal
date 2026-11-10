//! 远端文件：列目录、路径运算、拼 `sftp` 批处理命令。这一层是纯函数，不碰进程和界面，所以能单独测；
//! 真的跑 `ssh`、`sftp` 在 [`crate::ssh`]。
//!
//! 和监控一样借终端那条 SSH 连接，所以服务器要是 Linux（列目录用 GNU `find -printf`）。
//! 传输用 `sftp` 的批处理（`-b -`，命令从标准输入进）：`ssh` 把路径交给远端的登录 shell 解释，
//! 引号怎么写取决于那个 shell 是什么；`sftp` 批处理的引号规则是固定的，不管远端用什么 shell。
//!
//! 文件名是不可信的输入（远端什么名字都有）：带换行、控制字符的名字不进批处理（换行会变成另一条命令），
//! 路径一律先变成绝对路径（不会被当成 `sftp` 的选项）；存到 Windows 上时，那里文件名不能有的字换掉（反斜杠是分隔符）。

use std::path::{Path, PathBuf};

use crate::monitor::Failure;

/// 一个条目是什么
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    /// 管道、套接字、设备、断掉的符号链接……显示出来，但不能下载
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
}

/// 一个目录里有什么
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// 绝对路径（`pwd` 报的）
    pub path: String,
    /// 文件夹在前，各自按名字（不分大小写）排
    pub entries: Vec<Entry>,
}

/// 给 `sh` 的单引号字符串：整个包在单引号里，里面的单引号写成 `'\''`
pub fn quote_sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// 列目录的脚本（从标准输入交给远端的 `sh -s`，所以路径只需要照 `sh` 的规矩引起来，不用管登录 shell）。
/// 输出：先是 `@@绝对路径`，然后每个条目一条 `类型 TAB 大小 TAB 名字`，记录之间用 NUL 隔开
/// （文件名里什么字符都可能有，包括换行）。`%Y` 是跟着符号链接走之后的类型。`dir` 为 `None` 是主目录
pub fn list_script(dir: Option<&str>) -> String {
    let cd = match dir {
        Some(dir) => format!("cd -- {} || exit 3", quote_sh(dir)),
        None => "cd || exit 3".to_string(),
    };
    format!(
        "{cd}\nprintf '@@%s\\000' \"$(pwd)\"\nfind . -maxdepth 1 -mindepth 1 -printf '%Y\\t%s\\t%f\\000'\n"
    )
}

/// 解析列目录脚本的输出。没有 `@@` 开头的记录算 [`Failure::Failed`]
pub fn parse_listing(output: &[u8]) -> Result<Listing, Failure> {
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let path = records
        .next()
        .and_then(|record| record.strip_prefix(b"@@"))
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| Failure::Failed("看不懂服务器返回的目录".into()))?;
    let mut entries = Vec::new();
    for record in records {
        let text = String::from_utf8_lossy(record);
        let mut fields = text.splitn(3, '\t');
        let (Some(kind), Some(size), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        entries.push(Entry {
            name: name.to_string(),
            kind: match kind {
                "d" => Kind::Dir,
                "f" => Kind::File,
                _ => Kind::Other,
            },
            size: size.parse().unwrap_or(0),
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != Kind::Dir)
            .cmp(&(b.kind != Kind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Listing { path, entries })
}

/// 列目录失败时的错误输出：`find` 不认 `-printf`（BusyBox、BSD、macOS）说明不是 GNU 环境
pub fn classify_list_error(errors: &str) -> Failure {
    let lowered = errors.to_lowercase();
    if lowered.contains("printf")
        || lowered.contains("unknown predicate")
        || lowered.contains("unrecognized")
    {
        return Failure::Unsupported;
    }
    let reason = errors
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("读不了这个目录")
        .trim();
    Failure::Failed(reason.chars().take(120).collect())
}

/// 目录里的一个名字能不能拿来传输：不能是空的、`.`、`..`，不能带 `/`，不能有控制字符（换行会变成另一条命令）
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

/// 远端路径：目录加名字。目录是 `/` 时不重复斜杠
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 上一级。根目录的上一级还是根目录
pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => trimmed[..index].to_string(),
    }
}

/// 拖进终端、粘进命令行的路径：只有字母数字和 `/ . _ - + @ : , = %` 的原样写（`ls /var/log` 看着自然），
/// 别的整个用单引号引起来。带控制字符（尤其是换行）的返回 `None`：换行写进终端就等于按了回车，
/// 远端的文件名想让你执行什么就执行什么
pub fn shell_word(path: &str) -> Option<String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let plain = path.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '/' | '.' | '_' | '-' | '+' | '@' | ':' | ',' | '=' | '%'
            )
    });
    Some(if plain {
        path.to_string()
    } else {
        quote_sh(path)
    })
}

/// 本机 shell 引路径的规矩（从访达、资源管理器拖文件进本机终端时用）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalShell {
    /// `sh` 一系（zsh、bash、fish……，Windows 上的 Git Bash、WSL 也是）：单引号，见 [`shell_word`]
    Sh,
    /// PowerShell：单引号，里面的单引号写两遍
    PowerShell,
    /// cmd：双引号
    Cmd,
}

impl LocalShell {
    /// 看程序名认是哪种 shell（`C:\…\pwsh.exe`、`/bin/zsh` 都行，不分大小写）。
    /// 认不出来的按 cmd 的规矩：Windows 上的程序大多按双引号拆参数（只在 Windows 上这么认）
    pub fn of_program(program: &str) -> Self {
        let name = program
            .trim_matches('"')
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match name.strip_suffix(".exe").unwrap_or(&name) {
            "pwsh" | "pwsh-preview" | "powershell" => Self::PowerShell,
            "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "wsl" => Self::Sh,
            _ => Self::Cmd,
        }
    }
}

/// PowerShell 的单引号字符串：里面的单引号写两遍。PowerShell 把弯的 `‘ ’ ‚ ‛` 也当单引号（不写两遍的话字符串就在那里断了，
/// 后半截变成命令），一样写两遍
fn quote_powershell(text: &str) -> Option<String> {
    let mut quoted = String::from("'");
    for ch in text.chars() {
        if matches!(ch, '\'' | '‘' | '’' | '‚' | '‛') {
            quoted.push(ch);
        }
        quoted.push(ch);
    }
    quoted.push('\'');
    Some(quoted)
}

/// cmd 的双引号。`"` 本来就不会出现在 Windows 的文件名里；有的话不写
fn quote_cmd(text: &str) -> Option<String> {
    (!text.contains('"')).then(|| format!("\"{text}\""))
}

/// 拖进本机终端的路径，按 `shell` 的规矩引好；带控制字符的返回 `None`（同 [`shell_word`]）。
///
/// cmd、PowerShell：只有字母数字和 `\ / . _ - : + ~` 的原样写。别的先看双引号行不行：没有 `$`、`` ` ``、`%`、`!`、
/// 弯双引号的，cmd 和 PowerShell 在双引号里都不展开，两边写法一样——起的是 cmd、里面又开了 PowerShell（或者反过来）
/// 也还是对的，`Program Files (x86)`、`&`、单引号都不怕。剩下的才按各自的规矩：PowerShell 用单引号
/// （双引号里 `$(…)` 会被执行），cmd 用双引号（cmd 的双引号里 `%名字%` 没法不展开，只是没有这个变量时原样留着）
pub fn local_word(path: &str, shell: LocalShell) -> Option<String> {
    let quote: fn(&str) -> Option<String> = match shell {
        LocalShell::Sh => return shell_word(path),
        LocalShell::PowerShell => quote_powershell,
        LocalShell::Cmd => quote_cmd,
    };
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let plain = path.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '\\' | '/' | '.' | '_' | '-' | ':' | '+' | '~')
    });
    if plain {
        return Some(path.to_string());
    }
    let expands = path
        .chars()
        .any(|ch| matches!(ch, '$' | '`' | '%' | '!' | '"' | '“' | '”' | '„'));
    if expands {
        quote(path)
    } else {
        quote_cmd(path)
    }
}

/// 在远端目录里开一个登录 shell 的命令（`ssh` 的最后一个参数）。
/// 整条交给远端用户的登录 shell 解释，所以外面包一层 `sh -c '…'`，里面再按 `sh` 的规矩引目录：
/// 目录进不去就报一句、留在主目录，不让会话一闪而过。目录必须是绝对路径、不能有控制字符
pub fn login_command(directory: &str) -> Option<String> {
    if !directory.starts_with('/') || directory.chars().any(char::is_control) {
        return None;
    }
    let script = format!(
        "cd -- {} 2>/dev/null || echo 'nimbus: 进不了这个目录' >&2; exec \"${{SHELL:-/bin/sh}}\" -l",
        quote_sh(directory)
    );
    Some(format!("sh -c {}", quote_sh(&script)))
}

/// `sftp` 批处理里的一个参数：双引号包起来，里面的 `\` 和 `"` 前面加反斜杠。
/// 实测（本机的 sftp-server）：`* ? [` 在引号里不会被展开成别的文件，所以不用转义——转义了反而找不到文件。
/// 带控制字符（换行、制表……）的路径返回 `None`：批处理是一行一条命令，换行会让后半截变成另一条命令
pub fn quote_sftp(path: &str) -> Option<String> {
    if path.chars().any(char::is_control) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// 本地路径在 `sftp` 批处理里的写法。必须从根目录起（`/…`；Windows 上 `C:\…`、`\\服务器\共享\…`）：
/// 不然以 `-` 开头的名字会被 `sftp` 当成选项，从根目录起的路径不会以 `-` 开头。
/// Windows 上 `canonicalize` 给的 `\\?\` 前缀先去掉，再换成正斜杠（`C:/Users/…`）：Windows 自带的 `sftp` 认这种写法，
/// 也不用在批处理里再给反斜杠转义一层。别的平台上反斜杠是文件名里的普通字符，原样
fn sftp_local(local: &Path) -> Option<String> {
    if !local.has_root() {
        return None;
    }
    let local = crate::links::without_verbatim(local.to_path_buf());
    let text = local.to_str()?;
    Some(if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.to_string()
    })
}

/// 下载一个条目到本地 `local` 的批处理。`remote` 得是绝对路径（以 `/` 开头），`local` 见 [`sftp_local`]：
/// 不然以 `-` 开头的名字会被 `sftp` 当成选项。目录用 `-r`
pub fn sftp_get(remote: &str, local: &Path, directory: bool) -> Option<String> {
    let local = sftp_local(local)?;
    if !remote.starts_with('/') {
        return None;
    }
    let flag = if directory { "-r " } else { "" };
    Some(format!(
        "get {flag}{} {}\n",
        quote_sftp(remote)?,
        quote_sftp(&local)?
    ))
}

/// 上传本地 `local` 到远端目录 `remote_dir` 里（名字不变）的批处理。目录用 `-r`
pub fn sftp_put(local: &Path, remote_dir: &str, directory: bool) -> Option<String> {
    let local = sftp_local(local)?;
    if !remote_dir.starts_with('/') {
        return None;
    }
    let flag = if directory { "-r " } else { "" };
    // 结尾的 `/`：目的地是个目录，文件放进去
    let target = if remote_dir.ends_with('/') {
        remote_dir.to_string()
    } else {
        format!("{remote_dir}/")
    };
    Some(format!(
        "put {flag}{} {}\n",
        quote_sftp(&local)?,
        quote_sftp(&target)?
    ))
}

/// 远端的名字在 Windows 上当本地文件名：`\ / : * ? " < > |` 换成 `_`（`\` 是分隔符，`..\..\x`、`C:\x` 会跑出下载目录；
/// `:` 会写进 NTFS 的备用数据流），结尾的点和空格去掉（系统会悄悄去掉，存成的名字就对不上了），
/// `CON`、`NUL`、`COM1` 这类设备名前面加 `_`（不然写进去的是设备）。去掉之后是空的就叫 `_`
fn windows_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = replaced.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        return "_".to_string();
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            rest.chars().count() == 1
                && rest
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || matches!(ch, '¹' | '²' | '³'))
        })
    });
    if numbered
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
    {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// 本地要存成什么名字：`name` 在 `directory` 里已经有了就加 ` (1)`、` (2)`……（扩展名前面），不覆盖本地的文件。
/// Windows 上先把名字换成那里能用的（见 [`windows_name`]），远端的名字跑不出 `directory`
pub fn unique_local(directory: &Path, name: &str) -> PathBuf {
    let name = if cfg!(windows) {
        windows_name(name)
    } else {
        name.to_string()
    };
    let name = name.as_str();
    let first = directory.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rfind('.').filter(|index| *index > 0) {
        Some(index) => (&name[..index], &name[index..]),
        None => (name, ""),
    };
    (1..)
        .map(|number| directory.join(format!("{stem} ({number}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

/// 容量：字节换成人看的（`1.2 MB`）
pub fn size(bytes: u64) -> String {
    crate::monitor::size(bytes.div_ceil(1024))
}

/// 文件页现在的状态
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// 还没读过
    #[default]
    Idle,
    Loading,
    Ready,
    /// 终端的连接不在：连上之后会自己再读
    NotConnected,
    Unsupported,
    Failed(String),
    /// 要用户输服务器的密码（见 [`crate::monitor::Failure::PasswordNeeded`]）
    PasswordNeeded {
        wrong: bool,
    },
}

/// 一次传输
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub id: u64,
    pub name: String,
    pub upload: bool,
    pub state: TransferState,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransferState {
    Running,
    /// 做完了，带一句结果（下载存到哪里）
    Done(String),
    Failed(String),
}

/// 最多留几条传输记录：太多会把面板占满
pub const TRANSFERS_KEPT: usize = 4;

/// 一个服务器标签的文件页：在哪个目录、里面有什么、读得怎么样、传输记录
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilesState {
    /// 现在在哪个目录（绝对路径）；还没读到时是 `None`
    pub path: Option<String>,
    pub entries: Vec<Entry>,
    pub status: Status,
    pub show_hidden: bool,
    pub transfers: Vec<Transfer>,
    /// 读目录的请求编号：回来的结果和现在的编号对不上（又点了别的目录）就丢掉，旧结果不能盖掉新的
    request: u64,
    /// 读过的目录（最近用的在后面）：再去的时候先显示这份，后台重新读，读回来有变化再换。
    /// 只在这个标签的这次连接里有效（标签关了、状态没了就没了），不落盘
    cache: Vec<(String, Listing)>,
    /// 远端主目录的绝对路径（第一次读 `~` 之后知道），`~` 也能用缓存
    pub home: Option<String>,
}

/// 最多缓存几个目录
pub const CACHE_KEPT: usize = 40;

impl FilesState {
    /// 开始读一个目录，返回这次请求的编号。`cached` 是这个目录以前读到的（有的话）：马上显示它，
    /// 状态是「读取中」（界面照常显示列表，后台在刷新），读回来再换。没有缓存就还是显示原来的目录
    pub fn begin_load(&mut self, cached: Option<Listing>) -> u64 {
        self.request += 1;
        self.status = Status::Loading;
        if let Some(listing) = cached {
            self.path = Some(listing.path);
            self.entries = listing.entries;
        }
        self.request
    }

    /// 这个目录的缓存。`path` 是绝对路径；`None` 是主目录（知道主目录是哪个之后才有）
    pub fn cached(&self, path: Option<&str>) -> Option<Listing> {
        let key = path.or(self.home.as_deref())?;
        self.cache
            .iter()
            .find(|(cached, _)| cached == key)
            .map(|(_, listing)| listing.clone())
    }

    fn remember(&mut self, listing: &Listing) {
        self.cache.retain(|(path, _)| *path != listing.path);
        self.cache.push((listing.path.clone(), listing.clone()));
        if self.cache.len() > CACHE_KEPT {
            self.cache.remove(0);
        }
    }

    /// 收下读目录的结果；编号过期了（后来又发了新的请求）返回 `false`，什么也不改。
    /// `asked_home`：这次读的是主目录（`~`），读到就知道主目录是哪个了
    pub fn finish_load(
        &mut self,
        request: u64,
        asked_home: bool,
        result: Result<Listing, Failure>,
    ) -> bool {
        if request != self.request {
            return false;
        }
        match result {
            Ok(listing) => {
                if asked_home {
                    self.home = Some(listing.path.clone());
                }
                self.remember(&listing);
                self.path = Some(listing.path);
                self.entries = listing.entries;
                self.status = Status::Ready;
            }
            Err(Failure::NotConnected) => self.status = Status::NotConnected,
            Err(Failure::Unsupported) => self.status = Status::Unsupported,
            Err(Failure::Failed(reason)) => self.status = Status::Failed(reason),
            Err(Failure::PasswordNeeded { wrong }) => {
                self.status = Status::PasswordNeeded { wrong };
            }
        }
        true
    }

    /// 要显示的条目：隐藏文件（点开头）默认不显示
    pub fn visible(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|entry| self.show_hidden || !entry.name.starts_with('.'))
            .cloned()
            .collect()
    }

    pub fn has_entry(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }

    pub fn start_transfer(&mut self, id: u64, name: &str, upload: bool) {
        self.transfers.push(Transfer {
            id,
            name: name.to_string(),
            upload,
            state: TransferState::Running,
        });
        self.trim_transfers();
    }

    /// 一次传输直接就失败了（还没开始传就知道不行）
    pub fn fail_transfer(&mut self, id: u64, name: &str, upload: bool, reason: &str) {
        self.transfers.push(Transfer {
            id,
            name: name.to_string(),
            upload,
            state: TransferState::Failed(reason.to_string()),
        });
        self.trim_transfers();
    }

    pub fn finish_transfer(&mut self, id: u64, state: TransferState) {
        if let Some(transfer) = self.transfers.iter_mut().find(|transfer| transfer.id == id) {
            transfer.state = state;
        }
    }

    /// 清掉做完的记录
    pub fn clear_finished(&mut self) {
        self.transfers
            .retain(|transfer| transfer.state == TransferState::Running);
    }

    /// 只留最近几条；还在传的不能丢（丢了就看不到它的结果了）
    fn trim_transfers(&mut self) {
        while self.transfers.len() > TRANSFERS_KEPT {
            match self
                .transfers
                .iter()
                .position(|transfer| transfer.state != TransferState::Running)
            {
                Some(index) => {
                    self.transfers.remove(index);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &[u8] = b"@@/home/deploy\0d\t4096\tsrc\0f\t120\tREADME.md\0f\t5\t.env\0d\t4096\t.cache\0f\t7\tzeta.txt\0f\t1\tAlpha.txt\0p\t0\tpipe\0l\t0\tdangling\0";

    #[test]
    fn a_listing_is_parsed_and_sorted_folders_first() {
        let listing = parse_listing(OUTPUT).expect("能解析");
        assert_eq!(listing.path, "/home/deploy");
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                ".cache",
                "src",
                ".env",
                "Alpha.txt",
                "dangling",
                "pipe",
                "README.md",
                "zeta.txt"
            ]
        );
        assert_eq!(listing.entries[1].kind, Kind::Dir);
        assert_eq!(listing.entries[4].kind, Kind::Other, "不认识的类型不能下载");
        assert_eq!(listing.entries[6].size, 120);
    }

    /// 文件名里什么都可能有：制表符、换行、非 UTF-8；记录用 NUL 隔开，所以不会错位
    #[test]
    fn awkward_names_survive_the_parse() {
        let output = b"@@/tmp\0f\t3\ta\tb\0f\t4\tline\nbreak\0f\t5\tbad\xff\0";
        let listing = parse_listing(output).expect("能解析");
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_eq!(names, ["a\tb", "bad\u{fffd}", "line\nbreak"]);
    }

    #[test]
    fn garbage_is_a_failure_and_an_empty_directory_is_fine() {
        assert!(matches!(parse_listing(b""), Err(Failure::Failed(_))));
        assert!(matches!(parse_listing(b"hello\0"), Err(Failure::Failed(_))));
        assert!(
            matches!(parse_listing(b"@@relative\0"), Err(Failure::Failed(_))),
            "路径必须是绝对的"
        );
        assert!(
            parse_listing(b"@@/empty\0")
                .expect("空目录")
                .entries
                .is_empty()
        );
    }

    #[test]
    fn find_without_printf_means_unsupported() {
        for errors in [
            "find: unknown predicate `-printf'",
            "find: -printf: unknown primary or operator",
            "find: unrecognized: -printf",
        ] {
            assert_eq!(
                classify_list_error(errors),
                Failure::Unsupported,
                "{errors}"
            );
        }
        assert_eq!(
            classify_list_error("sh: 1: cd: can't cd to /root: Permission denied\n"),
            Failure::Failed("sh: 1: cd: can't cd to /root: Permission denied".into())
        );
    }

    /// 脚本里的路径按 sh 的规矩引起来：单引号、空格、`$`、反引号都当字面
    #[test]
    fn the_listing_script_quotes_the_directory_for_sh() {
        assert_eq!(quote_sh("a b"), "'a b'");
        assert_eq!(quote_sh("it's"), "'it'\\''s'");
        let script = list_script(Some("/srv/$(id) `x` 'q'"));
        assert!(
            script.starts_with("cd -- '/srv/$(id) `x` '\\''q'\\''' || exit 3\n"),
            "{script}"
        );
        assert!(list_script(None).starts_with("cd || exit 3\n"));
        assert!(script.contains("find . -maxdepth 1 -mindepth 1 -printf"));
    }

    #[test]
    fn names_that_could_break_a_batch_are_unsafe() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "line\nbreak",
            "tab\there",
            "nul\0x",
            "\u{7f}",
        ] {
            assert!(!is_safe_name(name), "{name:?}");
        }
        for name in ["a b", "-rf", "中文.txt", "a\"b", "back\\slash", "[x]*?"] {
            assert!(is_safe_name(name), "{name:?}");
        }
    }

    #[test]
    fn paths_join_and_go_up() {
        assert_eq!(join("/home/a", "b"), "/home/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(parent("/home/a/b"), "/home/a");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/home/a/"), "/home");
    }

    #[test]
    fn sftp_arguments_escape_only_backslash_and_quote() {
        assert_eq!(quote_sftp("/a b/c").as_deref(), Some("\"/a b/c\""));
        assert_eq!(
            quote_sftp("/a\"b\\c*[?").as_deref(),
            Some("\"/a\\\"b\\\\c*[?\"")
        );
        assert_eq!(quote_sftp("/a\nb"), None, "换行不能进批处理");
        assert_eq!(quote_sftp("/a\tb"), None);
    }

    #[test]
    fn batches_need_absolute_paths_and_use_r_for_directories() {
        assert_eq!(
            sftp_get(
                "/srv/a b.txt",
                Path::new("/Users/x/Downloads/a b.txt"),
                false
            )
            .as_deref(),
            Some("get \"/srv/a b.txt\" \"/Users/x/Downloads/a b.txt\"\n")
        );
        assert_eq!(
            sftp_get("/srv/dir", Path::new("/tmp/dir"), true).as_deref(),
            Some("get -r \"/srv/dir\" \"/tmp/dir\"\n")
        );
        assert_eq!(
            sftp_get("-oProxyCommand=x", Path::new("/tmp/x"), false),
            None,
            "不是绝对路径：会被当成选项"
        );
        assert_eq!(sftp_get("/srv/a", Path::new("relative"), false), None);
        assert_eq!(
            sftp_put(Path::new("/Users/x/a.txt"), "/srv/up", false).as_deref(),
            Some("put \"/Users/x/a.txt\" \"/srv/up/\"\n")
        );
        assert_eq!(
            sftp_put(Path::new("/Users/x/d"), "/", true).as_deref(),
            Some("put -r \"/Users/x/d\" \"/\"\n")
        );
        assert_eq!(sftp_put(Path::new("/Users/x/a\nb"), "/srv", false), None);
        assert_eq!(sftp_put(Path::new("-x"), "/srv", false), None);
    }

    /// 本地路径按本机的 `sftp` 认的写：Windows 上盘符开头的、共享路径算绝对路径，换成正斜杠（批处理里不用再转义反斜杠），
    /// `\\?\` 原样路径先换回平常的，`C:foo` 不算；别的平台上反斜杠是文件名里的字，原样转义
    #[test]
    fn local_paths_are_written_the_way_the_local_sftp_reads_them() {
        if cfg!(windows) {
            assert_eq!(
                sftp_get(
                    "/srv/a b.txt",
                    Path::new(r"C:\Users\x\Downloads\a b.txt"),
                    false
                )
                .as_deref(),
                Some("get \"/srv/a b.txt\" \"C:/Users/x/Downloads/a b.txt\"\n")
            );
            assert_eq!(
                sftp_put(Path::new(r"\\?\D:\data\d"), "/srv", true).as_deref(),
                Some("put -r \"D:/data/d\" \"/srv/\"\n")
            );
            assert_eq!(
                sftp_put(Path::new(r"\\nas\share\a.txt"), "/srv", false).as_deref(),
                Some("put \"//nas/share/a.txt\" \"/srv/\"\n")
            );
            assert_eq!(
                sftp_get("/srv/a", Path::new("C:relative"), false),
                None,
                "盘符后面没有斜杠：不是绝对路径"
            );
            assert_eq!(sftp_put(Path::new(r"x\y"), "/srv", false), None);
        } else {
            assert_eq!(
                sftp_put(Path::new("/tmp/back\\slash"), "/srv", false).as_deref(),
                Some("put \"/tmp/back\\\\slash\" \"/srv/\"\n")
            );
        }
    }

    /// 本地已有同名文件就加序号，不覆盖；序号放在扩展名前面，隐藏文件的点不算扩展名
    #[test]
    fn local_names_never_overwrite() {
        let directory = crate::store::tests::scratch("unique-local");
        std::fs::create_dir_all(&directory).expect("建得了目录");
        assert_eq!(unique_local(&directory, "a.txt"), directory.join("a.txt"));
        std::fs::write(directory.join("a.txt"), "x").expect("写得了");
        assert_eq!(
            unique_local(&directory, "a.txt"),
            directory.join("a (1).txt")
        );
        std::fs::write(directory.join("a (1).txt"), "x").expect("写得了");
        assert_eq!(
            unique_local(&directory, "a.txt"),
            directory.join("a (2).txt")
        );
        std::fs::write(directory.join(".env"), "x").expect("写得了");
        assert_eq!(unique_local(&directory, ".env"), directory.join(".env (1)"));
        std::fs::write(directory.join("noext"), "x").expect("写得了");
        assert_eq!(
            unique_local(&directory, "noext"),
            directory.join("noext (1)")
        );
        if cfg!(windows) {
            assert_eq!(
                unique_local(&directory, r"..\..\evil.bat"),
                directory.join(".._.._evil.bat"),
                "远端的名字跑不出下载目录"
            );
        }
        std::fs::remove_dir_all(directory).ok();
    }

    /// 远端的名字在 Windows 上当本地文件名：分隔符、冒号这类换掉（跑不出下载目录、写不进备用数据流），
    /// 结尾的点和空格去掉，设备名前面加 `_`；平常的名字原样
    #[test]
    fn remote_names_are_made_safe_for_windows() {
        for (name, expected) in [
            ("a b.txt", "a b.txt"),
            ("中文.txt", "中文.txt"),
            (".env", ".env"),
            (r"..\..\evil.bat", ".._.._evil.bat"),
            (r"C:\Windows\x", "C__Windows_x"),
            ("a:stream", "a_stream"),
            ("what?<>|*\".txt", "what______.txt"),
            ("trailing. . ", "trailing"),
            ("...", "_"),
            ("con", "_con"),
            ("NUL.txt", "_NUL.txt"),
            ("com1.log", "_com1.log"),
            ("LPT¹", "_LPT¹"),
            ("console.txt", "console.txt"),
            ("com10", "com10"),
        ] {
            assert_eq!(windows_name(name), expected, "{name:?}");
        }
    }

    #[test]
    fn sizes_are_in_human_units() {
        assert_eq!(size(0), "0 KB");
        assert_eq!(size(1536), "2 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
    }

    fn listing(path: &str, names: &[&str]) -> Listing {
        Listing {
            path: path.to_string(),
            entries: names
                .iter()
                .map(|name| Entry {
                    name: name.to_string(),
                    kind: Kind::File,
                    size: 1,
                })
                .collect(),
        }
    }

    /// 旧请求的结果不能盖掉新请求的：先点 A 再点 B，A 慢了一步回来，页面要停在 B
    #[test]
    fn a_stale_listing_is_dropped() {
        let mut state = FilesState::default();
        let first = state.begin_load(None);
        let second = state.begin_load(None);
        assert!(state.finish_load(second, false, Ok(listing("/b", &["b.txt"]))));
        assert!(
            !state.finish_load(first, false, Ok(listing("/a", &["a.txt"]))),
            "过期的丢掉"
        );
        assert_eq!(state.path.as_deref(), Some("/b"));
        assert_eq!(state.status, Status::Ready);
        let third = state.begin_load(None);
        assert_eq!(state.status, Status::Loading);
        assert!(state.finish_load(third, false, Err(Failure::NotConnected)));
        assert_eq!(state.status, Status::NotConnected);
        assert_eq!(
            state.path.as_deref(),
            Some("/b"),
            "失败时还停在原来的目录，条目也在"
        );
        let fourth = state.begin_load(None);
        state.finish_load(fourth, false, Err(Failure::Unsupported));
        assert_eq!(state.status, Status::Unsupported);
        let fifth = state.begin_load(None);
        state.finish_load(fifth, false, Err(Failure::Failed("没权限".into())));
        assert_eq!(state.status, Status::Failed("没权限".into()));
    }

    #[test]
    fn hidden_files_are_hidden_until_asked() {
        let mut state = FilesState::default();
        let request = state.begin_load(None);
        state.finish_load(request, false, Ok(listing("/", &[".env", "a.txt"])));
        assert_eq!(state.visible().len(), 1);
        state.show_hidden = true;
        assert_eq!(state.visible().len(), 2);
        assert!(state.has_entry(".env") && !state.has_entry("nope"));
    }

    /// 读过的目录有缓存：再去的时候马上显示旧的，读回来换成新的；`~` 知道主目录是哪个之后也有缓存；
    /// 缓存有上限，最久没用的先丢；读失败不污染缓存
    #[test]
    fn visited_directories_are_cached_and_shown_instantly() {
        let mut state = FilesState::default();
        let home = state.begin_load(None);
        state.finish_load(home, true, Ok(listing("/home/me", &["a.txt"])));
        assert_eq!(state.home.as_deref(), Some("/home/me"));
        let sub = state.begin_load(None);
        state.finish_load(sub, false, Ok(listing("/home/me/src", &["main.rs"])));
        // 回主目录（用 `~`，也用绝对路径）：先显示缓存，状态是读取中
        let cached = state.cached(None).expect("主目录有缓存");
        assert_eq!(cached.entries[0].name, "a.txt");
        let request = state.begin_load(Some(cached));
        assert_eq!(
            state.path.as_deref(),
            Some("/home/me"),
            "马上换成缓存的目录"
        );
        assert_eq!(state.entries[0].name, "a.txt");
        assert_eq!(state.status, Status::Loading);
        // 读回来有变化：换成新的，缓存也更新
        state.finish_load(request, true, Ok(listing("/home/me", &["a.txt", "b.txt"])));
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.cached(Some("/home/me")).expect("有").entries.len(), 2);
        // 刷新失败：列表还是缓存的那份，缓存不变
        let request = state.begin_load(state.cached(Some("/home/me/src")));
        state.finish_load(request, false, Err(Failure::Failed("超时".into())));
        assert_eq!(state.path.as_deref(), Some("/home/me/src"));
        assert_eq!(
            state.cached(Some("/home/me/src")).expect("有").entries[0].name,
            "main.rs"
        );
        assert_eq!(state.cached(Some("/nope")), None);
    }

    #[test]
    fn the_cache_is_bounded_and_drops_the_least_recently_used() {
        let mut state = FilesState::default();
        for index in 0..CACHE_KEPT + 10 {
            let request = state.begin_load(None);
            state.finish_load(request, false, Ok(listing(&format!("/d{index}"), &["x"])));
        }
        assert!(state.cached(Some("/d0")).is_none(), "最久没用的丢了");
        assert!(
            state
                .cached(Some(&format!("/d{}", CACHE_KEPT + 9)))
                .is_some()
        );
        // 再访问 /d10 让它变成最近用的，然后继续塞：它不会被丢
        let request = state.begin_load(None);
        state.finish_load(request, false, Ok(listing("/d10", &["x"])));
        for index in 0..CACHE_KEPT - 1 {
            let request = state.begin_load(None);
            state.finish_load(request, false, Ok(listing(&format!("/e{index}"), &["x"])));
        }
        assert!(state.cached(Some("/d10")).is_some(), "刚用过的还在");
    }

    /// 传输记录只留最近几条，而且还在传的不会被挤掉
    #[test]
    fn transfer_records_are_trimmed_but_running_ones_stay() {
        let mut state = FilesState::default();
        state.start_transfer(1, "running.bin", false);
        for id in 2..=8 {
            state.start_transfer(id, "x", true);
            state.finish_transfer(id, TransferState::Done("ok".into()));
        }
        assert_eq!(state.transfers.len(), TRANSFERS_KEPT);
        assert_eq!(state.transfers[0].id, 1, "还在传的留着");
        assert_eq!(state.transfers.last().map(|transfer| transfer.id), Some(8));
        state.fail_transfer(9, "bad", true, "同名");
        state.clear_finished();
        assert_eq!(
            state
                .transfers
                .iter()
                .map(|transfer| transfer.id)
                .collect::<Vec<_>>(),
            [1],
            "清掉做完的，传输中的留着"
        );
    }

    /// 拖进终端的路径：平常的原样，有空格、引号、美元符的整个引起来；带换行的不给（会变成回车）
    #[test]
    fn dragged_paths_are_quoted_for_the_shell_and_newlines_are_refused() {
        assert_eq!(
            shell_word("/var/log/nginx.log").as_deref(),
            Some("/var/log/nginx.log")
        );
        assert_eq!(shell_word("/srv/a b").as_deref(), Some("'/srv/a b'"));
        assert_eq!(shell_word("/srv/$(id)").as_deref(), Some("'/srv/$(id)'"));
        assert_eq!(shell_word("/srv/it's").as_deref(), Some("'/srv/it'\\''s'"));
        assert_eq!(
            shell_word("/srv/;rm -rf ~").as_deref(),
            Some("'/srv/;rm -rf ~'")
        );
        assert_eq!(
            shell_word("/srv/中文").as_deref(),
            Some("'/srv/中文'"),
            "非 ASCII 也引起来，稳妥"
        );
        for bad in [
            "",
            "/srv/a\nrm -rf ~",
            "/srv/a\r",
            "/srv/a\tb",
            "/srv/\u{1b}[31m",
        ] {
            assert_eq!(shell_word(bad), None, "{bad:?}");
        }
    }

    /// 认得出常见的 shell：带不带目录、`.exe`、大小写都一样；认不出来的按 cmd 的规矩
    #[test]
    fn the_local_shell_is_known_by_its_program_name() {
        for (program, shell) in [
            (
                r"C:\Program Files\PowerShell\7\pwsh.exe",
                LocalShell::PowerShell,
            ),
            (
                r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe",
                LocalShell::PowerShell,
            ),
            (
                "\"C:\\Program Files\\PowerShell\\7\\pwsh.exe\"",
                LocalShell::PowerShell,
            ),
            ("pwsh", LocalShell::PowerShell),
            (r"C:\WINDOWS\system32\cmd.exe", LocalShell::Cmd),
            ("CMD.EXE", LocalShell::Cmd),
            (r"C:\Program Files\Git\bin\bash.exe", LocalShell::Sh),
            ("wsl.exe", LocalShell::Sh),
            ("/bin/zsh", LocalShell::Sh),
            ("nu.exe", LocalShell::Cmd),
        ] {
            assert_eq!(LocalShell::of_program(program), shell, "{program}");
        }
    }

    /// 拖进本机终端的路径：平常的原样；一般的怪名字（空格、括号、`&`、单引号、中文）cmd 和 PowerShell 都写成双引号，
    /// 两边都对；带 `$`、`%` 这类双引号里会展开的，才按各自的规矩：PowerShell 单引号（弯的单引号也写两遍），cmd 双引号
    #[test]
    fn local_paths_are_quoted_for_the_local_shell() {
        use LocalShell::{Cmd, PowerShell, Sh};
        for shell in [Cmd, PowerShell] {
            for (path, expected) in [
                (r"C:\Users\me\notes.txt", r"C:\Users\me\notes.txt"),
                (r"\\nas\share\a-b_c+1~2.log", r"\\nas\share\a-b_c+1~2.log"),
                (
                    r"C:\Program Files (x86)\a b.txt",
                    r#""C:\Program Files (x86)\a b.txt""#,
                ),
                (r"C:\a&calc.txt", r#""C:\a&calc.txt""#),
                (r"C:\it's;x,y=z.txt", r#""C:\it's;x,y=z.txt""#),
                (r"C:\中文\笔记.md", r#""C:\中文\笔记.md""#),
            ] {
                assert_eq!(
                    local_word(path, shell).as_deref(),
                    Some(expected),
                    "{shell:?} {path}"
                );
            }
            for bad in ["", "C:\\a\nb", "C:\\a\r", "C:\\a\u{1b}[31m"] {
                assert_eq!(local_word(bad, shell), None, "{shell:?} {bad:?}");
            }
        }
        assert_eq!(
            local_word(r"C:\$(calc)`x\100%.txt", PowerShell).as_deref(),
            Some(r"'C:\$(calc)`x\100%.txt'")
        );
        assert_eq!(
            local_word("C:\\it's $x.txt", PowerShell).as_deref(),
            Some("'C:\\it''s $x.txt'")
        );
        assert_eq!(
            local_word("C:\\don’t $x.txt", PowerShell).as_deref(),
            Some("'C:\\don’’t $x.txt'"),
            "弯的单引号 PowerShell 也认，一样写两遍"
        );
        assert_eq!(
            local_word(r"\\srv\c$\a b", Cmd).as_deref(),
            Some(r#""\\srv\c$\a b""#)
        );
        assert_eq!(
            local_word(r"C:\100%.txt", Cmd).as_deref(),
            Some(r#""C:\100%.txt""#)
        );
        // `sh` 一系还是 `sh` 的规矩
        assert_eq!(
            local_word("/srv/it's", Sh).as_deref(),
            Some("'/srv/it'\\''s'")
        );
        assert_eq!(
            local_word(r"C:\Users\me", Sh).as_deref(),
            Some(r"'C:\Users\me'")
        );
    }

    /// 起始目录的命令：外面 `sh -c` 单引号一层，里面目录再引一层；不是绝对路径、带控制字符的不要
    #[test]
    fn the_login_command_quotes_twice_and_needs_an_absolute_directory() {
        let command = login_command("/srv/my app").expect("能用");
        assert!(
            command.starts_with("sh -c '") && command.ends_with('\''),
            "{command}"
        );
        assert!(command.contains("exec \"${SHELL:-/bin/sh}\" -l"));
        assert_eq!(login_command("relative"), None);
        assert_eq!(login_command("/srv/a\nb"), None);
        assert_eq!(login_command("-oProxyCommand=x"), None);
    }

    /// 真的用本机的 `sh` 展开一遍，确认怪目录名（空格、引号、美元符）最后到 `cd` 手里的就是原样的目录。
    /// 要本机有 `sh`、目录是 `/` 开头的（命令是给远端的 Unix 用的），所以只在 Unix 上跑
    #[cfg(unix)]
    #[test]
    fn the_login_command_survives_two_levels_of_quoting() {
        // 用 sh 把整条命令当作「登录 shell 收到的一行」再解释一遍：把最后的 exec 换成 pwd，看 cd 进了哪里
        let base = crate::store::tests::scratch("login-command");
        let directory = base.join("it's $(echo x) \"q\" a b");
        std::fs::create_dir_all(&directory).expect("建得了目录");
        let command = login_command(directory.to_str().expect("UTF-8"))
            .expect("能用")
            .replace("exec \"${SHELL:-/bin/sh}\" -l", "pwd -P");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .expect("跑得了");
        let printed = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(
            printed,
            directory
                .canonicalize()
                .expect("路径在")
                .to_str()
                .expect("UTF-8"),
            "{command}"
        );
        // 进不去的目录：报一句，不是一闪而过
        let missing = login_command("/nonexistent/nowhere")
            .expect("能用")
            .replace("exec \"${SHELL:-/bin/sh}\" -l", "echo still-here");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&missing)
            .output()
            .expect("跑得了");
        assert!(String::from_utf8_lossy(&output.stdout).contains("still-here"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("进不了"));
        std::fs::remove_dir_all(base).ok();
    }
}
