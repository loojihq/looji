//! The window's top strip, split in two so the sidebar runs the full height of the window, as in JLC.
//!
//! [`SidebarHead`] sits over the sidebar and holds the app's logo and name. [`TitleBar`] sits over the
//! main pane and holds the window controls. Both are drag regions, and Windows handles dragging,
//! snapping and double-click to maximize from them.

mod window_controls;

use std::sync::{Arc, LazyLock};

use gpui::{prelude::*, *};

use crate::theme::ActiveTheme;
use window_controls::WindowsWindowControls;

/// Zed hard-codes 32px on Windows to match the native caption height. JLC does the same.
pub const TITLE_BAR_HEIGHT: Pixels = px(32.);

/// The app's icon, the same 48px artwork JLC shows at the start of its title bar.
static APP_LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/app-icon-48.png").to_vec(),
    ))
});

/// The head over the sidebar: the logo and the app's name, on the sidebar's colour.
#[derive(IntoElement)]
pub struct SidebarHead {
    title: SharedString,
    width: Pixels,
}

impl SidebarHead {
    pub fn new(title: impl Into<SharedString>, width: Pixels) -> Self {
        Self {
            title: title.into(),
            width,
        }
    }
}

impl RenderOnce for SidebarHead {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .w(self.width)
            .h(TITLE_BAR_HEIGHT)
            .pl(px(6.))
            .bg(theme.palette.surface_raised)
            .border_r_1()
            .border_color(theme.palette.border)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                crate::window_move::drag_from(event, window)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(div().px(px(7.)).child(img(APP_LOGO.clone()).size(px(24.))))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.palette.text_secondary)
                            .child(self.title),
                    ),
            )
    }
}

/// The title bar over the main pane. It holds the window controls, and the rest of it drags the window.
#[derive(IntoElement)]
pub struct TitleBar;

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let background = if window.is_window_active() {
            theme.palette.title_bar
        } else {
            theme.palette.title_bar_inactive
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .flex_none()
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            .bg(background)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                crate::window_move::drag_from(event, window)
            })
            .child(WindowsWindowControls)
    }
}
