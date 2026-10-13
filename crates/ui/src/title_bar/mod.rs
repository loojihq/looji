//! Custom titlebar, mirroring Zed's Windows titlebar
//! (crates/platform_title_bar/src/platform_title_bar.rs).

mod window_controls;

use gpui::{prelude::*, *};

use crate::theme::ActiveTheme;
use window_controls::WindowsWindowControls;

/// Zed hard-codes 32px on Windows to match the native caption height.
const TITLE_BAR_HEIGHT: Pixels = px(32.);

#[derive(IntoElement)]
pub struct TitleBar {
    title: SharedString,
}

impl TitleBar {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let background = if window.is_window_active() {
            theme.title_bar_background
        } else {
            theme.title_bar_inactive_background
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            .pl_2()
            .bg(background)
            // Windows handles dragging, snapping, and double-click to
            // maximize for this region.
            .window_control_area(WindowControlArea::Drag)
            .child(div().text_sm().child(self.title))
            .child(WindowsWindowControls)
    }
}
