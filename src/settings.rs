//! 设置：有哪些项、默认值、落盘。界面（`chrome::settings`）读写 [`Form`]，会话和窗口读它来干活。
//!
//! [`Settings`] 是一份普通数据（读写文件用），[`Form`] 是同样的项各包成一个信号（界面和会话绑它）。
//! 两边的字段由同一个宏生成，加一项只用改一处。

use aureole::prelude::*;
use serde::{Deserialize, Serialize};

use crate::store::Store;

/// 文件名
const FILE: &str = "settings.json";

/// 启动时
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Startup {
    /// 不开终端，从空白开始
    Empty,
    /// 把上次关掉时开着的终端按原来的目录再开出来
    Restore,
}

/// 新终端用哪个 shell
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    /// 系统的默认 shell（`$SHELL`），按登录 shell 起
    Login,
    /// 指定一个程序
    Program,
}

/// 没有文件夹目录可用时，新终端从哪个目录开始
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryKind {
    Home,
    Custom,
}

/// 关掉最后一个终端之后
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastTab {
    /// 窗口留着，显示「没有标签」
    Keep,
    Quit,
}

/// 启动时侧栏展开还是收起
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarStart {
    Open,
    Closed,
}

/// 标签上写什么
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    /// 一直是「终端」（自己改过名的除外）
    Fixed,
    /// 跟着程序报告的标题走（vim、ssh 会改标题）
    Reported,
}

/// 界面（连同终端）的外观
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    Light,
    Dark,
    /// 跟着系统的外观走
    System,
}

/// 终端字体
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontChoice {
    System,
    Menlo,
    Monaco,
    CourierNew,
}

impl FontChoice {
    /// 字体族的名字。必须是系统里真有的字体：名字找不到会退到界面字体（不是等宽的），
    /// 终端里的字就一个个隔得很开。「系统等宽」按平台挑一个一定装着的等宽字体
    pub fn family(self) -> &'static str {
        match self {
            Self::System => {
                if cfg!(target_os = "macos") {
                    "Menlo"
                } else if cfg!(windows) {
                    "Consolas"
                } else {
                    "DejaVu Sans Mono"
                }
            }
            Self::Menlo => "Menlo",
            Self::Monaco => "Monaco",
            Self::CourierNew => "Courier New",
        }
    }
}

/// 终端字的粗细：常规和粗两档
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    /// 以前还有细、中等两档，存过的设置里写的是那些名字，归到常规
    #[serde(alias = "light", alias = "medium")]
    Regular,
    /// 以前的「半粗」归到粗
    #[serde(alias = "semibold")]
    Bold,
}

impl FontWeight {
    pub fn value(self) -> f32 {
        match self {
            Self::Regular => 400.0,
            Self::Bold => 700.0,
        }
    }
}

/// 默认的光标形状（程序用转义序列指定了形状时听程序的）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    Block,
    Underline,
    Bar,
}

/// 响铃（程序输出 BEL）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bell {
    Off,
    Sound,
    Flash,
}

/// 字号的范围
pub const FONT_SIZE: (f64, f64) = (9.0, 32.0);
/// 行距倍数的范围
pub const LINE_HEIGHT: (f64, f64) = (1.0, 2.0);
/// 历史行数的范围
pub const SCROLLBACK: (f64, f64) = (100.0, 200_000.0);
/// 滚轮一格滚几行的范围
pub const WHEEL: (f64, f64) = (1.0, 20.0);

macro_rules! settings {
    ($($(#[$doc:meta])* $name:ident : $ty:ty = $default:expr),* $(,)?) => {
        /// 全部设置，写进 `settings.json`。缺的项用默认值，多出来的项忽略
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Settings {
            $($(#[$doc])* pub $name: $ty,)*
        }

        impl Default for Settings {
            fn default() -> Self {
                Self { $($name: $default,)* }
            }
        }

        /// 每一项一个信号：界面读写、会话绑定
        #[derive(Clone, Copy)]
        pub struct Form {
            $(pub $name: Signal<$ty>,)*
        }

        impl Form {
            pub fn new(settings: &Settings) -> Self {
                Self { $($name: signal(settings.$name.clone()),)* }
            }

            /// 现在的全部设置。在副作用里调用会跟着每一项
            pub fn snapshot(&self) -> Settings {
                Settings { $($name: self.$name.get(),)* }
            }

            /// 整份换掉（恢复默认）
            pub fn apply(&self, settings: &Settings) {
                $(self.$name.set(settings.$name.clone());)*
            }
        }
    };
}

settings! {
    // 通用
    /// 启动时
    startup: Startup = Startup::Restore,
    shell: ShellKind = ShellKind::Login,
    shell_program: String = String::new(),
    directory: DirectoryKind = DirectoryKind::Home,
    directory_path: String = String::new(),
    title: TitleSource = TitleSource::Fixed,
    /// 关掉应用时把每个标签里的内容存下来，恢复标签时一起带回来
    save_output: bool = true,
    confirm_close: bool = true,
    last_tab: LastTab = LastTab::Keep,
    sidebar: SidebarStart = SidebarStart::Open,
    // 外观
    appearance: ThemeChoice = ThemeChoice::System,
    font: FontChoice = FontChoice::System,
    font_weight: FontWeight = FontWeight::Regular,
    font_size: f64 = 13.0,
    line_height: f64 = 1.2,
    cursor: CursorStyle = CursorStyle::Block,
    cursor_blink: bool = true,
    status_bar: bool = true,
    /// 连到服务器的会话显示右侧的监控和文件面板；关掉就一直不显示
    remote: bool = true,
    // 终端
    scrollback: f64 = 10_000.0,
    wheel: f64 = 3.0,
    copy_on_select: bool = false,
    right_click_paste: bool = false,
    /// 程序没开括号粘贴时，粘贴多行文字先问一下（每一行都会当成命令立刻执行）
    confirm_paste: bool = true,
    bell: Bell = Bell::Off,
    /// 本机的 zsh 自动加载 shell 集成（OSC 133 命令标记）：跳转命令、复制输出、命令历史
    shell_integration: bool = true,
    /// 窗口在后台时，命令跑完、程序要求弹的通知用系统通知提醒
    notifications: bool = true,
}

impl Settings {
    /// 把越界的数字拉回范围（手改了文件、旧版本留下的值）
    fn clamped(mut self) -> Self {
        let clamp = |value: f64, (low, high): (f64, f64), fallback: f64| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        let default = Self::default();
        self.font_size = clamp(self.font_size, FONT_SIZE, default.font_size);
        self.line_height = clamp(self.line_height, LINE_HEIGHT, default.line_height);
        self.scrollback = clamp(self.scrollback, SCROLLBACK, default.scrollback).round();
        self.wheel = clamp(self.wheel, WHEEL, default.wheel).round();
        self
    }

    pub fn load(store: &Store) -> Self {
        store.load::<Settings>(FILE).clamped()
    }
}

impl Form {
    /// 设置一变就写盘
    pub fn persist(self, store: Store) {
        if !store.is_persistent() {
            return;
        }
        Effect::new(move || {
            store.save(FILE, &self.snapshot());
        });
    }

    /// 历史行数
    pub fn scrollback_lines(&self) -> usize {
        self.scrollback.get_untracked().max(0.0) as usize
    }

    /// 滚轮一格滚几行
    pub fn wheel_lines(&self) -> f32 {
        self.wheel.get_untracked() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_gives_the_defaults_and_partial_files_fill_in() {
        let directory = crate::store::tests::scratch("settings");
        let store = Store::at(&directory);
        assert_eq!(Settings::load(&store), Settings::default());
        std::fs::create_dir_all(&directory).expect("建得了目录");
        std::fs::write(
            directory.join(FILE),
            r#"{"appearance":"dark","font_size":999,"scrollback":"坏"}"#,
        )
        .expect("写得进");
        // 有一项读不懂：整份退回默认值，而不是留一半
        assert_eq!(Settings::load(&store), Settings::default());
        std::fs::write(
            directory.join(FILE),
            r#"{"appearance":"dark","font_size":999}"#,
        )
        .expect("写得进");
        let loaded = Settings::load(&store);
        assert_eq!(loaded.appearance, ThemeChoice::Dark);
        assert_eq!(loaded.font_size, FONT_SIZE.1, "越界的拉回范围");
        assert_eq!(loaded.cursor, Settings::default().cursor, "缺的用默认");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_font_weight_survives_a_round_trip() {
        let settings = Settings {
            font_weight: FontWeight::Bold,
            ..Settings::default()
        };
        let text = serde_json::to_string(&settings).expect("写得出");
        let back: Settings = serde_json::from_str(&text).expect("读得回");
        assert_eq!(back, settings);
    }

    /// 以前的设置文件里写过细、中等、半粗：读得进来（归到常规、粗），不会因为一个选项读不懂就整份退回默认
    #[test]
    fn weights_from_older_versions_still_load() {
        for (old, expected) in [
            ("light", FontWeight::Regular),
            ("medium", FontWeight::Regular),
            ("regular", FontWeight::Regular),
            ("semibold", FontWeight::Bold),
            ("bold", FontWeight::Bold),
        ] {
            let text = format!(r#"{{"font_weight":"{old}","font_size":15}}"#);
            let loaded: Settings = serde_json::from_str(&text).expect("读得进来");
            assert_eq!(loaded.font_weight, expected, "{old}");
            assert_eq!(loaded.font_size, 15.0, "别的项也在");
        }
    }

    #[test]
    fn the_form_round_trips_through_a_snapshot() {
        let settings = Settings {
            appearance: ThemeChoice::Dark,
            shell_program: "/bin/bash".into(),
            wheel: 7.0,
            ..Settings::default()
        };
        let form = Form::new(&settings);
        assert_eq!(form.snapshot(), settings);
        form.apply(&Settings::default());
        assert_eq!(form.snapshot(), Settings::default());
    }
}
