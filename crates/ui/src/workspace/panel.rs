//! The task detail panel: the full task, its history, and its AI chat. It reads
//! the task from the board by ID, so an edit made here changes the card straight away. Each edit is
//! written with its history entry, then the board's copy is updated in place, with no reload.
//!
//! When the task is deleted while the panel is open, the panel closes and the board's status line
//! says so. A draft the user was typing is logged rather than thrown away silently.

use gpui::{prelude::*, *};
use looji_domain::{
    Attachment, AttachmentRepository, Status, Task, TaskEvent, TaskEventKind, TaskId, TeamMemberId,
    Timestamp,
};

use super::board::status_label;
use super::card::{civil_from_days, tooltip};
use super::drag::DropResult;
use super::fields::{field, member_mark, segment};
use super::media;
use super::members;
use super::task_editors;
use super::{WorkspaceView, store};
use crate::components::{edit_box, prop_row};
use crate::{text_edit::TextEdit, theme::Theme};

/// The text fields the panel can edit inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DraftField {
    Title,
    Description,
}

/// Text being typed into an inline field. Committed on Enter, dropped on Escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Draft {
    pub field: DraftField,
    pub text: TextEdit,
}

/// The panel's state. It holds the task's ID, not a copy of the task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PanelState {
    pub task: TaskId,
    /// Newest first, as it's shown.
    pub history: Vec<TaskEvent>,
    pub draft: Option<Draft>,
    /// The task's media, shown with its description.
    pub media: Vec<Attachment>,
    /// Why the last edit didn't save, shown until the next attempt.
    pub error: Option<String>,
    /// Plain information, such as a link that isn't available yet.
    pub notice: Option<String>,
    /// Other issues that look like this one.
    pub related: Vec<looji_knowledge::related::RelatedIssue>,
    /// What similar finished tasks suggest for this one.
    pub hints: looji_knowledge::hints::Hints,
    /// Quiet risk notes: stale, overdue, or due soon with nothing committed.
    pub risks: Vec<looji_knowledge::risk::Note>,
    /// The task's reviews, newest first.
    pub review_history: Vec<looji_domain::ReviewRecord>,
}

/// What a history entry says, in words.
pub(super) fn describe(event: &TaskEvent) -> String {
    match &event.kind {
        TaskEventKind::Created => "Created".to_string(),
        TaskEventKind::Edited { fields } => format!("Edited {}", fields.join(", ")),
        TaskEventKind::StatusChanged { from, to } => {
            format!(
                "Moved from {} to {}",
                status_label(*from),
                status_label(*to)
            )
        }
    }
}

/// A timestamp as UTC, for display. Stored values stay in UTC.
pub(super) fn format_time(at: Timestamp) -> String {
    let millis = at.unix_millis();
    let (year, month, day) = civil_from_days(millis.div_euclid(86_400_000));
    let minutes = millis.rem_euclid(86_400_000) / 60_000;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// The assignee as the panel shows them: the same mark and label as the dialog's list, with the full
/// name and ID in a tooltip. A member who has been removed says so rather than showing a stale name.
fn assignee_row(
    assignee: Option<TeamMemberId>,
    entries: &[members::Entry],
    theme: &Theme,
) -> AnyElement {
    let Some(id) = assignee else {
        return div()
            .text_color(theme.palette.text_secondary)
            .child("Assignee: Unassigned")
            .into_any_element();
    };
    match entries.iter().find(|entry| entry.id == id) {
        Some(entry) => div()
            .id("panel-assignee")
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .tooltip(tooltip(members::tooltip(entry)))
            .child("Assignee:")
            .child(member_mark(&entry.initials, entry.colour, px(22.), theme))
            .child(entry.label.clone())
            .into_any_element(),
        None => div()
            .text_color(theme.palette.danger)
            .child("Assignee: no longer on the team")
            .into_any_element(),
    }
}

impl WorkspaceView {
    /// Opens the panel for a task on the board, with its history.
    pub(super) fn open_panel(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let history = match store(cx).task_history(task) {
            Ok(events) => events.into_iter().rev().collect(),
            Err(err) => {
                eprintln!(
                    "could not load the history of task {}: {err}",
                    task.as_uuid()
                );
                Vec::new()
            }
        };
        let media = store(cx).list_for_task(task).unwrap_or_else(|err| {
            eprintln!("could not load the media of task {}: {err}", task.as_uuid());
            Vec::new()
        });
        self.load_review(task, cx);
        let review_history = looji_domain::ReviewHistoryRepository::reviews(store(cx), task)
            .unwrap_or_else(|err| {
                eprintln!(
                    "could not load the reviews of task {}: {err}",
                    task.as_uuid()
                );
                Vec::new()
            });
        let related = match (
            self.projects.active_id(),
            self.board.iter().find(|t| t.id() == task),
        ) {
            (Some(project), Some(found)) => super::knowledge::related_for(
                &cx.global::<crate::database::AppDatabase>().knowledge_path,
                project,
                task,
                found.title(),
                found.description(),
            ),
            _ => Vec::new(),
        };
        let hints = hints_for(&related, &self.board);
        let risks = match (
            self.projects.active_id(),
            self.board.iter().find(|t| t.id() == task),
        ) {
            (Some(project), Some(found)) => super::knowledge::risks_for(
                &cx.global::<crate::database::AppDatabase>().knowledge_path,
                project,
                found,
                &self.board,
            ),
            _ => Vec::new(),
        };
        let already_open = self.panel.is_some();
        self.panel = Some(PanelState {
            task,
            history,
            draft: None,
            media,
            error: None,
            notice: None,
            hints,
            related,
            risks,
            review_history,
        });
        // The task opens in its own window. One that is already open follows the new task.
        if !already_open {
            let main = cx.entity();
            cx.defer(move |cx| {
                super::dialog_window::open(cx, main, super::dialog_window::Kind::TaskDetail)
            });
        }
        cx.notify();
    }

    /// Closes the task window. It closes itself once the panel is gone, and the board keeps its focus.
    pub(super) fn close_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        cx.notify();
    }

    /// A task that's no longer on the board closes its panel, and the board says why.
    pub(super) fn close_panel_if_gone(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.as_ref() else {
            return;
        };
        if !is_gone(&self.board, panel.task) {
            return;
        }
        let unsaved = panel
            .draft
            .as_ref()
            .map(|draft| draft.text.as_str().to_string());
        self.panel = None;
        let mut notice = "That task was deleted, so its panel was closed.".to_string();
        if let Some(text) = unsaved.filter(|text| !text.trim().is_empty()) {
            eprintln!("unsaved text from a deleted task: {text}");
            notice.push_str(" Text you hadn't saved was written to the log.");
        }
        self.drag.keyboard_notice = Some(notice);
        cx.notify();
    }

    /// Starts editing a text field in the panel, with its current text.
    fn start_draft(&mut self, field: DraftField, cx: &mut Context<Self>) {
        let Some(task) = self.panel_task() else {
            return;
        };
        let text = match field {
            DraftField::Title => TextEdit::new(task.title()),
            DraftField::Description => TextEdit::new(task.description()),
        };
        if let Some(panel) = self.panel.as_mut() {
            panel.draft = Some(Draft { field, text });
            panel.error = None;
        }
        cx.notify();
    }

    pub(super) fn panel_task(&self) -> Option<&Task> {
        let panel = self.panel.as_ref()?;
        self.board.iter().find(|task| task.id() == panel.task)
    }

    fn set_panel_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(panel) = self.panel.as_mut() {
            panel.error = Some(message);
        }
        cx.notify();
    }

    /// Commits the draft: an empty title is refused and the draft stays, so nothing is lost. The
    /// write goes through the same path as every other edit.
    fn commit_draft(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.as_ref() else {
            return;
        };
        let (task, Some(draft)) = (panel.task, panel.draft.clone()) else {
            return;
        };
        let result = match draft.field {
            DraftField::Title => self.edit_task(task, |task| task.rename(&draft.text), cx),
            DraftField::Description => self.edit_task(
                task,
                |task| {
                    task.set_description(draft.text.trim().to_string());
                    Ok(())
                },
                cx,
            ),
        };
        match result {
            Ok(()) => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.draft = None;
                }
            }
            Err(message) => self.set_panel_error(message, cx),
        }
        cx.notify();
    }

    /// Moves the task to another status column, at its end. This is the same move a drag makes.
    pub(super) fn set_panel_status(&mut self, status: Status, cx: &mut Context<Self>) {
        let Some(task) = self.panel_task() else {
            return;
        };
        if task.status() == status {
            return;
        }
        let index = self
            .board
            .iter()
            .filter(|other| other.status() == status)
            .count();
        let drop = DropResult {
            task: task.id(),
            from: task.status(),
            status,
            index,
        };
        self.commit_move(drop, cx);
    }

    /// A key while the panel is open. Returns whether the panel used it.
    pub(super) fn panel_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        if let Some(draft) = panel.draft.as_mut() {
            match key {
                "escape" => panel.draft = None,
                "enter" => return self.commit_draft_consumed(cx),
                _ => {
                    if !draft.text.handle_key(&event.keystroke) {
                        return false;
                    }
                }
            }
            cx.notify();
            return true;
        }
        if key == "escape" {
            self.close_panel(window, cx);
            return true;
        }
        false
    }

    fn commit_draft_consumed(&mut self, cx: &mut Context<Self>) -> bool {
        self.commit_draft(cx);
        true
    }
}

/// The panel, drawn beside the board. Nothing is drawn when no task is open.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let panel = view.panel.as_ref()?;
    let task = view.board.iter().find(|task| task.id() == panel.task)?;
    let draft = panel.draft.as_ref();

    let title_body = match draft.filter(|draft| draft.field == DraftField::Title) {
        Some(draft) => edit_box(&draft.text, "Title", true, theme).into_any_element(),
        None => div()
            .text_size(theme.typography.title)
            .font_weight(FontWeight::MEDIUM)
            .child(task.title().to_string())
            .into_any_element(),
    };
    let description_body = match draft.filter(|draft| draft.field == DraftField::Description) {
        Some(draft) => edit_box(&draft.text, "Description", true, theme).into_any_element(),
        None => div()
            .text_color(theme.palette.text_secondary)
            .child(if task.description().is_empty() {
                "No description. Click to add one.".to_string()
            } else {
                task.description().to_string()
            })
            .into_any_element(),
    };

    let mut statuses = div().flex().flex_row().flex_wrap().gap(px(4.));
    let visible = view
        .projects
        .active()
        .map(|project| project.visible_statuses())
        .unwrap_or_default();
    for status in visible {
        statuses = statuses.child(
            segment(task.status() == status, theme)
                .px(theme.spacing.xs)
                .id(("panel-status", status as u64))
                .child(status_label(status))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_panel_status(status, cx)
                })),
        );
    }

    let priorities = task_editors::priority_picker("panel", task, theme, cx);
    let tag_editor = task_editors::tag_editor("panel", task, view.tag_edit.as_ref(), theme, cx);

    let estimate = task
        .estimate()
        .map(|estimate| format!("{} minutes", estimate.minutes()))
        .unwrap_or_else(|| "No estimate".to_string());
    let start = task
        .start_on()
        .map(|start| start.to_string())
        .unwrap_or_else(|| "No start date".to_string());
    let due = task
        .due_on()
        .map(|due| due.to_string())
        .unwrap_or_else(|| "No due date".to_string());
    let needs_hint = looji_domain::needs_estimate_hint(
        task.status(),
        task.due_on(),
        task.estimate().map(|estimate| estimate.minutes()),
    );
    let mut history = div().flex().flex_col().gap(theme.spacing.xs);
    if panel.history.is_empty() {
        history = history.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("No history yet."),
        );
    }
    for event in &panel.history {
        history = history.child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(format_time(event.at)),
                )
                .child(div().child(describe(event))),
        );
    }

    let panel_column = div()
        .id("task-detail-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w_full()
        .px(theme.spacing.lg)
        .py(theme.spacing.md)
        .child(field(
            "Title",
            panel.error.as_deref(),
            div()
                .id("panel-title")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.start_draft(DraftField::Title, cx)
                    }),
                )
                .child(title_body),
            theme,
        ))
        .child(prop_row("Status", statuses, theme))
        .child(prop_row("Priority", priorities, theme))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .child(media::description_label(panel.task, theme, cx))
                .child(
                    div()
                        .id("panel-description")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                this.start_draft(DraftField::Description, cx)
                            }),
                        )
                        .child(description_body),
                )
                .children(media::tiles(panel.task, &panel.media, false, theme, cx)),
        )
        .child(prop_row(
            "Assignee",
            // The assignee is chosen in the edit dialog, so the row opens it.
            div()
                .id("panel-assignee-edit")
                .cursor_pointer()
                .child(assignee_row(
                    task.assignee_id(),
                    &members::entries(&view.members),
                    theme,
                ))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    if let Some(id) = this.panel.as_ref().map(|panel| panel.task) {
                        this.open_task_dialog(super::task_form::Mode::Edit(id), window, cx);
                    }
                })),
            theme,
        ))
        .child(prop_row("Estimate", div().child(estimate), theme))
        .child(prop_row("Start", div().child(start), theme))
        .child(prop_row("Due", div().child(due), theme))
        .children(needs_hint.then(|| {
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Add an estimate to schedule this task.")
        }))
        .child(field("Tags", None, tag_editor, theme))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(theme.spacing.sm)
                .child(
                    segment(false, theme)
                        .id("panel-edit-details")
                        .child("Edit details")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            if let Some(id) = this.panel.as_ref().map(|panel| panel.task) {
                                this.open_task_dialog(super::task_form::Mode::Edit(id), window, cx);
                            }
                        })),
                )
                .child(
                    segment(false, theme)
                        .id("panel-chat")
                        .child("Ask AI")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_chat(window, cx)
                        })),
                ),
        )
        .when_some(panel.notice.clone(), |column, notice| {
            column.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(notice),
            )
        })
        .children(related_section(view, panel, theme, cx))
        .children(super::review::section(
            view,
            panel.task,
            task.status(),
            theme,
            cx,
        ))
        .child(crate::components::section_label(
            "History, newest first",
            theme,
        ))
        .child(history);

    Some(panel_column.into_any_element())
}

/// Other issues that look like the open one, with the likely duplicates marked. A click opens the other issue.
fn related_section(
    view: &WorkspaceView,
    panel: &PanelState,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let task = view.board.iter().find(|task| task.id() == panel.task)?;
    let estimate_hint = panel
        .hints
        .estimate_minutes
        .filter(|_| task.estimate().is_none());
    let assignee_hint = panel
        .hints
        .assignee
        .as_deref()
        .and_then(|key| uuid::Uuid::parse_str(key).ok())
        .map(TeamMemberId::from_uuid)
        .filter(|_| task.assignee_id().is_none())
        .and_then(|id| view.members.iter().find(|member| member.id() == id));
    if panel.related.is_empty()
        && estimate_hint.is_none()
        && assignee_hint.is_none()
        && panel.risks.is_empty()
    {
        return None;
    }
    let mut column = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(crate::components::section_label("Looks like", theme));
    for (index, issue) in panel.related.iter().take(3).enumerate() {
        let Some(id) = uuid::Uuid::parse_str(&issue.id).ok().map(TaskId::from_uuid) else {
            continue;
        };
        let note = if issue.possible_duplicate {
            "Possible duplicate"
        } else {
            "Related"
        };
        column = column.child(
            div()
                .id(("panel-related", index))
                .flex()
                .flex_row()
                .gap(theme.spacing.sm)
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .text_size(theme.typography.caption)
                        .text_color(if issue.possible_duplicate {
                            theme.palette.danger
                        } else {
                            theme.palette.text_secondary
                        })
                        .child(note),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(format!("{} ({})", issue.title, issue.status)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx))),
        );
    }
    let note = |text: String| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(text)
    };
    for risk in &panel.risks {
        column = column.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(risk.text()),
        );
    }
    if let Some(minutes) = estimate_hint {
        let task_id = panel.task;
        column = column.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(note(format!(
                    "Similar finished tasks took about {} ({} of them).",
                    hours_text(minutes),
                    panel.hints.estimate_from
                )))
                .child(
                    super::task_dialog::button(
                        "hint-estimate",
                        &format!("Use {}", hours_text(minutes)),
                        false,
                        theme,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.apply_estimate_hint(task_id, minutes, cx)
                        },
                    )),
                ),
        );
    }
    if let Some(member) = assignee_hint {
        let (task_id, member_id) = (panel.task, member.id());
        column = column.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(note(format!(
                    "{} has {} similar task(s).",
                    member.name(),
                    panel.hints.assignee_from
                )))
                .child(
                    super::task_dialog::button(
                        "hint-assignee",
                        &format!("Assign {}", member.name()),
                        false,
                        theme,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.apply_assignee_hint(task_id, member_id, cx)
                        },
                    )),
                ),
        );
    }
    Some(column.into_any_element())
}

/// Minutes as the panel writes them: "3h", "1.5h", or "45 min".
fn hours_text(minutes: u32) -> String {
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = f64::from(minutes) / 60.0;
    if minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{hours:.1}h")
    }
}

/// What the similar tasks on the board suggest. A similar task counts once it can be found on the board, and it counts
/// as done in the board's Done column.
fn hints_for(
    related: &[looji_knowledge::related::RelatedIssue],
    board: &[Task],
) -> looji_knowledge::hints::Hints {
    let past: Vec<looji_knowledge::hints::PastTask> = related
        .iter()
        .filter_map(|issue| {
            let id = uuid::Uuid::parse_str(&issue.id).ok()?;
            let task = board.iter().find(|task| task.id().as_uuid() == id)?;
            Some(looji_knowledge::hints::PastTask {
                percent: issue.percent,
                done: task.status() == Status::Done,
                minutes: task.estimate().map(|estimate| estimate.minutes()),
                assignee: task
                    .assignee_id()
                    .map(|member| member.as_uuid().to_string()),
            })
        })
        .collect();
    looji_knowledge::hints::hints(&past)
}

impl WorkspaceView {
    /// Gives the task the estimate similar finished tasks suggest.
    fn apply_estimate_hint(&mut self, task: TaskId, minutes: u32, cx: &mut Context<Self>) {
        let result = self.edit_task(
            task,
            |task| task.set_estimate_minutes(Some(i64::from(minutes))),
            cx,
        );
        if let Err(message) = result {
            self.set_panel_error(message, cx);
        }
        cx.notify();
    }

    /// Assigns the task to the person who has the most similar work.
    fn apply_assignee_hint(&mut self, task: TaskId, member: TeamMemberId, cx: &mut Context<Self>) {
        let result = self.edit_task(
            task,
            |task| {
                task.assign(Some(member));
                Ok(())
            },
            cx,
        );
        if let Err(message) = result {
            self.set_panel_error(message, cx);
        }
        cx.notify();
    }
}

/// Puts an edited task back on the board, in its place. Only that one task changes, so the card for
/// it re-renders without the rest of the board being read again. `false` if it isn't on the board.
pub(super) fn replace_task(board: &mut [Task], updated: Task) -> bool {
    match board.iter_mut().find(|task| task.id() == updated.id()) {
        Some(slot) => {
            *slot = updated;
            true
        }
        None => false,
    }
}

/// Whether a task is no longer on the board, which closes its panel.
pub(super) fn is_gone(board: &[Task], task: TaskId) -> bool {
    !board.iter().any(|other| other.id() == task)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{
        Position, ProjectId, Status, Task, TaskEvent, TaskEventKind, TaskId, Timestamp,
    };

    use super::{describe, format_time, hints_for, hours_text, is_gone, replace_task};

    fn task(title: &str) -> Task {
        Task::new(
            ProjectId::new(),
            title,
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn an_edit_changes_only_its_own_card_in_place() {
        let first = task("First");
        let second = task("Second");
        let mut board = vec![first.clone(), second.clone()];
        let mut edited = first.clone();
        edited.rename("First, edited").unwrap();

        assert!(replace_task(&mut board, edited));
        assert_eq!(board[0].title(), "First, edited");
        assert_eq!(board[0].id(), first.id(), "the card keeps its place");
        assert_eq!(board[1], second, "the other card is untouched");
    }

    #[test]
    fn replacing_a_task_that_is_not_on_the_board_changes_nothing() {
        let mut board = vec![task("Only")];
        assert!(!replace_task(&mut board, task("Stranger")));
        assert_eq!(board.len(), 1);
    }

    #[test]
    fn a_task_missing_from_the_board_is_gone() {
        let kept = task("Kept");
        let board = vec![kept.clone()];
        assert!(!is_gone(&board, kept.id()));
        assert!(is_gone(&board, TaskId::new()));
    }

    #[test]
    fn history_entries_say_what_happened() {
        let at = Timestamp::from_unix_millis(0);
        let edited = TaskEvent {
            at,
            kind: TaskEventKind::Edited {
                fields: vec!["title".to_string(), "tags".to_string()],
            },
        };
        assert_eq!(describe(&edited), "Edited title, tags");
        let created = TaskEvent {
            at,
            kind: TaskEventKind::Created,
        };
        assert_eq!(describe(&created), "Created");
    }

    #[test]
    fn timestamps_show_in_utc() {
        assert_eq!(
            format_time(Timestamp::from_unix_millis(0)),
            "1970-01-01 00:00 UTC"
        );
        // 2026-10-14 00:23:00 UTC.
        let millis = 1_791_937_380_000;
        assert_eq!(
            format_time(Timestamp::from_unix_millis(millis)),
            "2026-10-14 00:23 UTC"
        );
    }

    #[test]
    fn estimates_read_as_hours_or_minutes() {
        assert_eq!(hours_text(45), "45 min");
        assert_eq!(hours_text(180), "3h");
        assert_eq!(hours_text(90), "1.5h");
    }

    #[test]
    fn similar_finished_tasks_on_the_board_give_an_estimate_hint() {
        use looji_knowledge::related::RelatedIssue;
        let mut first = task("Fix login timeout");
        first.set_status(Status::Done);
        first.set_estimate_minutes(Some(120)).unwrap();
        let mut second = task("Fix login timeout again");
        second.set_status(Status::Done);
        second.set_estimate_minutes(Some(180)).unwrap();
        let related: Vec<RelatedIssue> = [&first, &second]
            .iter()
            .map(|found| RelatedIssue {
                id: found.id().as_uuid().to_string(),
                title: found.title().to_string(),
                status: "Done".to_string(),
                percent: 50,
                possible_duplicate: false,
            })
            .collect();
        let hints = hints_for(&related, &[first, second]);
        assert_eq!(hints.estimate_minutes, Some(150));
        assert_eq!(hints.estimate_from, 2);
    }
}
