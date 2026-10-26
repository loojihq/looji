//! The create and edit dialog for a task. One dialog serves both: Create opens blank, and Edit opens
//! from an existing task and writes to it. The values and their validation are in `task_form.rs`.
//! This file draws the dialog, routes its keys, asks before discarding edits, and saves.
//!
//! Keys belong to the dialog while it's open. Escape closes the discard prompt first, then the
//! dialog. Tab moves between fields and never reaches the board behind.

use gpui::{prelude::*, *};
use looji_domain::{
    Attachment, AttachmentRepository, DomainError, Position, Priority, RepoError, Status, Task,
    TaskEventKind, TaskRepository, TeamMember, TeamMemberId, after, changed_fields,
};

use super::card::tooltip;
use super::fields::{chip, field, member_mark};
use super::members;
use super::task_form::{Field, Mode, TaskForm, Validated};
use super::{WorkspaceView, store};
use crate::{components::edit_box, theme::Theme};

/// An open dialog. `status` is the column a new task goes into.
pub(super) struct TaskDialog {
    pub form: TaskForm,
    /// The discard prompt is showing over the dialog.
    pub confirming: bool,
    /// The assignee list is open under its field.
    pub assignee_open: bool,
    /// The priority menu is open under its select.
    pub priority_open: bool,
    pub status: Status,
    /// The media of the task being edited. It changes as files are added and removed.
    pub media: Vec<Attachment>,
}

impl WorkspaceView {
    /// Opens the form for a new task, inline at the top of the board, or the editor window for an existing
    /// task. The New task button closes the inline form again.
    pub(super) fn open_task_dialog(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let creating = matches!(mode, Mode::Create);
        if creating {
            if self.plan_wizard.is_some() {
                self.request_plan_close(window, cx);
                if self.plan_wizard.is_some() {
                    return;
                }
            }
            if self
                .task_dialog
                .as_ref()
                .is_some_and(|dialog| matches!(dialog.form.mode, Mode::Create))
            {
                return self.request_task_dialog_close(cx);
            }
            if self.projects.active().is_none() {
                self.status = Some("Create a project first.".to_string());
                return cx.notify();
            }
            // The inline form lives on the board, so a screen that stands in for the board gives way.
            self.screen = None;
        }
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
        let status = self.new_task_status();
        // Refresh the members on open, so the list is current even if the team changed since.
        self.reload_members(cx);
        let media = match mode {
            Mode::Edit(id) => store(cx).list_for_task(id).unwrap_or_default(),
            Mode::Create => Vec::new(),
        };
        self.task_dialog = Some(TaskDialog {
            form,
            confirming: false,
            assignee_open: false,
            priority_open: false,
            status,
            media,
        });
        if !creating {
            let main = cx.entity();
            cx.defer(move |cx| {
                super::dialog_window::open(cx, main, super::dialog_window::Kind::Task)
            });
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Every key goes to the dialog while it's open, so the board behind it doesn't see them.
    pub(super) fn task_dialog_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifier = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let member_ids: Vec<TeamMemberId> = self.members.iter().map(TeamMember::id).collect();
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
        // Paste, copy and cut work in whichever text field has the keyboard.
        if modifier && matches!(key, "v" | "c" | "x") {
            if key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    form.type_text(&text);
                }
            } else if let Some(text) = form.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                if key == "x" {
                    form.backspace();
                }
            }
            self.follow_description_caret();
            return cx.notify();
        }
        match key {
            // Escape closes the assignee list first, so it doesn't take the whole dialog with it.
            "escape" if dialog.assignee_open => dialog.assignee_open = false,
            "escape" => return self.request_task_dialog_close(cx),
            "enter" if modifier => return self.save_task_dialog(cx),
            // Enter in a one-line field saves, so a task is quick to add: type the title and press Enter.
            "enter" if matches!(form.active, Field::Title | Field::Estimate) => {
                return self.save_task_dialog(cx);
            }
            // Enter in the description starts a new line.
            "enter" if form.active == Field::Description => form.type_text("\n"),
            "enter" => {
                if form.active == Field::Tags {
                    form.commit_tag();
                } else if form.active == Field::Assignee {
                    dialog.assignee_open = !dialog.assignee_open;
                }
            }
            "left" if form.active == Field::Assignee => form.step_assignee(&member_ids, false),
            "right" if form.active == Field::Assignee => form.step_assignee(&member_ids, true),
            "tab" => {
                if event.keystroke.modifiers.shift {
                    form.focus_previous();
                } else {
                    form.focus_next();
                }
                dialog.assignee_open = false;
            }
            "left" if form.active == Field::Priority => form.step_priority(false),
            "right" if form.active == Field::Priority => form.step_priority(true),
            _ => {
                form.edit_key(&event.keystroke);
            }
        }
        self.follow_description_caret();
        cx.notify();
    }

    /// Scrolls the description area so the caret stays in view while typing, pasting, or moving through the text.
    fn follow_description_caret(&self) {
        if let Some(dialog) = self.task_dialog.as_ref()
            && dialog.form.active == Field::Description
        {
            let edit = &dialog.form.values.description;
            self.description_scroll
                .scroll_to_item(crate::edit_area::caret_chunk(edit.as_str(), edit.caret()));
        }
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
            // Clicking another field closes the assignee list. Its own field leaves it to the
            // toggle, which runs after this press.
            if field != Field::Assignee {
                dialog.assignee_open = false;
            }
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
                // The failure may be a member removed since the dialog opened, so the list is
                // reloaded and the removed member drops out of the choices.
                self.reload_members(cx);
                if let Some(dialog) = self.task_dialog.as_mut() {
                    dialog.form.save_error = Some(message);
                }
            }
        }
        cx.notify();
    }

    /// Picks an assignee from the list, or Unassigned, and closes the list.
    fn choose_task_assignee(&mut self, assignee: Option<TeamMemberId>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.form.values.assignee = assignee;
            dialog.form.focus(Field::Assignee);
            dialog.assignee_open = false;
            cx.notify();
        }
    }

    fn toggle_task_assignee_list(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.form.focus(Field::Assignee);
            dialog.assignee_open = !dialog.assignee_open;
            dialog.priority_open = false;
            self.open_calendar = None;
            cx.notify();
        }
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
        let before = task.clone();
        apply(&mut task, valid).map_err(|err| err.to_string())?;
        let event = match mode {
            Mode::Create => TaskEventKind::Created,
            Mode::Edit(_) => {
                let fields = changed_fields(&before, &task);
                if fields.is_empty() {
                    return Ok(());
                }
                TaskEventKind::Edited {
                    fields: fields.into_iter().map(str::to_string).collect(),
                }
            }
        };
        store
            .save_task_recording(&task, &event)
            .map_err(|err| match err {
                RepoError::Rejected(DomainError::UnknownMember) => {
                    "That team member was removed. Pick someone else or Unassigned, then save."
                        .to_string()
                }
                RepoError::Rejected(rule) => format!("{rule}."),
                err => {
                    // The detail goes to the log, and the user sees a plain message.
                    eprintln!("could not save task {}: {err}", task.id().as_uuid());
                    "Could not save the task. Your edits are still here, so try again.".to_string()
                }
            })
    }
}

impl WorkspaceView {
    /// New tasks go into To do when it's shown, otherwise the first shown column.
    pub(super) fn new_task_status(&self) -> Status {
        self.projects
            .active()
            .map(|project| project.visible_statuses())
            .and_then(|visible| {
                visible
                    .iter()
                    .copied()
                    .find(|status| *status == Status::Todo)
                    .or_else(|| visible.first().copied())
            })
            .unwrap_or(Status::Todo)
    }
}

/// Writes the validated values onto a task.
fn apply(task: &mut Task, valid: &Validated) -> Result<(), DomainError> {
    task.rename(&valid.title)?;
    task.set_description(valid.description.clone());
    task.set_estimate(valid.estimate);
    task.set_priority(valid.priority);
    task.assign(valid.assignee);
    task.set_start_on(valid.start);
    task.set_due_on(valid.due);
    task.set_tags(valid.tags.clone());
    Ok(())
}

/// The dialog's panel and its discard prompt. The dialog window draws the header with the title and
/// close button. Nothing is drawn when no dialog is open.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.task_dialog.as_ref()?;
    let form = &dialog.form;
    let values = &form.values;
    let active = form.active;
    let errors = &form.errors;
    // A new task asks only for what it needs. Its description and tags are added in the editor later. The
    // editor's window shows every field, stacked in one column.
    let wide = matches!(form.mode, Mode::Create);

    let title_box = edit_box(
        &values.title,
        "What needs doing?",
        active == Field::Title,
        theme,
    );
    let estimate_box = edit_box(
        &values.estimate,
        "e.g. 2.5",
        active == Field::Estimate,
        theme,
    );
    let title = field(
        "Title",
        errors.title.as_deref(),
        focusable("field-title", Field::Title, title_box, cx),
        theme,
    );
    let priority = field(
        "Priority",
        None,
        focusable(
            "field-priority",
            Field::Priority,
            priority_select(values.priority, dialog.priority_open, theme, cx),
            cx,
        ),
        theme,
    );
    let assignee = field(
        "Assignee",
        None,
        focusable(
            "field-assignee",
            Field::Assignee,
            assignee_select(
                values.assignee,
                dialog.assignee_open,
                &members::entries(&view.members),
                theme,
                cx,
            ),
            cx,
        ),
        theme,
    );
    let start = field(
        "Start",
        errors.start.as_deref(),
        super::date_picker::date_field(
            super::date_picker::DateTarget::TaskStart,
            &values.start,
            view.open_calendar,
            theme,
            cx,
        ),
        theme,
    );
    let due = field(
        "Due",
        errors.due.as_deref(),
        super::date_picker::date_field(
            super::date_picker::DateTarget::TaskDue,
            &values.due,
            view.open_calendar,
            theme,
            cx,
        ),
        theme,
    );
    let estimate = field(
        "Estimate (hours)",
        errors.estimate.as_deref(),
        focusable("field-estimate", Field::Estimate, estimate_box, cx),
        theme,
    );
    let notes = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .children(form.schedule_warning().map(|message| {
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(message)
        }))
        .children(form.save_error.clone().map(|message| {
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(message)
        }));
    let footer = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(theme.spacing.sm)
        .child(
            crate::components::button(
                "task-cancel",
                "Cancel",
                crate::components::Variant::Ghost,
                theme,
            )
            .on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.request_task_dialog_close(cx)),
            ),
        )
        .child(
            crate::components::button(
                "task-save",
                "Save",
                crate::components::Variant::Primary,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_task_dialog(cx))),
        );

    let body = if wide {
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.sm)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(theme.spacing.sm)
                    .child(div().flex_1().min_w_0().child(title))
                    .child(div().w(px(150.)).flex_none().child(priority))
                    .child(div().w(px(200.)).flex_none().child(assignee)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap(theme.spacing.sm)
                    .child(div().w(px(170.)).flex_none().child(start))
                    .child(div().w(px(170.)).flex_none().child(due))
                    .child(div().w(px(150.)).flex_none().child(estimate))
                    .child(div().flex_1())
                    .child(footer),
            )
            .child(notes)
    } else {
        let description_box = crate::edit_area::edit_area(
            &values.description,
            "Optional",
            active == Field::Description,
            110.,
            &view.description_scroll,
            theme,
        );
        let mut chips = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
        for tag in &values.tags {
            chips = chips.child(chip(tag, theme));
        }
        let tag_draft = edit_box(
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
        let task_id = match form.mode {
            Mode::Edit(id) => Some(id),
            Mode::Create => None,
        };
        // The description, with its paperclip beside the label and its media right under the text.
        let description = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(theme.spacing.xs)
            .child(match task_id {
                Some(id) => super::media::description_label(id, theme, cx),
                None => div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child("Description"),
            })
            .child(
                focusable("field-description", Field::Description, description_box, cx)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col(),
            )
            .children(
                task_id.and_then(|id| super::media::tiles(id, &dialog.media, true, theme, cx)),
            );
        let tags = field(
            "Tags",
            errors.tags.as_deref(),
            focusable("field-tags", Field::Tags, tags_body, cx),
            theme,
        );
        div()
            .flex()
            .flex_col()
            .h_full()
            .gap(theme.spacing.sm)
            .child(title)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.sm)
                    .child(div().flex_1().min_w_0().child(priority))
                    .child(div().flex_1().min_w_0().child(assignee)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.sm)
                    .child(div().flex_1().min_w_0().child(start))
                    .child(div().flex_1().min_w_0().child(due))
                    .child(div().flex_1().min_w_0().child(estimate)),
            )
            .child(description)
            .child(tags)
            .child(notes)
            // The description takes the room that is left, so the buttons sit at the bottom of the window.
            .child(div().flex().flex_row().justify_end().child(footer))
    };

    let panel = div()
        .id("task-panel")
        .flex()
        .flex_col()
        .w_full()
        .h_full()
        .p(theme.spacing.md)
        // A press on the form closes any open menu or calendar. Presses on a menu stay with it.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_task_menus(cx)),
        )
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(body);

    // The panel fills its space. The discard prompt is drawn over it, in the same place.
    let mut layer = div().relative().size_full().child(panel);
    if dialog.confirming {
        layer = layer.child(discard_prompt(theme, cx));
    }
    Some(layer.into_any_element())
}

/// The new-task form for the project header. Nothing is drawn unless a new task is being written.
pub(super) fn new_task_form(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.task_dialog.as_ref()?;
    if !matches!(dialog.form.mode, Mode::Create) {
        return None;
    }
    panel(view, theme, cx)
}

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

/// The priority choices, in the order the menu lists them.
const PRIORITY_CHOICES: [(&str, Option<Priority>); 4] = [
    ("None", None),
    ("Low", Some(Priority::Low)),
    ("Medium", Some(Priority::Medium)),
    ("High", Some(Priority::High)),
];

fn priority_label(priority: Option<Priority>) -> &'static str {
    match priority {
        None => "None",
        Some(Priority::Low) => "Low",
        Some(Priority::Medium) => "Medium",
        Some(Priority::High) => "High",
    }
}

/// The priority select, as the filter bar's selects are: the choice in a small control, and a menu that
/// floats right under it when it's open, so opening it moves nothing.
fn priority_select(
    selected: Option<Priority>,
    open: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let trigger = crate::components::select("task-priority", priority_label(selected), open, theme)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_task_priority_list(cx)));
    // Presses on the select and its menu stay with them, so the form doesn't close the menu first.
    let mut item = div()
        .relative()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);
    if open {
        let rows: Vec<AnyElement> = PRIORITY_CHOICES
            .iter()
            .enumerate()
            .map(|(index, (label, value))| {
                let value = *value;
                crate::components::dropdown_row(
                    ("task-priority-option", index as u64),
                    label,
                    selected == value,
                    false,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_task_priority(value, cx)
                }))
                .into_any_element()
            })
            .collect();
        item = item.child(
            deferred(crate::components::dropdown_menu(
                "task-priority-menu",
                rows,
                theme,
            ))
            .with_priority(1),
        );
    }
    item.into_any_element()
}

/// The assignee select: the current choice, and when open, a menu of the members under it. The first
/// choice is Unassigned, then each member, shown with the same mark and label as on the card.
fn assignee_select(
    selected: Option<TeamMemberId>,
    open: bool,
    entries: &[members::Entry],
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let label = match selected {
        None => "Unassigned".to_string(),
        Some(id) => entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.label.clone())
            // The member was removed after the form opened. The save's message says so.
            .unwrap_or_else(|| "No longer on the team".to_string()),
    };
    let trigger = crate::components::select("task-assignee", &label, open, theme)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_task_assignee_list(cx)));
    let mut item = div()
        .relative()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);
    if open {
        let mut rows =
            vec![assignee_option(0, None, None, selected.is_none(), theme, cx).into_any_element()];
        for (index, entry) in entries.iter().enumerate() {
            rows.push(
                assignee_option(
                    index + 1,
                    Some(entry.id),
                    Some(entry),
                    selected == Some(entry.id),
                    theme,
                    cx,
                )
                .into_any_element(),
            );
        }
        item = item.child(
            deferred(crate::components::dropdown_menu(
                "task-assignee-menu",
                rows,
                theme,
            ))
            .with_priority(1),
        );
    }
    item.into_any_element()
}

impl WorkspaceView {
    /// Opens or closes the priority menu. The assignee menu and the calendar close, so only one is open.
    fn toggle_task_priority_list(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.form.focus(Field::Priority);
            dialog.priority_open = !dialog.priority_open;
            dialog.assignee_open = false;
            self.open_calendar = None;
            cx.notify();
        }
    }

    /// Picks a priority from its menu, and closes the menu.
    fn choose_task_priority(&mut self, priority: Option<Priority>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.priority_open = false;
        }
        self.set_task_priority(priority, cx);
    }

    /// Closes any menu or calendar the form has open. A press outside them calls this.
    pub(super) fn close_task_menus(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.task_dialog.as_mut()
            && (dialog.priority_open || dialog.assignee_open || self.open_calendar.is_some())
        {
            dialog.priority_open = false;
            dialog.assignee_open = false;
            self.open_calendar = None;
            cx.notify();
        }
    }
}

/// One choice in the assignee list. A member's row has their mark, and a tooltip with the full name
/// and the ID that tells same-named members apart.
fn assignee_option(
    index: usize,
    choice: Option<TeamMemberId>,
    entry: Option<&members::Entry>,
    selected: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Stateful<Div> {
    let row = div()
        .id(("assignee-option", index as u64))
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.sm)
        .cursor_pointer()
        .when(selected, |row| row.bg(theme.palette.surface_selected))
        .hover(|style| style.bg(theme.palette.surface_hover))
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| this.choose_task_assignee(choice, cx)),
        );
    match entry {
        Some(entry) => row
            .tooltip(tooltip(members::tooltip(entry)))
            .child(member_mark(&entry.initials, entry.colour, px(22.), theme))
            .child(entry.label.clone()),
        None => row.child("Unassigned"),
    }
}

/// A dialog button. The primary one is the action the dialog is for.
pub(super) fn button(
    id: impl Into<ElementId>,
    label: &str,
    primary: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let variant = if primary {
        crate::components::Variant::Primary
    } else {
        crate::components::Variant::Secondary
    };
    crate::components::button(id, label, variant, theme)
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
