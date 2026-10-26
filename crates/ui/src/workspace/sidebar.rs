//! The sidebar: the workspace switcher at the top, then the screens and the projects. Workspace
//! settings (icon, rename, delete) and the app's preferences live on the Preferences screen.

use gpui::{prelude::*, *};

use super::screen::Screen;
use super::{WorkspaceView, projects, switcher};
use crate::{
    components,
    theme::{ActiveTheme, Theme},
};

pub(super) const SIDEBAR_WIDTH: Pixels = px(224.);

pub fn render(view: &WorkspaceView, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(theme.spacing.xs)
        .w(SIDEBAR_WIDTH)
        .flex_1()
        .min_h_0()
        .p(theme.spacing.sm)
        .bg(theme.palette.surface_raised)
        .border_r_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.body)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            crate::window_move::drag_from(event, window)
        })
        .child(switcher::render(view, cx))
        .child(
            div()
                .mt(theme.spacing.sm)
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .child(screen_item(Screen::Today, "sun", "Today", view, &theme, cx))
                .child(screen_item(
                    Screen::Week,
                    "calendar-week",
                    "Week",
                    view,
                    &theme,
                    cx,
                )),
        )
        .child(projects::sidebar_section(view, &theme, cx))
        .child(div().flex_1())
        .child(
            crate::components::nav_item(
                "sidebar-preferences",
                "settings",
                "Preferences",
                false,
                None,
                &theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_preferences(cx))),
        )
        .when_some(view.status.as_ref(), |sidebar, message| {
            sidebar.child(
                div()
                    .px(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message.clone()),
            )
        })
}

/// A sidebar entry that opens a workspace screen. It is highlighted while that screen is showing.
fn screen_item(
    screen: Screen,
    icon: &str,
    label: &'static str,
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id: &'static str = match screen {
        Screen::Today => "sidebar-today",
        Screen::Week => "sidebar-week",
        Screen::Workspace => "sidebar-workspace",
    };
    // Today shows how many tasks need attention, as the notification settings choose.
    let count = (screen == Screen::Today)
        .then(|| view.attention_count())
        .filter(|count| *count > 0)
        .map(|count| count.to_string());
    components::nav_item(
        id,
        icon,
        label,
        view.screen == Some(screen),
        count.as_deref(),
        theme,
    )
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_screen(screen, cx)))
}
