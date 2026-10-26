//! Moving the window from a press on the app's own surfaces.
//!
//! The app draws its own title bars, so a press on one asks GPUI to start the move. A double-click zooms the
//! window, as a title bar does.

use gpui::{MouseDownEvent, Window};

/// Starts moving the window from a left press, or zooms it on a double-click.
pub fn drag_from(event: &MouseDownEvent, window: &mut Window) {
    if event.click_count == 2 {
        window.zoom_window();
    } else {
        window.start_window_move();
    }
}
