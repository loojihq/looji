//! The root view of the main window: a title bar above a sidebar and main pane.

use gpui::{prelude::*, *};

use crate::{
    APP_NAME,
    theme::{ActiveTheme, Theme},
    title_bar::TitleBar,
};

const SIDEBAR_WIDTH: Pixels = px(240.);
const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1280.), px(800.));
/// Below this the sidebar and main pane can't both render usefully.
const MIN_WINDOW_SIZE: Size<Pixels> = size(px(640.), px(400.));

pub struct Workspace;

impl Workspace {
    pub fn open_window(cx: &mut App) -> Result<WindowHandle<Self>> {
        cx.open_window(window_options(cx), |_, cx| cx.new(|_| Self))
    }
}

fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            DEFAULT_WINDOW_SIZE,
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            // Hides the native caption; `TitleBar` draws our own.
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_id: Some(APP_NAME.to_lowercase()),
        window_min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.editor_background)
            .text_color(theme.text)
            .child(TitleBar::new(APP_NAME))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar(theme))
                    .child(main_pane()),
            )
    }
}

fn sidebar(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(SIDEBAR_WIDTH)
        .h_full()
        .p_3()
        .bg(theme.panel_background)
        .border_r_1()
        .border_color(theme.border)
        .text_sm()
        .child("Sidebar")
}

fn main_pane() -> impl IntoElement {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .justify_center()
        .items_center()
        .text_xl()
        .child("Main pane")
}
