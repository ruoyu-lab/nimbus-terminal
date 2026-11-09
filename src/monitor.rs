//! 服务器监控：在远端跑一小段脚本，读 `/proc` 里的 CPU、内存、磁盘、负载。
//!
//! 数据怎么取（`ssh` 那一头见 [`crate::ssh`]）：监控和终端走**同一条** SSH 连接——终端的 `ssh` 开了连接复用，
//! 这里再起一个 `ssh` 借它的连接跑一条命令，不用再认证一次（密码、两步验证、host key 都不会再问）。
//! 终端断了，连接就没了，监控显示「未连接」。这一层只管命令、解析和算数，不碰进程和界面，所以都能单独测。
//!
//! 只支持 Linux（读 `/proc`）；别的系统（macOS、BSD 服务器）会明确说「暂不支持」，不显示错的数字。

/// 在远端跑的脚本：每一段前面一行 `@@名字`，后面是原样的输出。
/// 只用最朴素的 `sh` 语法，并且整条放进单引号：远端用户的登录 shell 可能是 bash、zsh、fish、csh，
/// ssh 会把这条命令交给它去解释，里面不能有哪个 shell 会另外理解的字符（`!`、反引号、`$(…)`）
pub const REMOTE_COMMAND: &str = "sh -c 'echo @@stat; head -1 /proc/stat; echo @@mem; \
grep -E \"^(MemTotal|MemFree|MemAvailable|Buffers|Cached):\" /proc/meminfo; echo @@disk; \
df -Pk / | tail -1; echo @@load; cat /proc/loadavg; echo @@up; cat /proc/uptime'";

/// 保留多少个 CPU 采样画曲线：每 2 秒一个，两分钟
pub const HISTORY: usize = 60;

/// `/proc/stat` 第一行的累计时间（单位无所谓，只看两次之差的比例）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

/// 一次读到的原始数据
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub cpu: CpuTimes,
    /// 内存、磁盘的单位都是 KiB
    pub mem_total: u64,
    pub mem_available: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub load: [f32; 3],
    pub uptime: u64,
}

/// 这次没取到数据的原因
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// 终端的连接不在（还没连上、断了）
    NotConnected,
    /// 远端不是 Linux（没有 `/proc`）
    Unsupported,
    /// 别的：命令没跑起来、超时、输出看不懂。带一句能给人看的原因
    Failed(String),
}

/// 把脚本的输出解析成读数。不是 Linux（缺 `/proc/stat`）是 [`Failure::Unsupported`]，输出残缺是 [`Failure::Failed`]
pub fn parse(output: &str) -> Result<Reading, Failure> {
    let section = |name: &str| -> Vec<&str> {
        let marker = format!("@@{name}");
        let mut lines = output.lines().skip_while(|line| line.trim() != marker);
        lines.next();
        lines
            .take_while(|line| !line.starts_with("@@"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect()
    };
    let bad = |what: &str| Failure::Failed(format!("看不懂服务器返回的{what}"));
    let number = |text: &str| text.parse::<u64>().ok();

    let stat = section("stat");
    let Some(fields) = stat.first().and_then(|line| line.strip_prefix("cpu ")) else {
        // 没有 /proc/stat：不是 Linux，或者 /proc 没挂载
        return Err(Failure::Unsupported);
    };
    let ticks: Vec<u64> = fields.split_whitespace().filter_map(number).collect();
    if ticks.len() < 4 {
        return Err(bad("CPU 数据"));
    }
    // user nice system idle iowait irq softirq steal：guest 已经算在 user 里，不重复加
    let total: u64 = ticks.iter().take(8).sum();
    let idle = ticks[3] + ticks.get(4).copied().unwrap_or(0);

    let mut memory = std::collections::HashMap::new();
    for line in section("mem") {
        if let Some((name, rest)) = line.split_once(':')
            && let Some(kib) = rest.split_whitespace().next().and_then(number)
        {
            memory.insert(name.to_string(), kib);
        }
    }
    let mem_total = *memory.get("MemTotal").ok_or_else(|| bad("内存数据"))?;
    // 老内核没有 MemAvailable：用空闲加缓存估
    let mem_available = memory.get("MemAvailable").copied().unwrap_or_else(|| {
        ["MemFree", "Buffers", "Cached"].iter().filter_map(|name| memory.get(*name)).sum()
    });

    let disk = section("disk");
    let disk_fields: Vec<&str> = disk.last().map(|line| line.split_whitespace().collect()).unwrap_or_default();
    let (disk_total, disk_used) = match (disk_fields.get(1).and_then(|text| number(text)), disk_fields.get(2).and_then(|text| number(text))) {
        (Some(total), Some(used)) => (total, used),
        _ => return Err(bad("磁盘数据")),
    };

    let load_line = section("load");
    let load: Vec<f32> = load_line
        .first()
        .map(|line| line.split_whitespace().take(3).filter_map(|text| text.parse().ok()).collect())
        .unwrap_or_default();
    let [one, five, fifteen] = load[..] else {
        return Err(bad("负载数据"));
    };
    let uptime = section("up")
        .first()
        .and_then(|line| line.split_whitespace().next())
        .and_then(|text| text.parse::<f64>().ok())
        .map_or(0, |seconds| seconds.max(0.0) as u64);

    Ok(Reading {
        cpu: CpuTimes { busy: total.saturating_sub(idle), total },
        mem_total,
        mem_available: mem_available.min(mem_total),
        disk_total,
        disk_used: disk_used.min(disk_total),
        load: [one, five, fifteen],
        uptime,
    })
}

/// 两次读数之间 CPU 忙了百分之几（0..=100）。要两次才算得出；间隔里时间没走（或者远端重启计数回零）算不出
pub fn cpu_percent(before: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(before.total).filter(|total| *total > 0)?;
    let busy = now.busy.saturating_sub(before.busy).min(total);
    Some(busy as f32 / total as f32 * 100.0)
}

/// 界面要显示的一份数据
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// 第一次读还没有上一次可比，所以是 `None`
    pub cpu: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub load: [f32; 3],
    pub uptime: u64,
}

/// 监控现在的状态
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// 还没有取过
    #[default]
    Waiting,
    Live,
    NotConnected,
    Unsupported,
    Failed(String),
}

/// 一个标签的监控：最近的读数、CPU 的历史曲线、状态
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Monitor {
    previous: Option<CpuTimes>,
    pub cpu_history: Vec<f32>,
    pub latest: Option<Snapshot>,
    pub status: Status,
}

impl Monitor {
    /// 收下一次取数的结果。失败时保留最后一次的数字（界面可以淡着显示），但状态变了，CPU 的基准也作废
    /// （断线再连上之后第一次不能拿旧基准去算）
    pub fn apply(&mut self, result: Result<Reading, Failure>) {
        match result {
            Ok(reading) => {
                let cpu = self.previous.and_then(|before| cpu_percent(before, reading.cpu));
                self.previous = Some(reading.cpu);
                if let Some(percent) = cpu {
                    self.cpu_history.push(percent);
                    if self.cpu_history.len() > HISTORY {
                        self.cpu_history.remove(0);
                    }
                }
                self.latest = Some(Snapshot {
                    cpu,
                    mem_used: reading.mem_total - reading.mem_available,
                    mem_total: reading.mem_total,
                    disk_used: reading.disk_used,
                    disk_total: reading.disk_total,
                    load: reading.load,
                    uptime: reading.uptime,
                });
                self.status = Status::Live;
            }
            Err(failure) => {
                self.previous = None;
                self.status = match failure {
                    Failure::NotConnected => Status::NotConnected,
                    Failure::Unsupported => Status::Unsupported,
                    Failure::Failed(reason) => Status::Failed(reason),
                };
            }
        }
    }
}

/// 容量：KiB 换成人看的（`3.2 GB`、`512 MB`）
pub fn size(kib: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    let (mut value, mut unit) = (kib as f64, 0);
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 || unit == 0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// 运行时间：`3 天 4 小时`、`5 小时 12 分`、`7 分`
pub fn uptime(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes} 分"),
        (0, _) => format!("{hours} 小时 {minutes} 分"),
        _ => format!("{days} 天 {hours} 小时"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一台真实 Linux 的输出（精简过）
    const SAMPLE: &str = "\
@@stat
cpu  4705 150 1120 16250 520 0 30 0 0 0
@@mem
MemTotal:       16384000 kB
MemFree:         2048000 kB
MemAvailable:    8192000 kB
Buffers:          512000 kB
Cached:          4096000 kB
@@disk
/dev/vda1       102400000 40960000  56320000      43% /
@@load
0.52 0.61 0.70 2/345 12345
@@up
273600.45 530000.10
";

    #[test]
    fn a_linux_sample_is_parsed() {
        let reading = parse(SAMPLE).expect("能解析");
        assert_eq!(reading.cpu, CpuTimes { busy: 4705 + 150 + 1120 + 30, total: 4705 + 150 + 1120 + 16250 + 520 + 30 });
        assert_eq!((reading.mem_total, reading.mem_available), (16_384_000, 8_192_000));
        assert_eq!((reading.disk_total, reading.disk_used), (102_400_000, 40_960_000));
        assert_eq!(reading.load, [0.52, 0.61, 0.70]);
        assert_eq!(reading.uptime, 273_600);
    }

    /// 没有 MemAvailable 的老内核：用空闲加缓冲加缓存
    #[test]
    fn old_kernels_without_memavailable_are_estimated() {
        let text = SAMPLE.replace("MemAvailable:    8192000 kB\n", "");
        assert_eq!(parse(&text).expect("能解析").mem_available, 2_048_000 + 512_000 + 4_096_000);
    }

    /// 不是 Linux（macOS 服务器、BSD）：`/proc/stat` 不存在，只说「暂不支持」，不拿错的数字糊弄
    #[test]
    fn a_system_without_proc_is_unsupported() {
        let text = "@@stat\nhead: /proc/stat: No such file or directory\n@@mem\n@@disk\n/dev/disk1s1 1 1 1 1% /\n@@load\n@@up\n";
        assert_eq!(parse(text), Err(Failure::Unsupported));
        assert_eq!(parse(""), Err(Failure::Unsupported));
    }

    #[test]
    fn truncated_or_garbled_output_is_a_failure_not_a_crash() {
        for text in [
            "@@stat\ncpu  1 2\n@@mem\n@@disk\n@@load\n@@up\n",
            &SAMPLE.replace("0.52 0.61 0.70", "x y z"),
            &SAMPLE.replace("/dev/vda1       102400000 40960000  56320000      43% /", "df: error"),
            &SAMPLE.replace("MemTotal:       16384000 kB\n", ""),
        ] {
            assert!(matches!(parse(text), Err(Failure::Failed(_))), "{text}");
        }
    }

    /// 内存、磁盘的「已用」不能超过总量（读数跳动时），不然进度条会画出界
    #[test]
    fn used_never_exceeds_total() {
        let text = SAMPLE.replace("MemAvailable:    8192000 kB", "MemAvailable:   99999999 kB").replace("40960000", "999999999");
        let reading = parse(&text).expect("能解析");
        assert_eq!(reading.mem_available, reading.mem_total);
        assert_eq!(reading.disk_used, reading.disk_total);
    }

    #[test]
    fn cpu_percent_is_the_busy_share_of_the_interval() {
        let (before, now) = (CpuTimes { busy: 100, total: 1000 }, CpuTimes { busy: 150, total: 1100 });
        assert_eq!(cpu_percent(before, now), Some(50.0));
        assert_eq!(cpu_percent(now, now), None, "时间没走算不出");
        assert_eq!(cpu_percent(now, before), None, "计数回零（远端重启）算不出");
        // 忙的增量比总增量大（读数抖动）也不会超过 100%
        assert_eq!(cpu_percent(before, CpuTimes { busy: 900, total: 1100 }), Some(100.0));
    }

    /// 第一次读没有基准，没有 CPU；第二次有；曲线最多保留 `HISTORY` 个
    #[test]
    fn the_monitor_needs_two_readings_for_cpu_and_caps_the_history() {
        let mut monitor = Monitor::default();
        assert_eq!(monitor.status, Status::Waiting);
        let reading = |busy, total| Reading { cpu: CpuTimes { busy, total }, ..parse(SAMPLE).expect("能解析") };
        monitor.apply(Ok(reading(0, 100)));
        assert_eq!(monitor.status, Status::Live);
        assert_eq!(monitor.latest.as_ref().and_then(|snapshot| snapshot.cpu), None);
        assert_eq!(monitor.latest.as_ref().map(|snapshot| snapshot.mem_used), Some(16_384_000 - 8_192_000));
        monitor.apply(Ok(reading(50, 200)));
        assert_eq!(monitor.latest.as_ref().and_then(|snapshot| snapshot.cpu), Some(50.0));
        for step in 0..(HISTORY as u64 + 10) {
            monitor.apply(Ok(reading(100 + step, 300 + step * 2)));
        }
        assert_eq!(monitor.cpu_history.len(), HISTORY);
    }

    /// 断线再连上：第一次不能拿断线前的基准去算（中间隔了不知道多久），断线时保留最后的数字
    #[test]
    fn a_failure_resets_the_cpu_baseline_but_keeps_the_last_numbers() {
        let mut monitor = Monitor::default();
        let reading = |busy, total| Reading { cpu: CpuTimes { busy, total }, ..parse(SAMPLE).expect("能解析") };
        monitor.apply(Ok(reading(0, 100)));
        monitor.apply(Ok(reading(50, 200)));
        monitor.apply(Err(Failure::NotConnected));
        assert_eq!(monitor.status, Status::NotConnected);
        assert!(monitor.latest.is_some(), "最后的数字还在");
        monitor.apply(Ok(reading(5000, 9000)));
        assert_eq!(monitor.latest.as_ref().and_then(|snapshot| snapshot.cpu), None, "断线后的第一次没有基准");
        monitor.apply(Err(Failure::Unsupported));
        assert_eq!(monitor.status, Status::Unsupported);
        monitor.apply(Err(Failure::Failed("超时".into())));
        assert_eq!(monitor.status, Status::Failed("超时".into()));
    }

    #[test]
    fn sizes_and_uptimes_read_naturally() {
        assert_eq!(size(512), "512 KB");
        assert_eq!(size(1024), "1.0 MB");
        assert_eq!(size(16_384_000), "15.6 GB");
        assert_eq!(size(102_400_000), "97.7 GB");
        assert_eq!(size(300 * 1024 * 1024), "300 GB");
        assert_eq!(uptime(59), "0 分");
        assert_eq!(uptime(3 * 3600 + 5 * 60), "3 小时 5 分");
        assert_eq!(uptime(273_600), "3 天 4 小时");
    }

    /// 脚本里不能有哪个 shell 会另外理解的字符：它会被远端用户的登录 shell（可能是 fish、csh）解释
    #[test]
    fn the_remote_command_is_one_quoted_sh_invocation_without_shell_specials() {
        assert!(REMOTE_COMMAND.starts_with("sh -c '") && REMOTE_COMMAND.ends_with('\''));
        let inside = &REMOTE_COMMAND[7..REMOTE_COMMAND.len() - 1];
        assert!(!inside.contains('\''), "单引号里不能再有单引号");
        for special in ['!', '`', '$', '\\', '\n'] {
            assert!(!inside.contains(special), "{special:?}");
        }
    }
}
