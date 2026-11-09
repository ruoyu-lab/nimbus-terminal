//! 起一个会话：会话向外报告的钩子（`Hooks`），怎么起（`Launch`），起出来的东西（`Spawned`）。

use super::*;

/// 会话向外报告的几件事，外面（标签栏）决定怎么办。会话自己不认识窗口和标签
#[derive(Clone)]
pub struct Hooks {
    /// 程序设了新标题（OSC 0 / 2），可能是空的
    pub title: Rc<dyn Fn(&str)>,
    /// 行列数变了
    pub resized: Rc<dyn Fn(usize, usize)>,
    /// shell 退出了，带退出码
    pub exited: Rc<dyn Fn(i32)>,
    /// 读到了新的输出（一批一次）。标签不在前台时，侧栏靠它提示「有新输出」
    pub activity: Rc<dyn Fn()>,
    /// 值得叫人回来看一眼的事（命令跑完了、程序要求弹通知）。弹不弹、弹给谁由外面决定
    pub alert: Rc<dyn Fn(Alert)>,
    /// 要粘贴多行文字、程序又没开括号粘贴时调用：外面问用户，确认了再调 [`Session::paste_text`]。
    /// 没有就直接粘贴
    pub confirm_paste: Option<Rc<dyn Fn(String)>>,
    /// 跑完了一条命令（有命令标记才有），带上敲的命令：命令历史靠它
    pub record: Option<Rc<dyn Fn(String)>>,
}

impl Hooks {
    /// 什么都不管（测试里）
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            title: Rc::new(|_| {}),
            resized: Rc::new(|_, _| {}),
            exited: Rc::new(|_| {}),
            activity: Rc::new(|| {}),
            alert: Rc::new(|_| {}),
            confirm_paste: None,
            record: None,
        }
    }
}

/// 起一个会话要的东西：shell 的 pty 和读队列，或者起不来的原因
pub struct Spawned {
    pub pty: Option<Pty>,
    pub queue: Option<ByteQueue>,
    pub error: Option<String>,
}

/// 怎么起一个会话
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub target: Target,
    pub columns: usize,
    pub rows: usize,
    /// 起始目录；`None` 是主目录
    pub directory: Option<std::path::PathBuf>,
    /// 指定的 shell 程序；`None` 是系统默认的登录 shell
    pub shell: Option<String>,
    /// 终端底色是深色
    pub dark_background: bool,
    /// SSH 会话的连接复用套接字（监控借它）；别的会话、或者目录不可用时是 `None`
    pub control: Option<std::path::PathBuf>,
    /// 远端会话（SSH、容器）一开始进哪个目录（绝对路径）；`None` 是主目录
    pub remote_directory: Option<String>,
    /// shell 集成脚本所在的目录；`None` 是不用（设置里关了、写不进配置目录）
    pub integration: Option<std::path::PathBuf>,
}

/// 起一个会话：本机是登录 shell，服务器是 `ssh`，容器是 `docker exec`
pub fn spawn(launch: &Launch) -> Spawned {
    let (columns, rows) = (launch.columns as u16, launch.rows as u16);
    let mut config = match &launch.target {
        Target::Local => match &launch.shell {
            Some(program) => pty::Config::shell(program, columns, rows),
            None => pty::Config::login_shell(columns, rows),
        },
        Target::Ssh { host } => {
            // 界面那边已经验过一次；这里再验一次，主机名绝不能被 `ssh` 当成选项
            if !crate::ssh::is_valid_host(host) {
                return Spawned {
                    pty: None,
                    queue: None,
                    error: Some(format!("主机名不能用：{host}")),
                };
            }
            // 套接字目录不可用（权限不对）就不开复用：终端照常连，只是没有监控
            let control = launch
                .control
                .as_deref()
                .filter(|path| crate::ssh::control_ready(path));
            crate::ssh::config(
                host,
                columns,
                rows,
                control,
                launch.remote_directory.as_deref(),
            )
        }
        Target::Docker { container } => {
            // 同主机名：再验一次，容器名绝不能被 `docker` 当成选项
            if !crate::docker::is_valid_container(container) {
                return Spawned {
                    pty: None,
                    queue: None,
                    error: Some(format!("容器名不能用：{container}")),
                };
            }
            crate::docker::config(container, columns, rows, launch.remote_directory.as_deref())
        }
    };
    // 起始目录只对本机有意思：服务器上没有这个目录
    if let (Target::Local, Some(directory)) = (&launch.target, &launch.directory) {
        config.working_directory = Some(directory.clone());
    }
    config.dark_background = launch.dark_background;
    // shell 集成：zsh 加环境变量，Windows 上的 PowerShell 加参数；认的是要起的程序，设置里指定的别的 shell 不动
    if let (Target::Local, Some(directory)) = (&launch.target, &launch.integration) {
        crate::shell::integrate(directory, &mut config);
    }
    match pty::spawn(config).map_err(|error| error.to_string()) {
        Ok((pty, queue)) => Spawned {
            pty: Some(pty),
            queue: Some(queue),
            error: None,
        },
        Err(error) => Spawned {
            pty: None,
            queue: None,
            error: Some(error),
        },
    }
}
