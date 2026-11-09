//! 连到本机 Docker 里的容器：用系统的 `docker` 命令，终端是 `docker exec -it` 放进 pty 里跑。
//!
//! 不自己实现 Docker API：认证、远程 daemon（`DOCKER_HOST`、context）全沿用用户现有的 docker 配置。
//! 这里只做：把容器名变成命令行（防止被当成选项）、列出运行中的容器、判断一个名字能不能用。
//! 面板的常驻通道是 `docker exec -i`（[`crate::channel`]，按容器名共用），传输用 `docker cp`，
//! 监控用 `docker stats` 加 `docker inspect`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::channel::Channel;
use crate::files::{self, Listing};
use crate::monitor::{CpuSample, Failure, Reading};
use crate::pty;
use crate::remote::{Ran, run};

/// 容器名最长多少（Docker 自己的上限是 255）
const NAME_MAX: usize = 255;

/// 这个名字能不能交给 `docker`。只认字母数字和 `. _ -`，不能以 `-` 开头（不然会被当成选项）。
/// 容器 ID（十六进制）也在这套字符里
pub fn is_valid_container(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// 用户填的容器名：去掉首尾空白，能用就返回
pub fn normalize(name: &str) -> Option<String> {
    let name = name.trim();
    is_valid_container(name).then(|| name.to_string())
}

/// 连 `container` 要起的命令：`docker exec -it -- <container> sh -c '…'`。`--` 把容器名和选项隔开。
/// 进去之后有 bash 用 bash，没有就用 sh（都是登录 shell，和 SSH 一致）。
/// `directory` 是要进的目录（容器里的绝对路径）：进不去就在终端里留一句，shell 照常起
pub fn config(container: &str, columns: u16, rows: u16, directory: Option<&str>) -> pty::Config {
    let mut script = String::new();
    if let Some(directory) =
        directory.filter(|dir| dir.starts_with('/') && !dir.chars().any(char::is_control))
    {
        script.push_str(&format!(
            "cd -- {} 2>/dev/null || echo 'nimbus: 进不了这个目录' >&2; ",
            files::quote_sh(directory)
        ));
    }
    script.push_str("if command -v bash >/dev/null 2>&1; then exec bash -l; else exec sh -l; fi");
    let arguments = vec![
        "exec".into(),
        "-it".into(),
        "--".into(),
        container.into(),
        "sh".into(),
        "-c".into(),
        script,
    ];
    pty::Config::command("docker", arguments, columns, rows)
}

/// `docker ps` 的输出：一行一个名字。去掉重复的，不能用的跳过（不交给后面的命令）
pub fn parse_containers(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        let name = line.trim();
        if is_valid_container(name) && !names.iter().any(|known| known == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// 现在有哪些运行中的容器。docker 不在、daemon 没起都是 [`Failure::Failed`]（侧栏当作没有容器）
pub fn list_containers() -> Result<Vec<String>, Failure> {
    let mut command = Command::new("docker");
    command.args(["ps", "--format", "{{.Names}}"]);
    let ran = run(command, None, Some(Duration::from_secs(5)))?;
    if ran.success {
        Ok(parse_containers(&String::from_utf8_lossy(&ran.stdout)))
    } else {
        Err(Failure::Failed(first_reason(&ran.stderr)))
    }
}

/// 每个容器的常驻通道，按容器名找（同一个容器开几个标签共用一条）
static CHANNELS: LazyLock<Mutex<HashMap<String, Arc<Channel>>>> = LazyLock::new(Default::default);

fn channels() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Channel>>> {
    CHANNELS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 开一条通道：`docker exec -i -- <container> sh`，容器里的 `sh` 一直开着。
/// 起不来先看容器还在不在跑：不在跑（停了、被删了）是 `NotConnected`
fn start_channel(container: &str) -> Result<Channel, Failure> {
    let mut command = Command::new("docker");
    command.args(["exec", "-i", "--", container, "sh"]);
    Channel::start(command).map_err(|_| match inspect(container) {
        Ok((true, _)) => Failure::Failed("起不了通道".into()),
        _ => Failure::NotConnected,
    })
}

/// 在这个容器的常驻通道里跑一段 `sh` 脚本。没有通道就开一条；通道断了（容器重启过）换一条新的再试一次。
/// 超时：通道作废（下一次重开），这次报没有及时回应。容器不在跑是 `NotConnected`
fn run_in_container(container: &str, script: &str, timeout: Duration) -> Result<Ran, Failure> {
    use crate::channel::Error;
    if !is_valid_container(container) {
        return Err(Failure::Failed("容器名不能用".into()));
    }
    for _ in 0..2 {
        let existing = channels().get(container).cloned();
        let channel = match existing {
            Some(channel) => channel,
            None => {
                // 起通道要等 docker，不能占着登记表的锁
                let started = Arc::new(start_channel(container)?);
                channels()
                    .entry(container.to_string())
                    .or_insert(started)
                    .clone()
            }
        };
        match channel.request(script, timeout) {
            Ok(output) => {
                return Ok(Ran {
                    success: output.success,
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            Err(Error::Timeout) => {
                channels().remove(container);
                return Err(Failure::Failed("容器没有及时回应".into()));
            }
            Err(Error::Dead) => {
                channels().remove(container);
            }
        }
    }
    Err(match inspect(container) {
        Ok((true, _)) => Failure::Failed("和容器的通道断了".into()),
        _ => Failure::NotConnected,
    })
}

/// 这个容器不用了（最后一个用它的标签关了、应用要退出了）：收掉通道。
/// 通道的 Drop 要等正在跑的请求结束（互斥），放到后台线程做，不占着界面
pub fn shutdown(container: &str) {
    let channel = channels().remove(container);
    if let Some(channel) = channel {
        std::thread::spawn(move || drop(channel));
    }
}

/// 容器在不在跑、什么时候起的（`docker inspect`）。不存在是 `NotConnected`；daemon 没起是 `Failed`
fn inspect(container: &str) -> Result<(bool, Option<u64>), Failure> {
    let mut command = Command::new("docker");
    command.args([
        "inspect",
        "-f",
        "{{.State.Running}} {{.State.StartedAt}}",
        "--",
        container,
    ]);
    let ran = run(command, None, Some(Duration::from_secs(5)))?;
    if !ran.success {
        let reason = first_reason(&ran.stderr);
        return Err(if reason.contains("No such") {
            Failure::NotConnected
        } else {
            Failure::Failed(reason)
        });
    }
    let text = String::from_utf8_lossy(&ran.stdout);
    let mut fields = text.split_whitespace();
    let running = fields.next() == Some("true");
    let started = fields.next().and_then(rfc3339_seconds);
    Ok((running, started))
}

/// 取一次监控读数。停着的容器 `docker stats` 也返回成功（一排 0）：先看它还在不在跑，不在就是没连上。
/// CPU 是 `docker stats` 算好的百分比（多核可以超过 100%）；负载没有按容器算的，不显示；
/// 磁盘走通道问一次 `df`，问不到就不显示这一格（不显示错的数字）
pub fn poll(container: &str) -> Result<Reading, Failure> {
    if !is_valid_container(container) {
        return Err(Failure::Failed("容器名不能用".into()));
    }
    let (running, started) = inspect(container)?;
    if !running {
        return Err(Failure::NotConnected);
    }
    let mut command = Command::new("docker");
    command.args([
        "stats",
        "--no-stream",
        "--format",
        "{{.CPUPerc}}\t{{.MemUsage}}",
        "--",
        container,
    ]);
    let ran = run(command, None, Some(Duration::from_secs(10)))?;
    if !ran.success {
        return Err(Failure::Failed(first_reason(&ran.stderr)));
    }
    let mut reading = parse_stats(&String::from_utf8_lossy(&ran.stdout))?;
    reading.uptime = started.map(|started| now_seconds().saturating_sub(started));
    reading.disk = run_in_container(container, "df -Pk / | tail -1", Duration::from_secs(5))
        .ok()
        .and_then(|ran| parse_df(&String::from_utf8_lossy(&ran.stdout)));
    Ok(reading)
}

/// `docker stats` 的一行：`12.50%\t512MiB / 2GiB`
fn parse_stats(text: &str) -> Result<Reading, Failure> {
    let bad = || Failure::Failed("看不懂 docker stats 的输出".into());
    let line = text.lines().next().ok_or_else(bad)?;
    let (cpu, memory) = line.split_once('\t').ok_or_else(bad)?;
    let cpu: f32 = cpu
        .trim()
        .strip_suffix('%')
        .ok_or_else(bad)?
        .trim()
        .parse()
        .map_err(|_| bad())?;
    let (used, total) = memory.split_once('/').ok_or_else(bad)?;
    let used = parse_size_kib(used).ok_or_else(bad)?;
    let total = parse_size_kib(total).ok_or_else(bad)?;
    Ok(Reading {
        cpu: CpuSample::Percent(cpu),
        mem_total: total,
        mem_available: total.saturating_sub(used),
        disk: None,
        load: None,
        uptime: None,
    })
}

/// `docker stats` 的容量（`100KiB`、`15.66GiB`）换成 KiB。用的是 IEC 单位（1024 进制）
fn parse_size_kib(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|ch: char| ch.is_ascii_alphabetic())?;
    let (number, unit) = text.split_at(split);
    let number: f64 = number.trim().parse().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let scale: f64 = match unit.trim() {
        "B" => 1.0 / 1024.0,
        "KiB" => 1.0,
        "MiB" => 1024.0,
        "GiB" => 1024.0 * 1024.0,
        "TiB" => 1024.0_f64.powi(3),
        "PiB" => 1024.0_f64.powi(4),
        "EiB" => 1024.0_f64.powi(5),
        _ => return None,
    };
    Some((number * scale).round() as u64)
}

/// RFC3339 的 UTC 时间（`2026-11-09T04:24:41.76721328Z`）换成 Unix 秒。只认 `Z` 结尾
/// （`docker inspect` 给的就是 UTC）；看不懂就 `None`，不显示错的运行时间
fn rfc3339_seconds(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let time = time.strip_suffix('Z')?;
    let mut date_parts = date.split('-');
    let (year, month, day) = (
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
    );
    if date_parts.next().is_some() {
        return None;
    }
    let mut time_parts = time.split(':');
    let (hour, minute) = (
        time_parts.next()?.parse::<i64>().ok()?,
        time_parts.next()?.parse::<i64>().ok()?,
    );
    let second_text = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let second: i64 = second_text.split('.').next()?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // 从日期算距 1970-01-01 的天数（Howard Hinnant 的 days_from_civil：三月当岁首，闰年规则才好算）
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146097 + day_of_era - 719468;
    u64::try_from(days * 86400 + hour * 3600 + minute * 60 + second).ok()
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs())
}

/// `df -Pk /` 的最后一行：`overlay 102400000 40960000 … /`。第二列总量、第三列已用（KiB）
fn parse_df(text: &str) -> Option<(u64, u64)> {
    let fields: Vec<&str> = text.lines().last()?.split_whitespace().collect();
    let total = fields.get(1)?.parse().ok()?;
    let used: u64 = fields.get(2)?.parse().ok()?;
    Some((total, used.min(total)))
}

/// 错误输出里第一条非空行，给人看的一句话
fn first_reason(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("命令没有跑成")
        .chars()
        .take(160)
        .collect()
}

/// 列一个目录。`dir` 为 `None` 是容器里的主目录（`docker exec` 的默认用户，通常是 root）
pub fn list(container: &str, dir: Option<&str>) -> Result<Listing, Failure> {
    let ran = run_in_container(container, &files::list_script(dir), Duration::from_secs(15))?;
    if ran.success {
        files::parse_listing(&ran.stdout)
    } else {
        Err(files::classify_list_error(&ran.stderr))
    }
}

/// 把容器里的 `remote`（绝对路径）下载到本地目录 `into` 里，返回存成的路径。
/// 本地已有同名的就加序号，不覆盖；失败时删掉这次留下的半截。`-L` 跟着符号链接走（和文件页显示的类型一致）
pub fn download(
    container: &str,
    remote: &str,
    directory: bool,
    into: &Path,
) -> Result<PathBuf, Failure> {
    let name = remote.rsplit('/').next().unwrap_or_default();
    if !files::is_safe_name(name) {
        return Err(Failure::Failed("这个文件名不能传（带控制字符）".into()));
    }
    if !is_valid_container(container) || !remote.starts_with('/') {
        return Err(Failure::Failed("路径不能用".into()));
    }
    std::fs::create_dir_all(into)
        .map_err(|error| Failure::Failed(format!("建不了下载目录：{error}")))?;
    let destination = files::unique_local(into, name);
    let mut command = Command::new("docker");
    command
        .arg("cp")
        .arg("-L")
        .arg("--")
        .arg(format!("{container}:{remote}"))
        .arg(&destination);
    // 不设超时：大文件要传多久不知道，连接断了 `docker` 自己会退出
    let ran = run(command, None, None)?;
    if ran.success {
        Ok(destination)
    } else {
        let _ = if directory {
            std::fs::remove_dir_all(&destination)
        } else {
            std::fs::remove_file(&destination)
        };
        Err(Failure::Failed(first_reason(&ran.stderr)))
    }
}

/// 把本地的文件或目录上传到容器里的目录 `remote_dir`（绝对路径）里，名字不变
pub fn upload(container: &str, local: &Path, remote_dir: &str) -> Result<(), Failure> {
    let name = local
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !files::is_safe_name(name) {
        return Err(Failure::Failed("这个文件名不能传（带控制字符）".into()));
    }
    if !local.exists() {
        return Err(Failure::Failed("本地没有这个文件".into()));
    }
    if !is_valid_container(container) || !remote_dir.starts_with('/') {
        return Err(Failure::Failed("路径不能用".into()));
    }
    let mut command = Command::new("docker");
    // 结尾的斜杠：放进这个目录里，不是把目录本身替换掉
    command
        .arg("cp")
        .arg("--")
        .arg(local)
        .arg(format!("{container}:{}/", remote_dir.trim_end_matches('/')));
    let ran = run(command, None, None)?;
    if ran.success {
        Ok(())
    } else {
        Err(Failure::Failed(first_reason(&ran.stderr)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_valid() {
        for name in ["web-1", "my_app", "db.local", &"a".repeat(64), "0123abcd"] {
            assert!(is_valid_container(name), "{name}");
        }
        for name in [
            "",
            "-x",
            "a b",
            "a;b",
            "a'b",
            "a\"b",
            "a/b",
            "a\\b",
            "中文",
            &"x".repeat(256),
        ] {
            assert!(!is_valid_container(name), "{name}");
        }
    }

    #[test]
    fn normalize_trims_and_refuses() {
        assert_eq!(normalize("  web-1 \n").as_deref(), Some("web-1"));
        assert_eq!(normalize("a;rm -rf /"), None);
        assert_eq!(normalize("   "), None);
    }

    #[test]
    fn config_execs_into_the_container_with_a_login_shell() {
        let config = config("web-1", 120, 36, None);
        assert_eq!(config.program, "docker");
        assert_eq!(config.arguments[..5], ["exec", "-it", "--", "web-1", "sh"]);
        assert_eq!(config.arguments[5], "-c");
        assert!(config.arguments[6].contains("exec bash -l"));
        assert!(!config.arguments[6].contains("cd --"));
    }

    #[test]
    fn config_enters_the_directory_when_it_is_absolute() {
        let entered = config("web-1", 120, 36, Some("/srv/app"));
        assert!(
            entered.arguments[6].starts_with("cd -- '/srv/app'"),
            "{}",
            entered.arguments[6]
        );
        // 相对路径、带控制字符的不进（换行写进脚本就是另一条命令）
        for directory in ["srv/app", "/srv\n/app"] {
            let rejected = config("web-1", 120, 36, Some(directory));
            assert!(!rejected.arguments[6].contains("cd --"), "{directory}");
        }
    }

    #[test]
    fn container_listing_keeps_order_and_drops_duplicates_and_junk() {
        let names = parse_containers("web-1\ndb\nweb-1\n  api.v2  \nbad;name\n\n");
        assert_eq!(names, ["web-1", "db", "api.v2"]);
    }

    #[test]
    fn stats_parse_into_a_percent_sample_and_kib_memory() {
        let reading = parse_stats("12.50%\t512MiB / 2GiB\n").expect("解析得了");
        assert_eq!(reading.cpu, CpuSample::Percent(12.5));
        assert_eq!(reading.mem_total, 2 * 1024 * 1024);
        assert_eq!(reading.mem_available, (2048 - 512) * 1024);
        assert_eq!(reading.disk, None);
        assert_eq!(reading.load, None);
        assert_eq!(reading.uptime, None);
        // 多核可以超过 100%
        let reading = parse_stats("250.00%\t0B / 0B").expect("解析得了");
        assert_eq!(reading.cpu, CpuSample::Percent(250.0));
        assert_eq!(reading.mem_available, 0);
        for bad in [
            "",
            "12.5%",
            "12.5%\t512MiB",
            "x%\t1KiB / 2KiB",
            "1%\t1XB / 2KiB",
        ] {
            assert!(parse_stats(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sizes_use_iec_units() {
        assert_eq!(parse_size_kib("0B"), Some(0));
        assert_eq!(parse_size_kib("100KiB"), Some(100));
        assert_eq!(parse_size_kib("1.5MiB"), Some(1536));
        assert_eq!(parse_size_kib("2GiB"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size_kib("1TiB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size_kib("100KB"), None, "docker 用的是 IEC，不是 SI");
        assert_eq!(parse_size_kib("-1KiB"), None);
        assert_eq!(parse_size_kib("KiB"), None);
    }

    #[test]
    fn rfc3339_utc_timestamps_become_unix_seconds() {
        assert_eq!(rfc3339_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_seconds("2000-01-01T00:00:00Z"), Some(946_684_800));
        assert_eq!(
            rfc3339_seconds("2026-11-09T04:24:41.76721328Z"),
            Some(1_794_198_281)
        );
        // 只认 UTC：带时区偏移、缺 Z、残缺的都不猜
        for bad in [
            "2026-11-09T04:24:41+08:00",
            "2026-11-09T04:24:41",
            "2026-13-09T04:24:41Z",
            "garbage",
            "2026-11-09",
        ] {
            assert_eq!(rfc3339_seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn df_lines_become_total_and_used() {
        assert_eq!(
            parse_df("overlay 102400000 40960000 61440000 40% /\n"),
            Some((102_400_000, 40_960_000))
        );
        assert_eq!(
            parse_df("x 100 200 0 100% /"),
            Some((100, 100)),
            "已用不超过总量"
        );
        assert_eq!(parse_df(""), None);
        assert_eq!(parse_df("df: command not found"), None);
    }
}

/// 真的 Docker 实验室：`NIMBUS_DOCKER_LAB=容器名 cargo test docker::tests::lab`。
/// 容器要在跑、里面有 `sh` 和 GNU find（debian 系就行）。平时这些测试直接返回，不碰 Docker
#[cfg(test)]
mod lab {
    use super::*;

    fn container() -> Option<String> {
        std::env::var("NIMBUS_DOCKER_LAB")
            .ok()
            .filter(|name| is_valid_container(name))
    }

    #[test]
    fn a_real_container_lists_uploads_and_downloads() {
        let Some(container) = container() else { return };
        let listing = list(&container, Some("/")).expect("列得了根目录");
        assert_eq!(listing.path, "/");
        assert!(listing.entries.iter().any(|entry| entry.name == "tmp"));

        // 传一个临时文件上去，再下载回来，内容要一模一样
        let stamp = now_seconds();
        let name = format!("nimbus-lab-{stamp}.txt");
        let local = std::env::temp_dir().join(&name);
        let content = b"hello from nimbus lab\n";
        std::fs::write(&local, content).expect("写得了临时文件");
        upload(&container, &local, "/tmp").expect("传得上");
        let into = std::env::temp_dir().join(format!("nimbus-lab-back-{stamp}"));
        let back = download(&container, &format!("/tmp/{name}"), false, &into).expect("下载得了");
        assert_eq!(std::fs::read(&back).expect("读得了"), content);
        // 容器里清掉，本地也清掉
        let _ = run_in_container(
            &container,
            &format!("rm -f {}", files::quote_sh(&format!("/tmp/{name}"))),
            Duration::from_secs(5),
        );
        let _ = std::fs::remove_file(&local);
        let _ = std::fs::remove_dir_all(&into);
        shutdown(&container);
    }

    #[test]
    fn a_real_container_reports_monitor_readings() {
        let Some(container) = container() else { return };
        let reading = poll(&container).expect("取得到");
        assert!(matches!(reading.cpu, CpuSample::Percent(_)));
        assert!(reading.mem_total > 0);
        assert!(reading.uptime.is_some(), "运行中的容器有启动时间");
        assert_eq!(reading.load, None, "负载没有按容器算的");
        shutdown(&container);
    }
}
