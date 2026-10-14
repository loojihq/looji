//! The create and edit dialog for a task. One dialog serves both: Create opens blank, and Edit opens
//! from an existing task and writes to it. The values and their validation are in `task_form.rs`.
//! This file draws the dialog, routes its keys, asks before discarding edits, and saves.
//!
//! Keys belong to the dialog while it's open. Escape closes the discard prompt first, then the
//! dialog. Tab moves between fields and never reaches the board behind.

use gpui::{prelude::*, *};
use looji_domain::{DomainError, Position, Priority, Status, Task, TaskRepository, after};

use super::fields::{chip, field, segment, text_box};
use super::task_form::{Field, Mode, TaskForm, Validated};
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

/// An open dialog. `status` is the column a new task goes into.
pub(super) struct TaskDialog {
    pub form: TaskForm,
    /// The discard prompt is showing over the dialog.
    pub confirming: bool,
    pub status: Status,
}

impl WorkspaceView {
    /// Opens the dialog for a new task, or for an existing task on the board.
    pub(super) fn open_task_dialog(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = match mode {
            Mode::Create => TaskForm::create(),
            Mode::Edit(id) => match self.board.iter().find(|task| task.id() == id) {
                Some(task) => TaskForm::edit(task),
                None => {
                    self.status = Some("That task no longer exists.".to_string());
                    return cx.notify();
                }
            },
        };
        // New tasks go into To do when it's shown, otherwise the first shown column.
        let status = self
            .projects
            .active()
            .map(|project| project.visible_statuses())
            .and_then(|visible| {
                visible
                    .iter()
                    .copied()
                    .find(|status| *status == Status::Todo)
                    .or_else(|| visible.first().copied())
            })
            .unwrap_or(Status::Todo);
        self.task_dialog = Some(TaskDialog {
            form,
            confirming: false,
            status,
        });
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Every key goes to the dialog while it's open, so the board behind it doesn't see them.
    pub(super) fn task_dialog_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifier = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let Some(dialog) = self.task_dialog.as_mut() else {
            return;
        };
        if dialog.confirming {
            // "Keep editing" is the default, so Escape and Enter both keep the edits.
            if matches!(key, "escape" | "enter") {
                dialog.confirming = false;
            }
            return cx.notify();
        }
        let form = &mut dialog.form;
        match key {
            "escape" => return self.request_task_dialog_close(cx),
            "enter" if modifier => return self.save_task_dialog(cx),
            "enter" => {
                if form.active == Field::Tags {
                    form.commit_tag();
                }
            }
            "tab" => {
                if event.keystroke.modifiers.shift {
                    form.focus_previous();
                } else {
                    form.focus_next();
                }
            }
            "backspace" => form.backspace(),
            "left" if form.active == Field::Priority => form.step_priority(false),
            "right" if form.active == Field::Priority => form.step_priority(true),
            _ => {
                if !modifier && let Some(text) = typed_text(&event.keystroke) {
                    form.type_text(text);
                }
            }
        }
        cx.notify();
    }

    /// Closes the dialog, or asks first when there are edits that would be lost.
    pub(super) fn request_task_dialog_close(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.task_dialog.as_mut() else {
            return;
        };
        if dialog.form.is_dirty() {
            dialog.confirming = true;
        } else {
            self.task_dialog = None;
        }
        cx.notify();
    }

    pub(super) fn discard_task_dialog(&mut self, cx: &mut Context<Self>) {
        self.task_dialog = None;
        cx.notify();
    }

    fn focus_task_field(&mut self, field: Field, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.form.focus(field);
            cx.notify();
        }
    }

    fn set_task_priority(&mut self, priority: Option<Priority>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.form.values.priority = priority;
            dialog.form.focus(Field::Priority);
            cx.notify();
        }
    }

    /// Validates, then writes. On a validation error the dialog stays open with each field's
    /// message. On a write error it also stays open, with the edits kept, so nothing is lost.
    pub(super) fn save_task_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.task_dialog.as_mut() else {
            return;
        };
        let Some(valid) = dialog.form.validate() else {
            return cx.notify();
        };
        let mode = dialog.form.mode;
        let status = dialog.status;
        match self.write_task(mode, status, &valid, cx) {
            Ok(()) => {
                self.task_dialog = None;
                self.reload_board(cx);
            }
            Err(message) => {
                if let Some(dialog) = self.task_dialog.as_mut() {
                    dialog.form.save_error = Some(message);
                }
            }
        }
        cx.notify();
    }

    /// The one write path for both modes. Create makes a new task at the end of its column. Edit
    /// reads the task fresh from the database and applies the dialog's values to it.
    fn write_task(
        &self,
        mode: Mode,
        status: Status,
        valid: &Validated,
        cx: &App,
    ) -> Result<(), String> {
        let store = store(cx);
        let mut task = match mode {
            Mode::Create => {
                let project = self
                    .projects
                    .active_id()
                    .ok_or_else(|| "Choose a project first.".to_string())?;
                let last = self
                    .board
                    .iter()
                    .rev()
                    .find(|task| task.status() == status)
                    .map(|task| task.position().as_str().to_string());
                let key = after(last.as_deref()).map_err(|err| err.to_string())?;
                let position = Position::new(key).map_err(|err| err.to_string())?;
                Task::new(project, &valid.title, status, position).map_err(|err| err.to_string())?
            }
            Mode::Edit(id) => match TaskRepository::find(store, id) {
                Ok(Some(task)) => task,
                Ok(None) => return Err("That task no longer exists.".to_string()),
                Err(err) => return Err(format!("Could not load the task: {err}")),
            },
        };
        apply(&mut task, valid).map_err(|err| err.to_string())?;
        TaskRepository::save(store, &task).map_err(|err| {
            // The detail goes to the log, and the user sees a plain message.
            eprintln!("could not save task {}: {err}", task.id().as_uuid());
            "Could not save the task. Your edits are still here, so try again.".to_string()
        })
    }
}

/// Writes the validated values onto a task.
fn apply(task: &mut Task, valid: &Validated) -> Result<(), DomainError> {
    task.rename(&valid.title)?;
    task.set_description(valid.description.clone());
    task.set_estimate(valid.estimate);
    task.set_priority(valid.priority);
    task.set_due_on(valid.due);
    task.set_tags(valid.tags.clone());
    Ok(())
}

/// The dialog and its discard prompt, drawn over the window. Nothing is drawn when no dialog is
/// open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.task_dialog.as_ref()?;
    let form = &dialog.form;
    let heading = match form.mode {
        Mode::Create => "New task",
        Mode::Edit(_) => "Edit task",
    };
    let values = &form.values;
    let active = form.active;
    let errors = &form.errors;

    let title_box = text_box(
        &values.title,
        "What needs doing?",
        active == Field::Title,
        theme,
    );
    let description_box = text_box(
        &values.description,
        "Optional",
        active == Field::Description,
        theme,
    );
    let estimate_box = text_box(
        &values.estimate,
        "e.g. 2.5",
        active == Field::Estimate,
        theme,
    );
    let due_box = text_box(&values.due, "YYYY-MM-DD", active == Field::Due, theme);

    let mut chips = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
    for tag in &values.tags {
        chips = chips.child(chip(tag, theme));
    }
    let tag_draft = text_box(
        &values.tag_draft,
        "Type a tag, then comma or Enter",
        active == Field::Tags,
        theme,
    );
    let tags_body = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(chips)
        .child(tag_draft);

    let priority_options = [
        ("None", None),
        ("Low", Some(Priority::Low)),
        ("Medium", Some(Priority::Medium)),
        ("High", Some(Priority::High)),
    ];
    let mut priority_row = div().flex().flex_row().gap(theme.spacing.xs);
    for (index, (label, value)) in priority_options.into_iter().enumerate() {
        let selected = values.priority == value;
        priority_row = priority_row.child(
            segment(selected, theme)
                .id(("priority", index as u64))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_task_priority(value, cx)
                })),
        );
    }

    let panel = div()
        .id("task-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w(px(480.))
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .shadow(theme.elevation.overlay.clone())
        .border_1()
        .border_color(theme.palette.border)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(div().text_size(theme.typography.title).child(heading))
        .child(field(
            "Title (required)",
            errors.title.as_deref(),
            focusable("field-title", Field::Title, title_box, cx),
            theme,
        ))
        .child(field(
            "Description",
            None,
            focusable("field-description", Field::Description, description_box, cx),
            theme,
        ))
        .child(field(
            "Priority",
            None,
            focusable("field-priority", Field::Priority, priority_row, cx),
            theme,
        ))
        .child(field(
            "Estimate, in hours",
            errors.estimate.as_deref(),
            focusable("field-estimate", Field::Estimate, estimate_box, cx),
            theme,
        ))
        .child(field(
            "Due date",
            errors.due.as_deref(),
            focusable("field-due", Field::Due, due_box, cx),
            theme,
        ))
        .child(field(
            "Assignee",
            None,
            div()
                .text_color(theme.palette.text_secondary)
                .child("Unassigned. Team members arrive with the team task."),
            theme,
        ))
        .child(field(
            "Tags",
            errors.tags.as_deref(),
            focusable("field-tags", Field::Tags, tags_body, cx),
            theme,
        ))
        .when_some(form.save_error.clone(), |panel, message| {
            panel.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message),
            )
        })
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.spacing.sm)
                .child(button("task-cancel", "Cancel", false, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.request_task_dialog_close(cx)),
                ))
                .child(button("task-save", "Save", true, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.save_task_dialog(cx)),
                )),
        );

    let backdrop = div()
        .id("task-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.request_task_dialog_close(cx)))
        .child(panel);

    let mut layers = div().absolute().inset_0().child(backdrop);
    if dialog.confirming {
        layers = layers.child(discard_prompt(theme, cx));
    }
    Some(layers.into_any_element())
}

/// Makes a field's body clickable: a click makes that field the active one.
fn focusable(
    id: &'static str,
    field: Field,
    body: impl IntoElement,
    cx: &mut Context<WorkspaceView>,
) -> Stateful<Div> {
    div()
        .id(id)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| this.focus_task_field(field, cx)),
        )
        .child(body)
}

/// A dialog button. The primary one is the action the dialog is for.
fn button(id: &'static str, label: &str, primary: bool, theme: &Theme) -> Stateful<Div> {
    let base = div()
        .id(id)
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .child(label.to_string());
    if primary {
        base.bg(theme.palette.accent)
            .text_color(theme.palette.on_accent)
    } else {
        base.border_1()
            .border_color(theme.palette.border_control)
            .hover(|style| style.bg(theme.palette.surface_hover))
    }
}

/// The prompt that stands between the user and losing their edits. "Keep editing" is the default.
fn discard_prompt(theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    div()
        .id("discard-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .child(
            div()
                .id("discard-panel")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .w(px(360.))
                .p(theme.spacing.lg)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .shadow(theme.elevation.overlay.clone())
                .border_1()
                .border_color(theme.palette.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(theme.typography.title)
                        .child("Discard changes?"),
                )
                .child(
                    div()
                        .text_size(theme.typography.body)
                        .text_color(theme.palette.text_secondary)
                        .child("Your edits to this task haven't been saved."),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(
                            button("discard-keep", "Keep editing", true, theme).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(dialog) = this.task_dialog.as_mut() {
                                        dialog.confirming = false;
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(button("discard-confirm", "Discard", false, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.discard_task_dialog(cx)),
                        )),
                ),
        )
        .into_any_element()
}
