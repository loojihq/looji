//! The workspace list: rows to switch between, a rename field, and a create button.

use std::path::Path;

use gpui::{prelude::*, *};
use looji_domain::{Workspace, WorkspaceId};

use super::{Rename, WorkspaceView, avatar, projects};
use crate::{
    database::AppDatabase,
    theme::{ActiveTheme, Mode, Theme},
};

const SIDEBAR_WIDTH: Pixels = px(240.);

pub fn render(view: &WorkspaceView, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let active = view.list.active_workspace_id();
    let icons_dir = cx.global::<AppDatabase>().icons_dir.clone();

    let rows: Vec<AnyElement> = view
        .list
        .items()
        .iter()
        .map(|workspace| {
            let id = workspace.id();
            match &view.rename {
                Some(rename) if rename.id == id => editing_row(rename, &theme).into_any_element(),
                _ => label_row(workspace, id == active, &icons_dir, &theme, cx).into_any_element(),
            }
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(theme.spacing.xs)
        .w(SIDEBAR_WIDTH)
        .h_full()
        .p(theme.spacing.sm)
        .bg(theme.palette.surface_raised)
        .border_r_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.body)
        .children(rows)
        .child(new_workspace_button(&theme, cx))
        .child(projects::sidebar_section(view, &theme, cx))
        .child(theme_toggle(&theme, cx))
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

fn label_row(
    workspace: &Workspace,
    is_active: bool,
    icons_dir: &Path,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = workspace.id();
    div()
        .id(id.as_uuid())
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .when(is_active, |row| row.bg(theme.palette.surface_selected))
        .when(!is_active, |row| {
            row.hover(|style| style.bg(theme.palette.surface_hover))
        })
        // One click switches; double-click starts renaming.
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.begin_rename(id, window, cx);
            } else {
                this.switch_to(id, cx);
            }
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .min_w_0()
                .child(avatar::render(workspace, icons_dir, theme))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(workspace.name().to_string()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .child(icon_button(id, theme, cx))
                .child(rename_button(id, theme, cx))
                .child(delete_button(id, theme, cx)),
        )
}

fn icon_button(
    id: WorkspaceId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(("icon", id.as_uuid().as_u64_pair().0))
        .px(theme.spacing.xs)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .hover(|style| style.text_color(theme.palette.text_primary))
        .child("Icon")
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            // Keep this click from also switching to the row.
            cx.stop_propagation();
            this.choose_icon(id, cx);
        }))
}

fn rename_button(
    id: WorkspaceId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(("rename", id.as_uuid().as_u64_pair().0))
        .flex_shrink_0()
        .px(theme.spacing.xs)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .hover(|style| style.text_color(theme.palette.text_primary))
        .child("Rename")
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            // Keep this click from also switching to the row.
            cx.stop_propagation();
            this.begin_rename(id, window, cx);
        }))
}

fn delete_button(
    id: WorkspaceId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(("delete", id.as_uuid().as_u64_pair().0))
        .px(theme.spacing.xs)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .hover(|style| style.text_color(theme.palette.danger))
        .child("Delete")
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            // Keep this click from also switching to the row.
            cx.stop_propagation();
            this.begin_delete(id, window, cx);
        }))
}

/// The row being renamed. Keys are handled by the view; clicks here stay in the field.
fn editing_row(rename: &Rename, theme: &Theme) -> impl IntoElement {
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

fn new_workspace_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("new-workspace")
        .mt(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("+ New workspace")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.create_workspace(window, cx);
        }))
}

/// Switches between light and dark mode. The label names the mode the click switches to.
fn theme_toggle(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let target = match theme.mode {
        Mode::Dark => "Switch to light",
        Mode::Light => "Switch to dark",
    };
    div()
        .id("theme-toggle")
        .mt(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .text_color(theme.palette.text_secondary)
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(target)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_theme(cx)))
}
