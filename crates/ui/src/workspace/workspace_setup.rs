//! The workspace screen: the workspace's identity and its actions, its projects, its team, and delete.
//! It opens when a workspace is created, so the next step after naming it is a project.
//!
//! The team belongs to the workspace, so every project in it shares the same people. The team is
//! edited in the team dialog, opened from the team panel here.

use gpui::{prelude::*, *};

use super::{Rename, WorkspaceView, avatar, switcher};
use crate::{components, database::AppDatabase, theme::Theme};

/// The body of the workspace screen, under its header: an identity card, then projects and team side
/// by side, and delete at the foot.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let workspace = view.list.active();
    let id = workspace.id();
    let icons_dir = cx.global::<AppDatabase>().icons_dir.clone();
    let detail = format!(
        "{} · {}",
        plural(view.projects.items().len(), "project", "projects"),
        plural(view.members.len(), "person", "people"),
    );

    // While the name is being edited, the identity card shows the field with its own Save and Cancel,
    // so the name isn't saved only by pressing Enter.
    let (identity, actions) = match &view.rename {
        Some(rename) if rename.id == id => (rename_row(rename, theme, cx), None),
        _ => (
            name_block(workspace.name(), &detail, theme),
            Some(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.sm)
                    .child(
                        components::button(
                            "workspace-rename",
                            "Rename",
                            components::Variant::Secondary,
                            theme,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.begin_rename(id, window, cx);
                            },
                        )),
                    )
                    .child(
                        components::button(
                            "workspace-change-icon",
                            "Change icon",
                            components::Variant::Secondary,
                            theme,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.choose_icon(id, cx);
                            },
                        )),
                    )
                    .into_any_element(),
            ),
        ),
    };

    let hero = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.lg)
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface_raised)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(72.))
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.palette.border)
                .bg(theme.palette.surface)
                .child(avatar::render(workspace, &icons_dir, px(56.), theme)),
        )
        .child(div().flex_1().min_w_0().child(identity))
        .children(actions);

    let projects = projects_panel(view, theme, cx);
    let team = team_panel(view, theme, cx);
    let columns = div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.spacing.lg)
        .child(div().flex_1().min_w_0().child(projects))
        .child(div().flex_none().w(px(300.)).child(team));

    // While this workspace's delete is being confirmed, the card shows the confirmation in place of its button.
    let confirming = view
        .pending_delete
        .as_ref()
        .filter(|pending| pending.id == id);
    let delete_card = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.lg)
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Delete this workspace"),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child("Removes its projects and tasks. This can't be undone."),
                ),
        )
        .child(
            components::button(
                "workspace-delete",
                "Delete",
                components::Variant::Danger,
                theme,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.begin_delete(id, window, cx);
            })),
        );
    let delete = match confirming {
        Some(pending) => super::delete::dialog(pending, theme, cx).into_any_element(),
        None => delete_card.into_any_element(),
    };

    div()
        .id("workspace-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(theme.spacing.lg)
        .py(theme.spacing.lg)
        .child(
            div()
                .w_full()
                .max_w(px(960.))
                .flex()
                .flex_col()
                .gap(theme.spacing.lg)
                .child(hero)
                .child(columns)
                .child(delete),
        )
        .into_any_element()
}

/// The workspace's name, with a line under it saying what it holds.
fn name_block(name: &str, detail: &str, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Workspace"),
        )
        .child(
            div()
                .mt(theme.spacing.xs)
                .text_size(theme.typography.heading)
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(name.to_string()),
        )
        .child(
            div()
                .mt(px(2.))
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(detail.to_string()),
        )
        .into_any_element()
}

/// The name being edited: the field, then Cancel and Save.
fn rename_row(rename: &Rename, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(switcher::rename_field(rename, theme)),
        )
        .child(
            components::button(
                "workspace-name-cancel",
                "Cancel",
                components::Variant::Ghost,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_rename(cx))),
        )
        .child(
            components::button(
                "workspace-name-save",
                "Save",
                components::Variant::Primary,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.commit_rename(cx))),
        )
        .into_any_element()
}

/// A bordered panel with a header and a body. The header holds the title, a count, and one action.
fn panel(
    title: &str,
    detail: &str,
    action: AnyElement,
    body: impl IntoElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.spacing.sm)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_size(theme.typography.caption)
                                .text_color(theme.palette.text_secondary)
                                .child(detail.to_string()),
                        ),
                )
                .child(action),
        )
        .child(body)
}

/// The projects panel: each project with an Open button, and New project in the header.
fn projects_panel(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> Div {
    let mut rows = div().flex().flex_col().gap(theme.spacing.xs);
    // The create form opens at the top of this list, above the projects.
    if let Some(form) = view.project_dialog.as_ref() {
        rows = rows.child(super::projects::dialog(form, theme, cx));
    }
    if view.projects.items().is_empty() {
        rows = rows.child(
            div()
                .py(theme.spacing.sm)
                .text_color(theme.palette.text_secondary)
                .child("No projects yet. A project holds a board of tasks."),
        );
    }
    for (index, project) in view.projects.items().iter().enumerate() {
        let project_id = project.id();
        // The whole row opens the project, so there's no button to draw an outline around.
        rows = rows.child(
            div()
                .id(("workspace-project", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .px(theme.spacing.sm)
                .py(theme.spacing.sm)
                .rounded(theme.radius.md)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child(components::dot(theme.palette.border_control, 7.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(project.name().to_string()),
                )
                .child(components::icon(
                    "arrow-up-right",
                    16.,
                    theme.palette.text_secondary,
                ))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.select_project(project_id, cx);
                })),
        );
    }

    // The button turns into Cancel while the form is open, as the form's own Cancel does.
    let (label, variant) = if view.project_dialog.is_some() {
        ("Cancel", components::Variant::Secondary)
    } else {
        ("New project", components::Variant::Primary)
    };
    let new_project = components::button("workspace-new-project", label, variant, theme)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.open_project_dialog(window, cx);
        }))
        .into_any_element();
    let detail = plural(view.projects.items().len(), "project", "projects");
    panel("Projects", &detail, new_project, rows, theme)
}

/// The team panel: the workspace's members, each with their colour mark, and Manage team in the header.
fn team_panel(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> Div {
    // While the team is being edited, the card holds the editor, and its button is Done.
    if view.team_dialog.is_some() {
        let done = components::button(
            "workspace-team-done",
            "Done",
            components::Variant::Secondary,
            theme,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_team_dialog(cx)))
        .into_any_element();
        let detail = plural(view.members.len(), "person", "people");
        let body =
            super::team_dialog::inline(view, theme, cx).unwrap_or_else(|| div().into_any_element());
        return panel("Team", &detail, done, body, theme);
    }
    let mut rows = div().flex().flex_col().gap(theme.spacing.xs);
    if view.members.is_empty() {
        rows = rows.child(
            div()
                .py(theme.spacing.sm)
                .text_color(theme.palette.text_secondary)
                .child(
                    "No one is on this team yet. Add the people who work across this workspace.",
                ),
        );
    }
    for member in &view.members {
        rows = rows.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .child(components::member_mark(
                    &member.initials(),
                    member.colour(),
                    px(22.),
                    theme,
                ))
                .child(div().min_w_0().truncate().child(member.name().to_string())),
        );
    }

    let manage = components::button(
        "workspace-manage-team",
        "Manage team",
        components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
        this.open_team_dialog(window, cx);
    }))
    .into_any_element();
    let detail = plural(view.members.len(), "person", "people");
    panel("Team", &detail, manage, rows, theme)
}

/// "1 project" or "3 projects".
fn plural(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}
