//! 显示器，以及窗口的位置和大小按显示器分别记住。
//!
//! 拔掉外接显示器、换了分辨率或缩放之后，上次的位置可能落在屏幕外面（窗口找不回来）。
//! 所以每块显示器各记一份，而且「同一块显示器」的认定包括它的缩放比例和逻辑分辨率：
//! 缩放一变就当成另一份记录，不拿旧位置硬套。启动时按上次用的那块显示器找记录，
//! 那块不在了就用主显示器；找不到记录就用默认大小居中；记录对得上也要夹回屏幕可见区域里。
//!
//! 坐标都是逻辑像素，原点在主显示器左上角（和框架报的窗口位置同一套）。
//! 位置按相对于所在显示器左上角的偏移存，显示器在桌面上的排列变了也还对。

use serde::{Deserialize, Serialize};

use crate::store::Store;

/// 文件名
const FILE: &str = "window.json";
/// 最多留多少条记录：显示器和缩放的组合不会太多，超了丢最旧的
const MAX_RECORDS: usize = 24;

/// 一块矩形区域（逻辑像素）
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right(&self) -> f32 {
        self.x + self.width
    }

    fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// 两块区域重叠的面积
    fn overlap(&self, other: &Area) -> f32 {
        let width = self.right().min(other.right()) - self.x.max(other.x);
        let height = self.bottom().min(other.bottom()) - self.y.max(other.y);
        if width > 0.0 && height > 0.0 {
            width * height
        } else {
            0.0
        }
    }
}

/// 现在接着的一块显示器
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// 稳定的身份（macOS 上是显示器的 UUID），重启、重新插拔之后不变
    pub id: String,
    /// 缩放比例：一个逻辑像素是几个物理像素
    pub scale: f32,
    /// 整块屏幕
    pub frame: Area,
    /// 去掉菜单栏、程序坞之后能放窗口的部分
    pub visible: Area,
    pub primary: bool,
}

impl Display {
    pub fn key(&self) -> DisplayKey {
        DisplayKey {
            id: self.id.clone(),
            scale: (self.scale * 100.0).round() as u32,
            width: self.frame.width.round() as u32,
            height: self.frame.height.round() as u32,
        }
    }
}

/// 记录按什么认：哪块显示器、什么缩放、什么逻辑分辨率。三样都一样才算同一份
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayKey {
    pub id: String,
    /// 缩放比例乘 100（2.0 存 200）
    pub scale: u32,
    pub width: u32,
    pub height: u32,
}

/// 在某个显示器配置下，窗口上次摆在哪儿
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub key: DisplayKey,
    /// 窗口左上角离这块显示器左上角的偏移
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

/// 窗口位置的全部记忆
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    /// 上次窗口在哪块显示器上
    pub last: Option<DisplayKey>,
    /// 越靠后越新
    pub records: Vec<Record>,
}

/// 启动时窗口该怎么开
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub width: f32,
    pub height: f32,
    /// 窗口左上角；`None` 是让系统决定（拿不到显示器信息的平台）
    pub position: Option<(f32, f32)>,
    pub maximized: bool,
}

/// 窗口再小也不比这个小（和窗口的最小尺寸一致）
pub const MIN_SIZE: (f32, f32) = (720.0, 460.0);

impl Memory {
    pub fn load(store: &Store) -> Self {
        store.load(FILE)
    }

    pub fn save(&self, store: &Store) {
        store.save(FILE, self);
    }

    /// 启动时怎么开：`displays` 是现在接着的显示器，`default` 是没有记录时的大小
    pub fn plan(&self, displays: &[Display], default: (f32, f32)) -> Plan {
        let Some(target) = self.target(displays) else {
            // 没有显示器信息：只能沿用上次的大小，位置交给系统
            let (width, height) = self
                .records
                .last()
                .map_or(default, |record| (record.width, record.height));
            return Plan {
                width: width.max(MIN_SIZE.0),
                height: height.max(MIN_SIZE.1),
                position: None,
                maximized: false,
            };
        };
        let key = target.key();
        let (area, maximized) = if let Some(record) = self.records.iter().find(|r| r.key == key) {
            (
                Area::new(
                    target.frame.x + record.x,
                    target.frame.y + record.y,
                    record.width,
                    record.height,
                ),
                record.maximized,
            )
        } else if let Some(record) = self.records.iter().rev().find(|r| r.key.id == key.id) {
            // 同一块显示器但缩放或分辨率变了：位置不可信，大小还是用上次的，居中
            (
                centered(&target.visible, record.width, record.height),
                false,
            )
        } else {
            (centered(&target.visible, default.0, default.1), false)
        };
        let area = fit(area, &target.visible);
        Plan {
            width: area.width,
            height: area.height,
            position: Some((area.x, area.y)),
            maximized,
        }
    }

    /// 上次窗口在的那块显示器；它不在了就是主显示器
    fn target<'a>(&self, displays: &'a [Display]) -> Option<&'a Display> {
        self.last
            .as_ref()
            .and_then(|key| displays.iter().find(|display| display.id == key.id))
            .or_else(|| displays.iter().find(|display| display.primary))
            .or_else(|| displays.first())
    }

    /// 记下窗口现在的位置。最大化时只记「最大化了」，不拿最大化的大小覆盖平时的大小。
    /// 返回记忆有没有变
    pub fn remember(&mut self, displays: &[Display], bounds: Area, maximized: bool) -> bool {
        let Some(display) = display_of(displays, &bounds) else {
            return false;
        };
        let key = display.key();
        let previous = self.records.iter().position(|record| record.key == key);
        let mut record = match previous {
            Some(index) => self.records[index].clone(),
            None => {
                let area = if maximized {
                    centered(&display.visible, 1040.0, 680.0)
                } else {
                    bounds
                };
                Record {
                    key: key.clone(),
                    x: area.x - display.frame.x,
                    y: area.y - display.frame.y,
                    width: area.width,
                    height: area.height,
                    maximized,
                }
            }
        };
        record.maximized = maximized;
        if !maximized {
            record.x = bounds.x - display.frame.x;
            record.y = bounds.y - display.frame.y;
            record.width = bounds.width;
            record.height = bounds.height;
        }
        let mut next = self.clone();
        if let Some(index) = previous {
            next.records.remove(index);
        }
        next.records.push(record);
        if next.records.len() > MAX_RECORDS {
            next.records.remove(0);
        }
        next.last = Some(key);
        let changed = next != *self;
        *self = next;
        changed
    }
}

/// 窗口主要在哪块显示器上：重叠面积最大的；谁也不挨着就是离得最近的
pub fn display_of<'a>(displays: &'a [Display], bounds: &Area) -> Option<&'a Display> {
    let best = displays
        .iter()
        .map(|display| (display, display.frame.overlap(bounds)))
        .filter(|(_, overlap)| *overlap > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(display, _)| display);
    best.or_else(|| {
        let (cx, cy) = (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        displays
            .iter()
            .min_by(|a, b| distance(&a.frame, cx, cy).total_cmp(&distance(&b.frame, cx, cy)))
    })
}

fn distance(area: &Area, x: f32, y: f32) -> f32 {
    let dx = (area.x - x).max(0.0).max(x - area.right());
    let dy = (area.y - y).max(0.0).max(y - area.bottom());
    dx * dx + dy * dy
}

/// 在 `within` 里居中放一个这么大的窗口
fn centered(within: &Area, width: f32, height: f32) -> Area {
    Area::new(
        within.x + (within.width - width) / 2.0,
        within.y + (within.height - height) / 2.0,
        width,
        height,
    )
}

/// 把窗口夹进可见区域：大小不超过它（能放下时不小于最小尺寸），整个窗口都在里面
fn fit(area: Area, visible: &Area) -> Area {
    let width = area
        .width
        .min(visible.width)
        .max(MIN_SIZE.0.min(visible.width));
    let height = area
        .height
        .min(visible.height)
        .max(MIN_SIZE.1.min(visible.height));
    let x = area
        .x
        .clamp(visible.x, (visible.right() - width).max(visible.x));
    let y = area
        .y
        .clamp(visible.y, (visible.bottom() - height).max(visible.y));
    Area::new(x, y, width, height)
}

/// 现在接着的显示器
#[cfg(target_os = "macos")]
pub fn displays() -> Vec<Display> {
    macos::displays()
}

/// Windows：枚举现在接着的显示器，坐标和框架报的窗口位置同一套（逻辑像素，原点在主显示器左上角）
#[cfg(windows)]
pub fn displays() -> Vec<Display> {
    windows::displays()
}

/// 别的平台还没接显示器信息：窗口位置交给系统，只记大小
#[cfg(not(any(target_os = "macos", windows)))]
pub fn displays() -> Vec<Display> {
    Vec::new()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, NSString};

    use super::{Area, Display};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UuidBytes([u8; 16]);

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFUUIDGetUUIDBytes(uuid: *const c_void) -> UuidBytes;
        fn CFRelease(object: *const c_void);
    }

    /// 显示器的 UUID（十六进制）；系统给不出来时用显示器编号凑合
    fn identity(number: u32) -> String {
        // SAFETY: `CGDisplayCreateUUIDFromDisplayID` 对任何编号都能调用，给不出来返回空指针；
        // 返回的 CFUUID 归我们（Create 规则），取完字节就释放，只释放这一次
        unsafe {
            let uuid = CGDisplayCreateUUIDFromDisplayID(number);
            if uuid.is_null() {
                return format!("display-{number}");
            }
            let bytes = CFUUIDGetUUIDBytes(uuid);
            CFRelease(uuid);
            bytes.0.iter().map(|byte| format!("{byte:02x}")).collect()
        }
    }

    fn screen_number(screen: &NSScreen) -> Option<u32> {
        let description = screen.deviceDescription();
        let key = NSString::from_str("NSScreenNumber");
        let value: Retained<AnyObject> = description.objectForKey(&key)?;
        value
            .downcast_ref::<NSNumber>()
            .map(|number| number.unsignedIntValue())
    }

    pub(super) fn displays() -> Vec<Display> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        let screens = NSScreen::screens(mtm);
        // 第一块是主显示器（有菜单栏的），框架报的坐标以它的左上角为原点、y 向下
        let Some(primary_height) = screens
            .iter()
            .next()
            .map(|screen| screen.frame().size.height)
        else {
            return Vec::new();
        };
        let flip = |frame: objc2_foundation::NSRect| {
            Area::new(
                frame.origin.x as f32,
                (primary_height - frame.origin.y - frame.size.height) as f32,
                frame.size.width as f32,
                frame.size.height as f32,
            )
        };
        screens
            .iter()
            .enumerate()
            .map(|(index, screen)| Display {
                id: screen_number(&screen).map_or_else(|| format!("screen-{index}"), identity),
                scale: screen.backingScaleFactor() as f32,
                frame: flip(screen.frame()),
                visible: flip(screen.visibleFrame()),
                primary: index == 0,
            })
            .collect()
    }
}

#[cfg(windows)]
mod windows {
    use std::ptr;

    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
        MONITORINFO, MONITORINFOEXW,
    };
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
        SetProcessDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY,
    };
    use windows_sys::core::BOOL;

    use super::{Area, Display};

    /// 进程还没进窗口库时就要问显示器（窗口位置在 `App::run` 之前就算好）。
    /// 不先声明按每台显示器的缩放来，系统会把坐标虚拟成 96 DPI，和之后窗口库报的物理像素对不上
    fn become_dpi_aware() {
        // SAFETY: 参数是系统定义的常量（每台显示器的 DPI，第二版），不传指针。已经设过时调用会失败，忽略即可
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    fn logical(value: i32, scale: f32) -> f32 {
        value as f32 / scale
    }

    fn area_of(rect: RECT, scale: f32) -> Area {
        Area::new(
            logical(rect.left, scale),
            logical(rect.top, scale),
            logical(rect.right - rect.left, scale),
            logical(rect.bottom - rect.top, scale),
        )
    }

    fn wide_string(words: &[u16]) -> String {
        let end = words
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(words.len());
        String::from_utf16_lossy(&words[..end])
    }

    /// 显示器接口路径（插拔之后还是这块屏）。问不到就用 `\\.\DISPLAYn`
    fn identity(device_name: &[u16]) -> String {
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: `device_name` 是 `GetMonitorInfo` 填好的、以 0 结尾的设备名；
        // `device` 是栈上的 `DISPLAY_DEVICEW`，`cb` 已经写成它的大小，系统最多写这么多
        let found = unsafe {
            EnumDisplayDevicesW(
                device_name.as_ptr(),
                0,
                &mut device,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        };
        if found != 0 {
            let id = wide_string(&device.DeviceID);
            if !id.is_empty() {
                return id;
            }
        }
        wide_string(device_name)
    }

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _device: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` 是 `displays` 传进来的 `Vec<HMONITOR>`，回调返回之前它一直活着
        unsafe {
            let monitors = &mut *(data as *mut Vec<HMONITOR>);
            monitors.push(monitor);
        }
        1
    }

    pub(super) fn displays() -> Vec<Display> {
        become_dpi_aware();
        let mut monitors = Vec::new();
        // SAFETY: 回调只把监视器句柄推进上面这个 `Vec`，函数返回前不会再被调用
        unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(collect_monitor),
                &mut monitors as *mut Vec<HMONITOR> as LPARAM,
            );
        }
        let mut found = Vec::new();
        for monitor in monitors {
            let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: `info` 是栈上的 `MONITORINFOEXW`，`cbSize` 写成了它的大小（比 `MONITORINFO` 大，用来要设备名）
            let ok = unsafe {
                GetMonitorInfoW(
                    monitor,
                    &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
                )
            };
            if ok == 0 {
                continue;
            }
            let mut dpi_x = 0u32;
            let mut dpi_y = 0u32;
            // SAFETY: `monitor` 是刚枚举到的句柄；两个 `u32` 是写出口
            let dpi =
                unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            let scale = if dpi >= 0 && dpi_x > 0 {
                dpi_x as f32 / 96.0
            } else {
                1.0
            };
            let frame = area_of(info.monitorInfo.rcMonitor, scale);
            if frame.width <= 0.0 || frame.height <= 0.0 {
                continue;
            }
            found.push(Display {
                id: identity(&info.szDevice),
                scale,
                frame,
                visible: area_of(info.monitorInfo.rcWork, scale),
                primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
            });
        }
        found.sort_by_key(|display| !display.primary);
        found
    }
}

/// 窗口此刻的样子
#[derive(Clone, Copy, PartialEq)]
struct Shape {
    bounds: Area,
    maximized: bool,
    fullscreen: bool,
    minimized: bool,
}

impl Shape {
    fn of(window: aureole::WindowHandle) -> Self {
        let ([width, height], [x, y]) = (window.size(), window.position());
        Self {
            bounds: Area::new(x, y, width, height),
            maximized: window.is_maximized(),
            fullscreen: window.is_fullscreen(),
            minimized: window.is_minimized(),
        }
    }
}

/// 开始盯着窗口：位置、大小、最大化变了就记下来。停手 0.4 秒之后才写盘，拖动、缩放的过程中不频繁写
pub fn track(window: aureole::WindowHandle, store: Store) {
    use aureole::prelude::*;

    let memory = std::rc::Rc::new(std::cell::RefCell::new(Memory::load(&store)));
    let running = std::rc::Rc::new(std::cell::Cell::new(false));
    Effect::new(move || {
        // 只为了让窗口一变这里就重跑
        let _ = Shape::of(window);
        if running.replace(true) {
            return;
        }
        let (memory, store, running) = (memory.clone(), store.clone(), running.clone());
        // 任务不能记在这个副作用上：它每重跑一次就会取消上一次留下的任务
        aureole::detached(|| {
            aureole::spawn_local(async move {
                let mut last = Shape::of(window);
                loop {
                    aureole::sleep(ms(400)).await;
                    let now = Shape::of(window);
                    if now == last {
                        break;
                    }
                    last = now;
                }
                running.set(false);
                // 全屏、最小化时的大小位置不是「平时」的样子，不记
                if last.fullscreen || last.minimized || last.bounds.width <= 0.0 {
                    return;
                }
                if memory
                    .borrow_mut()
                    .remember(&displays(), last.bounds, last.maximized)
                {
                    memory.borrow().save(&store);
                }
            })
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: &str, scale: f32, frame: Area, primary: bool) -> Display {
        // 菜单栏占顶上 30
        let visible = Area::new(frame.x, frame.y + 30.0, frame.width, frame.height - 30.0);
        Display {
            id: id.to_string(),
            scale,
            frame,
            visible,
            primary,
        }
    }

    fn laptop() -> Display {
        display("laptop", 2.0, Area::new(0.0, 0.0, 1512.0, 982.0), true)
    }

    /// 外接显示器在笔记本右边
    fn external() -> Display {
        display(
            "external",
            1.0,
            Area::new(1512.0, 0.0, 2560.0, 1440.0),
            false,
        )
    }

    #[test]
    fn with_no_records_the_default_size_opens_centered_on_the_main_display() {
        let plan = Memory::default().plan(&[laptop(), external()], (1040.0, 680.0));
        assert_eq!((plan.width, plan.height), (1040.0, 680.0));
        let (x, y) = plan.position.expect("有位置");
        assert_eq!(x, (1512.0 - 1040.0) / 2.0);
        assert_eq!(y, 30.0 + (952.0 - 680.0) / 2.0);
        assert!(!plan.maximized);
    }

    #[test]
    fn each_display_remembers_its_own_window() {
        let displays = [laptop(), external()];
        let mut memory = Memory::default();
        memory.remember(&displays, Area::new(100.0, 80.0, 900.0, 600.0), false);
        memory.remember(&displays, Area::new(2000.0, 200.0, 1400.0, 900.0), false);
        assert_eq!(
            memory.last.as_ref().map(|key| key.id.as_str()),
            Some("external")
        );
        // 上次在外接屏上：回到那里
        let plan = memory.plan(&displays, (1040.0, 680.0));
        assert_eq!(plan.position, Some((2000.0, 200.0)));
        assert_eq!((plan.width, plan.height), (1400.0, 900.0));
        // 外接屏没接：用主显示器自己的那份，不会开到屏幕外面
        let plan = memory.plan(&[laptop()], (1040.0, 680.0));
        assert_eq!(plan.position, Some((100.0, 80.0)));
        assert_eq!((plan.width, plan.height), (900.0, 600.0));
    }

    /// 位置存的是相对显示器的偏移：外接屏在桌面上换了个排列，窗口还在它里面同一个地方
    #[test]
    fn positions_follow_the_display_when_the_arrangement_changes() {
        let mut memory = Memory::default();
        memory.remember(
            &[laptop(), external()],
            Area::new(2000.0, 200.0, 1400.0, 900.0),
            false,
        );
        let moved = display(
            "external",
            1.0,
            Area::new(-2560.0, 0.0, 2560.0, 1440.0),
            false,
        );
        let plan = memory.plan(&[laptop(), moved], (1040.0, 680.0));
        assert_eq!(plan.position, Some((-2560.0 + 488.0, 200.0)));
    }

    /// 同一块显示器换了缩放：记录对不上，位置不套用，大小还沿用，居中
    #[test]
    fn a_changed_scale_or_resolution_is_a_different_record() {
        let mut memory = Memory::default();
        memory.remember(
            &[external()],
            Area::new(1600.0, 100.0, 1200.0, 800.0),
            false,
        );
        let scaled = display("external", 2.0, Area::new(1512.0, 0.0, 1280.0, 720.0), true);
        assert_ne!(scaled.key(), external().key());
        let plan = memory.plan(std::slice::from_ref(&scaled), (1040.0, 680.0));
        assert_eq!(plan.position, Some((1552.0, 30.0)));
        assert_eq!(
            (plan.width, plan.height),
            (1200.0, 690.0),
            "大小夹进新的可见区域"
        );
        // 回到原来的缩放：原来那份还在
        let plan = memory.plan(&[external()], (1040.0, 680.0));
        assert_eq!(plan.position, Some((1600.0, 100.0)));
        // 同一块显示器两种缩放各有一份
        memory.remember(
            std::slice::from_ref(&scaled),
            Area::new(1520.0, 40.0, 1000.0, 600.0),
            false,
        );
        assert_eq!(memory.records.len(), 2);
        assert_eq!(
            memory.plan(&[external()], (1.0, 1.0)).position,
            Some((1600.0, 100.0))
        );
        assert_eq!(
            memory.plan(&[scaled], (1.0, 1.0)).position,
            Some((1520.0, 40.0))
        );
    }

    /// 记录落在屏幕外面（分辨率变小了）：夹回可见区域里
    #[test]
    fn a_saved_window_off_the_screen_is_pulled_back_in() {
        let mut memory = Memory::default();
        let big = display("main", 1.0, Area::new(0.0, 0.0, 2560.0, 1440.0), true);
        memory.remember(
            std::slice::from_ref(&big),
            Area::new(2000.0, 1200.0, 1400.0, 900.0),
            false,
        );
        let plan = memory.plan(&[big], (1040.0, 680.0));
        let (x, y) = plan.position.expect("有位置");
        assert!(
            x >= 0.0 && x + plan.width <= 2560.0,
            "横向在屏内：{x} + {}",
            plan.width
        );
        assert!(
            y >= 30.0 && y + plan.height <= 1440.0,
            "纵向在屏内：{y} + {}",
            plan.height
        );
        let small = display("main", 1.0, Area::new(0.0, 0.0, 1280.0, 800.0), true);
        let mut memory = Memory::default();
        memory.remember(
            &[display(
                "main",
                1.0,
                Area::new(0.0, 0.0, 1280.0, 800.0),
                true,
            )],
            Area::new(1000.0, 700.0, 1000.0, 600.0),
            false,
        );
        let plan = memory.plan(&[small], (1040.0, 680.0));
        let (x, y) = plan.position.expect("有位置");
        assert!(x + plan.width <= 1280.0 && y + plan.height <= 800.0);
    }

    #[test]
    fn maximizing_keeps_the_normal_size() {
        let displays = [laptop()];
        let mut memory = Memory::default();
        memory.remember(&displays, Area::new(100.0, 80.0, 900.0, 600.0), false);
        memory.remember(&displays, Area::new(0.0, 30.0, 1512.0, 952.0), true);
        let plan = memory.plan(&displays, (1040.0, 680.0));
        assert!(plan.maximized);
        assert_eq!(
            (plan.width, plan.height),
            (900.0, 600.0),
            "还原时回到平时的大小"
        );
        assert_eq!(plan.position, Some((100.0, 80.0)));
        memory.remember(&displays, Area::new(100.0, 80.0, 900.0, 600.0), false);
        assert!(!memory.plan(&displays, (1.0, 1.0)).maximized);
    }

    /// 窗口横跨两块显示器：算在重叠多的那块上
    #[test]
    fn a_window_belongs_to_the_display_it_mostly_covers() {
        let displays = [laptop(), external()];
        let straddling = Area::new(1200.0, 100.0, 1000.0, 600.0);
        assert_eq!(
            display_of(&displays, &straddling).map(|d| d.id.as_str()),
            Some("external")
        );
        let mostly_left = Area::new(900.0, 100.0, 1000.0, 600.0);
        assert_eq!(
            display_of(&displays, &mostly_left).map(|d| d.id.as_str()),
            Some("laptop")
        );
        let outside = Area::new(9000.0, 100.0, 400.0, 300.0);
        assert_eq!(
            display_of(&displays, &outside).map(|d| d.id.as_str()),
            Some("external")
        );
    }

    #[test]
    fn without_display_info_only_the_size_is_reused() {
        let plan = Memory::default().plan(&[], (1040.0, 680.0));
        assert_eq!(
            (plan.width, plan.height, plan.position),
            (1040.0, 680.0, None)
        );
        let mut memory = Memory::default();
        memory.records.push(Record {
            key: DisplayKey {
                id: "x".into(),
                scale: 100,
                width: 1,
                height: 1,
            },
            x: 0.0,
            y: 0.0,
            width: 1200.0,
            height: 800.0,
            maximized: false,
        });
        assert_eq!(memory.plan(&[], (1.0, 1.0)).width, 1200.0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_the_attached_displays() {
        let found = displays();
        assert!(!found.is_empty(), "至少有一块显示器");
        assert_eq!(found.iter().filter(|display| display.primary).count(), 1);
        for display in &found {
            assert!(display.scale > 0.0, "{}", display.id);
            assert!(display.frame.width > 0.0 && display.frame.height > 0.0);
            assert!(display.visible.width > 0.0 && display.visible.height > 0.0);
            assert!(!display.id.is_empty());
        }
    }

    #[test]
    fn the_memory_survives_a_round_trip_through_the_store() {
        let directory = crate::store::tests::scratch("window");
        let store = Store::at(&directory);
        let mut memory = Memory::default();
        memory.remember(&[laptop()], Area::new(100.0, 80.0, 900.0, 600.0), false);
        memory.save(&store);
        assert_eq!(Memory::load(&store), memory);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
