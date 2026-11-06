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
