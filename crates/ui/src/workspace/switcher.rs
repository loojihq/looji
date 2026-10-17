//! The workspace switcher at the top of the sidebar. It shows the active workspace. A click opens a
//! menu of every workspace, with a tick on the active one, and New workspace at the foot. Renaming
//! happens in place on the active workspace; icon and delete live on the Preferences screen.

use gpui::{prelude::*, *};
use looji_domain::Workspace;

use super::{Rename, WorkspaceView, avatar};
use crate::{
    components,
    database::AppDatabase,
    theme::{ActiveTheme, Theme},
};

pub(super) fn render(view: &WorkspaceView, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let theme = cx.theme().clone();
    let icons_dir = cx.global::<AppDatabase>().icons_dir.clone();
    let active = view.list.active();

    let trigger = match &view.rename {
        Some(rename) if rename.id == active.id() => rename_field(rename, &theme).into_any_element(),
        _ => trigger(active, &icons_dir, &theme, cx).into_any_element(),
    };

    let mut switcher = div().relative().flex().flex_col().child(trigger);
    if view.workspace_menu {
        // Deferred, so the menu paints above the sidebar entries below it. Without this they draw on top of it.
        switcher = switcher.child(deferred(menu(view, &icons_dir, &theme, cx)).with_priority(1));
    }
    switcher.into_any_element()
}

impl WorkspaceView {
    pub(super) fn toggle_workspace_menu(&mut self, cx: &mut Context<Self>) {
        self.workspace_menu = !self.workspace_menu;
        cx.notify();
    }

    pub(super) fn close_workspace_menu(&mut self, cx: &mut Context<Self>) {
        if self.workspace_menu {
            self.workspace_menu = false;
            cx.notify();
        }
    }
}

/// The closed switcher: the active workspace's mark and name, and a chevron that says it opens.
fn trigger(
    active: &Workspace,
    icons_dir: &std::path::Path,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id("workspace-switcher")
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        // The menu closes on any press outside it. A press on the trigger must not reach that, or
        // the same press would close the menu it just opened.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(avatar::render(active, icons_dir, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(active.name().to_string()),
        )
        .child(components::icon(
            "selector",
            14.,
            theme.palette.text_secondary,
        ))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_workspace_menu(cx)))
}

/// The open menu: one row per workspace, then New workspace. It sits over the sidebar below the trigger.
fn menu(
    view: &WorkspaceView,
    icons_dir: &std::path::Path,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let active = view.list.active_workspace_id();
    let mut list = div()
        .id("workspace-menu")
        .absolute()
        .top(px(40.))
        .left_0()
        .w_full()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .shadow(theme.elevation.overlay.clone())
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    for workspace in view.list.items() {
        let id = workspace.id();
        let is_active = id == active;
        list = list.child(
            div()
                .id(id.as_uuid())
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child(avatar::render(workspace, icons_dir, theme))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(workspace.name().to_string()),
                )
                .child(components::check_mark(is_active, theme))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.workspace_menu = false;
                    this.switch_to(id, cx);
                })),
        );
    }

    list.child(
        div()
            .mt(theme.spacing.xs)
            .pt(theme.spacing.xs)
            .border_t_1()
            .border_color(theme.palette.border)
            .child(
                div()
                    .id("workspace-new")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .px(theme.spacing.sm)
                    .py(theme.spacing.xs)
                    .rounded(theme.radius.md)
                    .cursor_pointer()
                    .text_color(theme.palette.text_secondary)
                    .hover(|style| style.bg(theme.palette.surface_hover))
                    .child(components::icon("plus", 14., theme.palette.text_secondary))
                    .child("New workspace")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.workspace_menu = false;
                        this.create_workspace(window, cx);
                    })),
            ),
    )
}

/// The active workspace's name while it is being renamed. Keys are handled by the view.
fn rename_field(rename: &Rename, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .bg(theme.palette.surface_selected)
                .border_1()
                .border_color(theme.palette.border)
                .child(format!("{}|", rename.draft)),
        )
        .when_some(rename.error.as_ref(), |row, message| {
            row.child(
                div()
                    .px(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message.clone()),
            )
        })
}
