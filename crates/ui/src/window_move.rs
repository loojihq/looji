//! Moving the window from a press on the app's own surfaces.
//!
//! The drag regions GPUI sets on Windows don't reliably reach the title bar, so the app asks the system
//! to move the window itself. That is the caption drag Windows starts from a real title bar. Elsewhere
//! GPUI starts the move. A double-click zooms the window, as a title bar does.

use gpui::{MouseDownEvent, Window};

/// Starts moving the window from a left press, or zooms it on a double-click.
pub fn drag_from(event: &MouseDownEvent, window: &mut Window) {
    if event.click_count == 2 {
        window.zoom_window();
    } else {
        start_move(window);
    }
}

#[cfg(target_os = "windows")]
fn start_move(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{
            Input::KeyboardAndMouse::ReleaseCapture,
            WindowsAndMessaging::{HTCAPTION, SendMessageW, WM_NCLBUTTONDOWN},
        },
    };

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut std::ffi::c_void);
    // Releasing the capture lets the system track the mouse for the move, as it does for a title bar.
    unsafe {
        let _ = ReleaseCapture();
        SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            Some(WPARAM(HTCAPTION as usize)),
            Some(LPARAM(0)),
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn start_move(window: &Window) {
    window.start_window_move();
}
