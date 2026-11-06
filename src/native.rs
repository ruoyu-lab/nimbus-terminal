//! macOS 的一处原生补丁。
//!
//! 窗口把标题栏藏起来、内容铺满之后，窗口库的内容视图（非不透明）对 `mouseDownCanMoveWindow` 回答 YES：
//! 在标题栏那一带按住任何东西往旁边拖，系统都会把整个窗口一起拖走，不管元素自己有没有接这次拖动
//! （标签拖动换位就是这样被抢走的）。这里让它回答 NO，窗口只在框架的 `window_drag_area` 标出来的
//! 空白处拖动（那条路走 `performWindowDragWithEvent`，和这个方法无关）。

#[cfg(target_os = "macos")]
pub fn keep_window_still_unless_asked() {
    use std::ffi::CStr;

    use objc2::MainThreadMarker;
    use objc2::ffi::class_replaceMethod;
    use objc2::runtime::{AnyObject, Bool, Sel};
    use objc2::sel;
    use objc2_app_kit::NSApplication;

    extern "C" fn no(_this: *mut AnyObject, _cmd: Sel) -> Bool {
        Bool::NO
    }

    let Some(mtm) = MainThreadMarker::new() else { return };
    #[cfg(target_arch = "aarch64")]
    let types: &CStr = c"B@:";
    #[cfg(not(target_arch = "aarch64"))]
    let types: &CStr = c"c@:";
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        let Some(view) = window.contentView() else { continue };
        let class = view.class();
        // SAFETY: 往这个视图的类上换一个同签名（返回 BOOL、两个隐含参数）的方法，实现是个什么都不碰的函数；
        // 在主线程上、视图已经存在，类不会同时被别的线程改
        unsafe {
            class_replaceMethod(
                std::ptr::from_ref(class).cast_mut(),
                sel!(mouseDownCanMoveWindow),
                std::mem::transmute::<extern "C" fn(*mut AnyObject, Sel) -> Bool, unsafe extern "C-unwind" fn()>(no),
                types.as_ptr(),
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn keep_window_still_unless_asked() {}

/// 临时：往自己的事件队列里投一个事件。kind: 0 移动 1 按下 2 拖动 3 松开
#[cfg(target_os = "macos")]
pub fn post_event(kind: u8, [x, y]: [f32; 2]) -> Option<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSProcessInfo};
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let window = app.windows().iter().next()?;
    let view = window.contentView()?;
    let height = view.bounds().size.height;
    let kind = match kind {
        0 => NSEventType::MouseMoved,
        1 => NSEventType::LeftMouseDown,
        2 => NSEventType::LeftMouseDragged,
        _ => NSEventType::LeftMouseUp,
    };
    let point = NSPoint::new(f64::from(x), if view.isFlipped() { f64::from(y) } else { height - f64::from(y) });
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        view.convertPoint_toView(point, None),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        1.0,
    )?;
    app.postEvent_atStart(&event, false);
    Some(())
}

#[cfg(target_os = "macos")]
pub fn window_frame() -> Option<[f64; 4]> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
    let f = app.windows().iter().next()?.frame();
    Some([f.origin.x, f.origin.y, f.size.width, f.size.height])
}

#[cfg(target_os = "macos")]
pub fn activate() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    if let Some(window) = app.windows().iter().next() {
        window.makeKeyAndOrderFront(None);
        eprintln!("SELFTEST key {} main {}", window.isKeyWindow(), window.isMainWindow());
    }
}
