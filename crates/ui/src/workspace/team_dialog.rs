//! The team dialog: a project's members, with add, rename, and remove. It opens from the Team button
//! in the header and owns the keyboard while it's open.
//!
//! Removing a member unassigns its tasks and never deletes them. The confirmation says how many tasks
//! that is, and Cancel is the default.

use gpui::{prelude::*, *};
use looji_domain::{
    DomainError, MemberColour, ProjectId, RepoError, TeamMember, TeamMemberId,
    TeamMemberRepository, validate_member_name,
};

use super::card::tooltip;
use super::fields::{member_mark, text_box};
use super::members::{self, Entry};
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

/// A member being renamed, with the name typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rename {
    pub id: TeamMemberId,
    pub draft: String,
    pub error: Option<String>,
}

/// A removal waiting for confirmation. `tasks` is how many tasks it will unassign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Removal {
    pub id: TeamMemberId,
    pub label: String,
    pub tasks: u64,
}

/// An open team dialog. The members themselves are on the view, so they stay current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TeamDialog {
    pub project: ProjectId,
    /// The name typed for a new member.
    pub adding: String,
    pub add_error: Option<String>,
    pub renaming: Option<Rename>,
    pub removing: Option<Removal>,
    /// A failed remove, shown at the foot of the dialog.
    pub error: Option<String>,
}

impl TeamDialog {
    fn new(project: ProjectId) -> Self {
        Self {
            project,
            adding: String::new(),
            add_error: None,
            renaming: None,
            removing: None,
            error: None,
        }
    }
}

/// The message for a rejected name.
fn name_message(err: &DomainError) -> String {
    match err {
        DomainError::EmptyName => "Name can't be empty.".to_string(),
        DomainError::TooLong { max } => format!("Names can be at most {max} characters."),
        other => format!("{other}."),
    }
}

impl WorkspaceView {
    /// Opens the team dialog for the selected project. Nothing opens when no project is selected.
    pub(super) fn open_team_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        self.team_dialog = Some(TeamDialog::new(project));
        self.focus_handle.focus(window);
        cx.notify();
    }

    pub(super) fn close_team_dialog(&mut self, cx: &mut Context<Self>) {
        self.team_dialog = None;
        cx.notify();
    }

    /// Reloads the selected project's members. Cards, the detail panel, and the task dialog read the
    /// list when they draw, so this is all a change to the team needs.
    pub(super) fn reload_members(&mut self, cx: &mut Context<Self>) {
        self.members = match self.projects.active_id() {
            Some(project) => TeamMemberRepository::list_for_project(store(cx), project)
                .unwrap_or_else(|err| {
                    self.status = Some(format!("Could not load the team: {err}"));
                    Vec::new()
                }),
            None => Vec::new(),
        };
    }

    /// Keys while the dialog is open. A confirmation or a rename takes the keys first, then the add
    /// field does.
    pub(super) fn team_dialog_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifier = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let Some(dialog) = self.team_dialog.as_ref() else {
            return;
        };
        let (removing, renaming) = (dialog.removing.is_some(), dialog.renaming.is_some());

        if removing {
            if matches!(key, "escape" | "enter") {
                self.cancel_team_removal(cx);
            }
            return;
        }
        if renaming {
            return match key {
                "enter" => self.commit_team_rename(cx),
                "escape" => self.cancel_team_rename(cx),
                _ => self.edit_team_text(&event.keystroke, modifier, cx),
            };
        }
        match key {
            "escape" => self.close_team_dialog(cx),
            "enter" => self.add_team_member(cx),
            _ => self.edit_team_text(&event.keystroke, modifier, cx),
        }
    }

    /// Types into the rename field while renaming, or into the add field otherwise.
    fn edit_team_text(&mut self, keystroke: &Keystroke, modifier: bool, cx: &mut Context<Self>) {
        let Some(dialog) = self.team_dialog.as_mut() else {
            return;
        };
        if keystroke.key == "backspace" {
            match dialog.renaming.as_mut() {
                Some(rename) => {
                    rename.draft.pop();
                    rename.error = None;
                }
                None => {
                    dialog.adding.pop();
                    dialog.add_error = None;
                }
            }
        } else if !modifier && let Some(text) = typed_text(keystroke) {
            match dialog.renaming.as_mut() {
                Some(rename) => {
                    rename.draft.push_str(text);
                    rename.error = None;
                }
                None => {
                    dialog.adding.push_str(text);
                    dialog.add_error = None;
                }
            }
        } else {
            return;
        }
        cx.notify();
    }

    /// Adds a member with the next colour. Two members may share a name, and they're told apart by
    /// their colour and their place in the list.
    pub(super) fn add_team_member(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.team_dialog.as_ref() else {
            return;
        };
        let (project, name) = (dialog.project, dialog.adding.clone());
        let colour = MemberColour::for_new_member(self.members.len());
        let member = match TeamMember::new(project, &name, colour) {
            Ok(member) => member,
            Err(err) => {
                return self.set_add_error(name_message(&err), cx);
            }
        };
        match TeamMemberRepository::save(store(cx), &member) {
            Ok(()) => {
                if let Some(dialog) = self.team_dialog.as_mut() {
                    dialog.adding.clear();
                    dialog.add_error = None;
                }
                self.reload_members(cx);
            }
            Err(err) => {
                eprintln!("could not add a team member: {err}");
                self.set_add_error("Could not add that member. Try again.".to_string(), cx);
            }
        }
        cx.notify();
    }

    fn set_add_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.team_dialog.as_mut() {
            dialog.add_error = Some(message);
        }
        cx.notify();
    }

    pub(super) fn begin_team_rename(&mut self, id: TeamMemberId, cx: &mut Context<Self>) {
        let Some(member) = self.members.iter().find(|member| member.id() == id) else {
            return;
        };
        let draft = member.name().to_string();
        if let Some(dialog) = self.team_dialog.as_mut() {
            dialog.removing = None;
            dialog.renaming = Some(Rename {
                id,
                draft,
                error: None,
            });
        }
        cx.notify();
    }

    fn cancel_team_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.team_dialog.as_mut() {
            dialog.renaming = None;
        }
        cx.notify();
    }

    /// Saves a rename. A name that's unchanged just closes the editor. A failure keeps the editor open
    /// with the message, so the typed name isn't lost.
    fn commit_team_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = self
            .team_dialog
            .as_ref()
            .and_then(|dialog| dialog.renaming.clone())
        else {
            return;
        };
        let Some(current) = self
            .members
            .iter()
            .find(|member| member.id() == rename.id)
            .cloned()
        else {
            return self.cancel_team_rename(cx);
        };
        let cleaned = match validate_member_name(&rename.draft) {
            Ok(name) => name,
            Err(err) => return self.reject_team_rename(name_message(&err), cx),
        };
        if cleaned == current.name() {
            return self.cancel_team_rename(cx);
        }
        let mut updated = current;
        if let Err(err) = updated.rename(&cleaned) {
            return self.reject_team_rename(name_message(&err), cx);
        }
        match TeamMemberRepository::save(store(cx), &updated) {
            Ok(()) => {
                if let Some(dialog) = self.team_dialog.as_mut() {
                    dialog.renaming = None;
                }
                self.reload_members(cx);
            }
            Err(err) => {
                eprintln!("could not rename a team member: {err}");
                self.reject_team_rename("Could not save the name. Try again.".to_string(), cx);
            }
        }
        cx.notify();
    }

    fn reject_team_rename(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(rename) = self
            .team_dialog
            .as_mut()
            .and_then(|dialog| dialog.renaming.as_mut())
        {
            rename.error = Some(message);
        }
        cx.notify();
    }

    /// Opens the removal confirmation, which says how many tasks will be unassigned.
    pub(super) fn begin_team_removal(&mut self, id: TeamMemberId, cx: &mut Context<Self>) {
        let Some(entry) = members::entries(&self.members)
            .into_iter()
            .find(|entry| entry.id == id)
        else {
            return;
        };
        let removal = match store(cx).assigned_task_count(id) {
            Ok(tasks) => Removal {
                id,
                label: entry.label,
                tasks,
            },
            Err(err) => {
                eprintln!("could not count a team member's tasks: {err}");
                if let Some(dialog) = self.team_dialog.as_mut() {
                    dialog.error = Some("Could not check that member's tasks.".to_string());
                }
                return cx.notify();
            }
        };
        if let Some(dialog) = self.team_dialog.as_mut() {
            dialog.renaming = None;
            dialog.error = None;
            dialog.removing = Some(removal);
        }
        cx.notify();
    }

    fn cancel_team_removal(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.team_dialog.as_mut() {
            dialog.removing = None;
        }
        cx.notify();
    }

    /// Removes the member. Their tasks are unassigned in the same transaction, so the board is
    /// reloaded to show them without the assignee. A failure leaves the member and their tasks as
    /// they were.
    pub(super) fn confirm_team_removal(&mut self, cx: &mut Context<Self>) {
        let Some(removal) = self
            .team_dialog
            .as_ref()
            .and_then(|dialog| dialog.removing.clone())
        else {
            return;
        };
        match TeamMemberRepository::remove(store(cx), removal.id) {
            Ok(_) => {
                if let Some(dialog) = self.team_dialog.as_mut() {
                    dialog.removing = None;
                    dialog.error = None;
                    dialog.renaming.take_if(|rename| rename.id == removal.id);
                }
                self.reload_board(cx);
            }
            Err(err) => {
                eprintln!(
                    "could not remove team member {}: {err}",
                    removal.id.as_uuid()
                );
                if let Some(dialog) = self.team_dialog.as_mut() {
                    dialog.removing = None;
                    dialog.error = Some(match err {
                        RepoError::Rejected(DomainError::UnknownMember) => {
                            "That member was already removed.".to_string()
                        }
                        _ => "Could not remove that member. Nothing was changed.".to_string(),
                    });
                }
                self.reload_members(cx);
            }
        }
        cx.notify();
    }
}

/// The team button in the header. It's drawn only while a project is selected.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("team-open")
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.body)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("Team")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_team_dialog(window, cx)))
}

/// The team dialog, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.team_dialog.as_ref()?;
    let project = view
        .projects
        .active()
        .map(|project| project.name().to_string())
        .unwrap_or_default();
    let entries = members::entries(&view.members);

    let mut list = div().flex().flex_col().gap(theme.spacing.xs);
    if entries.is_empty() {
        list = list.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("No one is on this team yet. Add the people who work on this project."),
        );
    }
    for (index, entry) in entries.iter().enumerate() {
        list = list.child(member_row(index, entry, dialog, theme, cx));
    }

    let adding_active = dialog.renaming.is_none() && dialog.removing.is_none();
    let add_row = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.sm)
        .items_center()
        .child(
            div()
                .id("team-add-field")
                .flex_1()
                .min_w_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        if let Some(dialog) = this.team_dialog.as_mut() {
                            dialog.renaming = None;
                        }
                        cx.notify();
                    }),
                )
                .child(text_box(
                    &dialog.adding,
                    "Add a name, then press Enter",
                    adding_active,
                    theme,
                )),
        )
        .child(
            button("team-add", "Add", true, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_team_member(cx))),
        );

    let panel =
        div()
            .id("team-panel")
            .flex()
            .flex_col()
            .gap(theme.spacing.md)
            .w(px(460.))
            .max_h(px(560.))
            .p(theme.spacing.lg)
            .rounded(theme.radius.lg)
            .bg(theme.palette.surface_raised)
            .shadow(theme.elevation.overlay.clone())
            .border_1()
            .border_color(theme.palette.border)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(div().text_size(theme.typography.title).child("Team"))
            .child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(format!("Members of {project}")),
            )
            .child(list)
            .child(add_row)
            .when_some(dialog.add_error.clone(), |panel, message| {
                panel.child(error_text(message, theme))
            })
            .when_some(dialog.error.clone(), |panel, message| {
                panel.child(error_text(message, theme))
            })
            .child(div().flex().flex_row().justify_end().child(
                button("team-done", "Done", false, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.close_team_dialog(cx)),
                ),
            ));

    Some(
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("team-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.palette.scrim)
                    .occlude()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_team_dialog(cx)))
                    .child(panel),
            )
            .into_any_element(),
    )
}

/// One member: a row with their mark and name, or the rename field, or the removal confirmation.
fn member_row(
    index: usize,
    entry: &Entry,
    dialog: &TeamDialog,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    if let Some(removal) = dialog
        .removing
        .as_ref()
        .filter(|removal| removal.id == entry.id)
    {
        return div()
            .id(("team-row", index as u64))
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .p(theme.spacing.sm)
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .child(format!("Remove {}?", removal.label))
            .child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(members::unassigned_message(removal.tasks)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(theme.spacing.sm)
                    .child(
                        button(("team-keep", index as u64), "Keep", true, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_team_removal(cx)),
                        ),
                    )
                    .child(
                        button(
                            ("team-confirm-remove", index as u64),
                            "Remove",
                            false,
                            theme,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm_team_removal(cx)
                            }),
                        ),
                    ),
            )
            .into_any_element();
    }

    let id = entry.id;
    let mark = member_mark(&entry.initials, entry.colour, px(26.), theme);
    let name = match dialog
        .renaming
        .as_ref()
        .filter(|rename| rename.id == entry.id)
    {
        Some(rename) => text_box(&rename.draft, "Name", true, theme).into_any_element(),
        None => div()
            .id(("team-name", index as u64))
            .flex_1()
            .min_w_0()
            .truncate()
            .tooltip(tooltip(members::tooltip(entry)))
            .child(entry.label.clone())
            .into_any_element(),
    };
    let renaming = dialog
        .renaming
        .as_ref()
        .is_some_and(|rename| rename.id == entry.id);

    let actions = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.xs)
        .when(!renaming, |row| {
            row.child(
                button(("team-rename", index as u64), "Rename", false, theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.begin_team_rename(id, cx)),
                ),
            )
            .child(
                button(("team-remove", index as u64), "Remove", false, theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.begin_team_removal(id, cx)),
                ),
            )
        });

    let rename_error = dialog
        .renaming
        .as_ref()
        .filter(|rename| rename.id == entry.id)
        .and_then(|rename| rename.error.clone());

    div()
        .id(("team-row", index as u64))
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(mark)
                .child(name)
                .child(actions),
        )
        .when_some(rename_error, |row, message| {
            row.child(error_text(message, theme))
        })
        .into_any_element()
}

fn error_text(message: String, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.danger)
        .child(message)
}
