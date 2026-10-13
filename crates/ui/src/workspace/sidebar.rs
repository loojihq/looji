//! The workspace list: rows to switch between, a rename field, and a create button.

use gpui::{prelude::*, *};
use looji_domain::WorkspaceId;

use super::{Rename, WorkspaceView};
use crate::theme::{ActiveTheme, Theme};

const SIDEBAR_WIDTH: Pixels = px(240.);

pub fn render(view: &WorkspaceView, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let active = view.list.active_workspace_id();

    let rows: Vec<AnyElement> = view
        .list
        .items()
        .iter()
        .map(|workspace| {
            let id = workspace.id();
            match &view.rename {
                Some(rename) if rename.id == id => editing_row(rename, &theme).into_any_element(),
                _ => label_row(workspace.name(), id == active, id, &theme, cx).into_any_element(),
            }
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_1()
        .w(SIDEBAR_WIDTH)
        .h_full()
        .p_2()
        .bg(theme.panel_background)
        .border_r_1()
        .border_color(theme.border)
        .text_sm()
        .children(rows)
        .child(new_workspace_button(&theme, cx))
        .when_some(view.status.as_ref(), |sidebar, message| {
            sidebar.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(theme.error)
                    .child(message.clone()),
            )
        })
}

fn label_row(
    name: &str,
    is_active: bool,
    id: WorkspaceId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(id.as_uuid())
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .when(is_active, |row| row.bg(theme.element_selected))
        .when(!is_active, |row| {
            row.hover(|style| style.bg(theme.ghost_element_hover))
        })
        // One click switches; double-click starts renaming.
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.begin_rename(id, window, cx);
            } else {
                this.switch_to(id, cx);
            }
        }))
        .child(div().min_w_0().truncate().child(name.to_string()))
        .child(rename_button(id, theme, cx))
}

fn rename_button(
    id: WorkspaceId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(("rename", id.as_uuid().as_u64_pair().0))
        .flex_shrink_0()
        .px_1()
        .text_xs()
        .text_color(theme.text.opacity(0.6))
        .hover(|style| style.text_color(theme.text))
        .child("Rename")
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            // Keep this click from also switching to the row.
            cx.stop_propagation();
            this.begin_rename(id, window, cx);
        }))
}

/// The row being renamed. Keys are handled by the view; clicks here stay in the field.
fn editing_row(rename: &Rename, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme.element_selected)
                .border_1()
                .border_color(theme.border)
                .child(format!("{}|", rename.draft)),
        )
        .when_some(rename.error.as_ref(), |row, message| {
            row.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(theme.error)
                    .child(message.clone()),
            )
        })
}

fn new_workspace_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("new-workspace")
        .mt_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(theme.ghost_element_hover))
        .child("+ New workspace")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.create_workspace(window, cx);
        }))
}
