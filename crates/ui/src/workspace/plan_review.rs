//! The backlog review: the spec, the user stories, and the tasks of a ready plan, each one editable. Every change is
//! applied through the store as soon as it's made, and the screen shows the draft the store returns, so what's on
//! screen is what was saved.
//!
//! One text edit is open at a time. Opening another one saves the open one first, and if that fails, the new one
//! doesn't open. Removing or moving a task never asks first, since a removed task can be added back.

use gpui::{prelude::*, *};
use looji_domain::{
    PlanDraft, PlanDraftRepository, PlanEdit, Priority, Publication, RefineTarget, RepoError,
    Status, TeamMember, TeamMemberId, Timestamp, display_labels, parse_estimate_hours,
};

use super::plan_refine;
use super::plan_wizard::PlanWizardDialog;
use super::task_dialog::button;
use super::{WorkspaceView, store};
use crate::{components::edit_box, text_edit::TextEdit, theme::Theme};

/// What an open edit changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    Spec,
    StoryTitle(usize),
    StoryBody(usize),
    TaskTitle(usize),
    TaskEstimate(usize),
    TaskTags(usize),
}

impl Target {
    /// Spec and story bodies span lines, so Enter adds a line rather than saving.
    fn multiline(&self) -> bool {
        matches!(self, Target::Spec | Target::StoryBody(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Editor {
    pub target: Target,
    pub text: TextEdit,
    /// Why the text was refused, when it was.
    pub error: Option<String>,
}

/// The text an edit starts with, read from the draft.
fn current_text(draft: &PlanDraft, target: &Target) -> String {
    match target {
        Target::Spec => draft.spec().to_string(),
        Target::StoryTitle(index) => draft
            .stories()
            .get(*index)
            .map(|story| story.title.clone())
            .unwrap_or_default(),
        Target::StoryBody(index) => draft
            .stories()
            .get(*index)
            .map(|story| story.body.clone())
            .unwrap_or_default(),
        Target::TaskTitle(index) => draft
            .tasks()
            .get(*index)
            .map(|task| task.title.clone())
            .unwrap_or_default(),
        Target::TaskEstimate(index) => draft
            .tasks()
            .get(*index)
            .and_then(|task| task.estimate)
            .map(|estimate| hours_text(estimate.minutes()))
            .unwrap_or_default(),
        Target::TaskTags(index) => draft
            .tasks()
            .get(*index)
            .map(|task| task.tags.join(", "))
            .unwrap_or_default(),
    }
}

/// Minutes as the hours a person would type. Two decimals at most, with the zeros trimmed.
fn hours_text(minutes: u32) -> String {
    let hours = format!("{:.2}", f64::from(minutes) / 60.0);
    hours
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// The edit an open editor stands for, or the reason its text can't be used.
fn edit_for(editor: &Editor) -> Result<PlanEdit, String> {
    let text = editor.text.as_str().to_string();
    Ok(match editor.target {
        Target::Spec => PlanEdit::SetSpec(text),
        Target::StoryTitle(index) => PlanEdit::SetStoryTitle { index, title: text },
        Target::StoryBody(index) => PlanEdit::SetStoryBody { index, body: text },
        Target::TaskTitle(index) => PlanEdit::SetTaskTitle { index, title: text },
        Target::TaskEstimate(index) => PlanEdit::SetTaskEstimate {
            index,
            estimate: parse_estimate_hours(&text).map_err(|err| err.to_string())?,
        },
        Target::TaskTags(index) => PlanEdit::SetTaskTags {
            index,
            tags: text
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect(),
        },
    })
}

fn next_priority(priority: Priority) -> Priority {
    match priority {
        Priority::Low => Priority::Medium,
        Priority::Medium => Priority::High,
        Priority::High => Priority::Low,
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

impl WorkspaceView {
    /// Starts an edit on one piece of the plan. An edit already open is saved first. If that fails, it stays
    /// open with its error, and the new one doesn't start.
    pub(super) fn begin_plan_edit(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The text box is drawn in this dialog, so the keyboard goes to the workspace rather than to a control
        // that may no longer be on screen.
        self.focus_handle.focus(window);
        if self.reject_if_published(cx) {
            return;
        }
        if self
            .plan_wizard
            .as_ref()
            .is_some_and(PlanWizardDialog::refining)
        {
            return;
        }
        let has_open = self
            .plan_wizard
            .as_ref()
            .is_some_and(|dialog| dialog.editor.is_some());
        if has_open && !self.commit_plan_edit(cx) {
            return;
        }
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(draft) = dialog.draft.as_ref() else {
            return;
        };
        let text = current_text(draft, &target);
        dialog.editor = Some(Editor {
            target,
            text: TextEdit::new(text),
            error: None,
        });
        cx.notify();
    }

    /// Saves the open edit. Returns whether it was saved, or whether there was nothing to save. A refused edit keeps
    /// the editor open with the reason.
    pub(super) fn commit_plan_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self
            .plan_wizard
            .as_ref()
            .and_then(|dialog| dialog.editor.clone())
        else {
            return true;
        };
        let outcome = edit_for(&editor).and_then(|edit| self.apply_plan_edit(edit, cx));
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return false;
        };
        match outcome {
            Ok(()) => {
                dialog.editor = None;
                cx.notify();
                true
            }
            Err(message) => {
                if let Some(open) = dialog.editor.as_mut() {
                    open.error = Some(message);
                }
                cx.notify();
                false
            }
        }
    }

    pub(super) fn cancel_plan_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_wizard.as_mut() {
            dialog.editor = None;
        }
        cx.notify();
    }

    /// Applies one change and stores the draft it returns. A change the domain refuses is returned with its reason,
    /// and nothing is written.
    fn apply_plan_edit(&mut self, edit: PlanEdit, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return Err("The plan is no longer open.".to_string());
        };
        let Some(id) = dialog.draft.as_ref().map(PlanDraft::id) else {
            return Err("There is no plan to change.".to_string());
        };
        match PlanDraftRepository::apply_edit(store(cx), id, edit, now()) {
            Ok(Some(updated)) => {
                dialog.draft = Some(updated);
                dialog.notice = None;
                cx.notify();
                Ok(())
            }
            Ok(None) => Err("This plan is no longer stored.".to_string()),
            Err(RepoError::Rejected(err)) => Err(err.to_string()),
            Err(err) => Err(format!("Could not save that change: {err}")),
        }
    }

    /// A change made by a button. There's no text to keep, so a refusal goes to the notice at the foot.
    fn plan_change(&mut self, edit: PlanEdit, cx: &mut Context<Self>) {
        if self.reject_if_published(cx) {
            return;
        }
        // A refinement is replacing a part, so nothing else may change the plan until it's done.
        if self
            .plan_wizard
            .as_ref()
            .is_some_and(PlanWizardDialog::refining)
        {
            return;
        }
        if let Err(message) = self.apply_plan_edit(edit, cx)
            && let Some(dialog) = self.plan_wizard.as_mut()
        {
            dialog.notice = Some(message);
            cx.notify();
        }
    }

    fn cycle_plan_priority(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(priority) = self
            .plan_wizard
            .as_ref()
            .and_then(|dialog| dialog.draft.as_ref())
            .and_then(|draft| draft.tasks().get(index))
            .map(|task| task.priority)
        else {
            return;
        };
        self.plan_change(
            PlanEdit::SetTaskPriority {
                index,
                priority: next_priority(priority),
            },
            cx,
        );
    }

    /// Moves through no one, then each team member in order, then back to no one.
    fn cycle_plan_assignee(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_ref() else {
            return;
        };
        let Some(current) = dialog
            .draft
            .as_ref()
            .and_then(|draft| draft.tasks().get(index))
            .map(|task| task.assignee)
        else {
            return;
        };
        let mut options: Vec<Option<TeamMemberId>> = vec![None];
        options.extend(dialog.members.iter().map(|member| Some(member.id())));
        let position = options
            .iter()
            .position(|option| *option == current)
            .unwrap_or(0);
        let next = options[(position + 1) % options.len()];
        self.plan_change(
            PlanEdit::SetTaskAssignee {
                index,
                assignee: next,
            },
            cx,
        );
    }

    /// Adds a task with a title no other task has, so the new task never clashes with an existing one.
    fn add_plan_task(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self
            .plan_wizard
            .as_ref()
            .and_then(|dialog| dialog.draft.as_ref())
        else {
            return;
        };
        let taken: Vec<String> = draft
            .tasks()
            .iter()
            .map(|task| task.title.to_lowercase())
            .collect();
        let title = (1..)
            .map(|number| match number {
                1 => "New task".to_string(),
                _ => format!("New task {number}"),
            })
            .find(|candidate| !taken.contains(&candidate.to_lowercase()))
            .unwrap_or_else(|| "New task".to_string());
        self.plan_change(PlanEdit::AddTask { title }, cx);
    }

    /// Whether the plan has been published. A published plan is in the backlog, so it can't change here. Says so when
    /// something tries.
    fn reject_if_published(&mut self, cx: &mut Context<Self>) -> bool {
        let published = self
            .plan_wizard
            .as_ref()
            .and_then(|dialog| dialog.draft.as_ref())
            .is_some_and(|draft| draft.published_at().is_some());
        if published && let Some(dialog) = self.plan_wizard.as_mut() {
            dialog.notice = Some("This plan is in the backlog, so it can't change.".to_string());
            cx.notify();
        }
        published
    }

    /// Publishes the plan's tasks to the Backlog. An edit still open is saved or cancelled first, so what's published
    /// is what's on screen. The store does the whole publication in one transaction. A failure adds nothing, and the
    /// plan stays as it was.
    pub(super) fn publish_plan(&mut self, status: Status, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.editor.is_some() {
            dialog.notice = Some("Save or cancel the open edit before publishing.".to_string());
            return cx.notify();
        }
        let Some(id) = dialog.draft.as_ref().map(PlanDraft::id) else {
            return;
        };
        let outcome = PlanDraftRepository::publish(store(cx), id, now(), status);
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let mut published = false;
        match outcome {
            Ok(Some(publication)) => {
                if let Ok(Some(updated)) = PlanDraftRepository::find(store(cx), id) {
                    dialog.draft = Some(updated);
                }
                dialog.notice = Some(publish_message(&publication, status));
                published = true;
            }
            Ok(None) => dialog.notice = Some("This plan is no longer stored.".to_string()),
            Err(RepoError::Rejected(err)) => dialog.notice = Some(err.to_string()),
            Err(err) => {
                dialog.notice = Some(format!(
                    "Could not publish the plan, so nothing was added: {err}"
                ))
            }
        }
        // The published tasks are on the board now. The board only reloads when its project changes, so reload it
        // here, or the Backlog would show nothing until the user switched away and back.
        if published {
            if status == Status::Backlog
                && self
                    .projects
                    .active()
                    .is_some_and(|project| !project.is_status_visible(Status::Backlog))
            {
                self.request_column_visibility(Status::Backlog, true, cx);
            }
            self.reload_board(cx);
        }
        cx.notify();
    }

    pub(super) fn handle_review_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .plan_wizard
            .as_ref()
            .is_some_and(|dialog| dialog.refine.is_some())
        {
            return plan_refine::refine_key(self, event, cx);
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let Some(editor) = self
            .plan_wizard
            .as_ref()
            .and_then(|dialog| dialog.editor.clone())
        else {
            if key == "escape" {
                self.request_plan_close(window, cx);
            }
            return;
        };
        match key {
            "escape" => self.cancel_plan_edit(cx),
            "enter" if editor.target.multiline() && !modifiers.control => {
                self.append_to_plan_editor("\n", cx)
            }
            "enter" => {
                self.commit_plan_edit(cx);
            }
            _ => {
                if let Some(open) = self.plan_wizard.as_mut().and_then(|d| d.editor.as_mut())
                    && open.text.handle_key(&event.keystroke)
                {
                    open.error = None;
                    cx.notify();
                }
            }
        }
    }

    fn append_to_plan_editor(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(open) = self.plan_wizard.as_mut().and_then(|d| d.editor.as_mut()) {
            open.text.insert(text);
            open.error = None;
        }
        cx.notify();
    }
}

/// The text of a value that can be clicked to edit. While its edit is open, the editor shows in its place.
fn editable(
    view: &PlanWizardDialog,
    target: Target,
    id: (&'static str, usize),
    shown: String,
    placeholder: &str,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let open = view
        .editor
        .as_ref()
        .filter(|editor| editor.target == target);
    if let Some(editor) = open {
        let body = if target.multiline() {
            let mut lines = div().flex().flex_col();
            for line in editor.text.split('\n') {
                lines = lines.child(div().min_h(px(18.)).child(line.to_string()));
            }
            lines.child("|").into_any_element()
        } else {
            edit_box(&editor.text, placeholder, true, theme).into_any_element()
        };
        return div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(body)
            .when_some(editor.error.clone(), |column, message| {
                column.child(
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
                    .child(
                        button(("review-cancel", id.1), "Cancel", false, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_plan_edit(cx)),
                        ),
                    )
                    .child(button(("review-save", id.1), "Save", true, theme).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.commit_plan_edit(cx);
                        }),
                    )),
            )
            .into_any_element();
    }
    let display = if shown.is_empty() {
        div()
            .text_color(theme.palette.text_secondary)
            .child(placeholder.to_string())
            .into_any_element()
    } else if target.multiline() {
        super::markdown_view::view(&shown, theme).into_any_element()
    } else {
        div().child(shown).into_any_element()
    };
    div()
        .id(id)
        .cursor_pointer()
        .rounded(theme.radius.sm)
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(display)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.begin_plan_edit(target.clone(), window, cx)
        }))
        .into_any_element()
}

/// What a publication says in the notice: how many tasks went in, and which titles were already there.
fn publish_message(publication: &Publication, status: Status) -> String {
    let count = publication.published.len();
    let noun = if count == 1 { "task" } else { "tasks" };
    let place = if status == Status::Todo {
        "To do"
    } else {
        "the backlog"
    };
    let mut message = format!("Published {count} {noun} to {place}.");
    if !publication.skipped.is_empty() {
        message.push_str(&format!(
            " Skipped {} the backlog already has: {}.",
            publication.skipped.len(),
            publication.skipped.join(", ")
        ));
    }
    message
}

/// The publish control. Before publishing it says what will happen. Afterwards it says the plan is in the backlog.
fn publish_row(dialog: &PlanWizardDialog, theme: &Theme, cx: &mut Context<WorkspaceView>) -> Div {
    // Stacked, so the note and the button both fit the side column.
    let row = div().flex().flex_col().items_start().gap(theme.spacing.sm);
    let Some(draft) = dialog.draft.as_ref() else {
        return row;
    };
    if draft.published_at().is_some() {
        return row.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("This plan is in the backlog. It can't change now."),
        );
    }
    let count = draft.tasks().len();
    let noun = if count == 1 { "task" } else { "tasks" };
    row.child(
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child("A task whose title the backlog already has is skipped."),
    )
    .child({
        // An empty plan has nothing to publish, and publishing it would still mark the plan as published.
        let mut actions = div().flex().flex_row().flex_wrap().gap(theme.spacing.sm);
        for (id, status, place, primary) in [
            (
                "review-publish-backlog",
                Status::Backlog,
                "the backlog",
                true,
            ),
            ("review-publish-todo", Status::Todo, "To do", false),
        ] {
            let publish = button(
                id,
                &format!("Publish {count} {noun} to {place}"),
                primary,
                theme,
            );
            let publish = if count == 0 {
                publish.opacity(0.5).cursor_default()
            } else {
                publish.on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.publish_plan(status, cx)),
                )
            };
            actions = actions.child(publish);
        }
        actions
    })
}

/// The mark on a part a person wrote or changed by hand. Refining that part asks first.
fn edited_mark(theme: &Theme) -> Div {
    div()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child("Edited by hand")
}

/// The button that asks for a refinement of one part.
fn refine_button(
    id: (&'static str, usize),
    target: RefineTarget,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    button(id, "Refine", false, theme).on_click(
        cx.listener(move |this, _: &ClickEvent, window, cx| this.open_refine(target, window, cx)),
    )
}

/// The review, drawn over the window in place of the wizard's form. Nothing is drawn when the wizard isn't open.
/// Stories shown on one page of the review.
const STORIES_PER_PAGE: usize = 3;
/// Tasks shown on one page of the review. Tasks carry more detail than stories, so fewer fit.
const TASKS_PER_PAGE: usize = 4;

/// The pages a list of `count` items fills. An empty list still has one page, so the pager always reads correctly.
fn page_count(count: usize, per_page: usize) -> usize {
    count.div_ceil(per_page).max(1)
}

/// The items on a page, as their indexes in the whole list.
fn page_range(page: usize, count: usize, per_page: usize) -> std::ops::Range<usize> {
    let start = (page * per_page).min(count);
    start..(start + per_page).min(count)
}

impl WorkspaceView {
    /// Moves a review list to its next or previous page, within its last page at the end.
    pub(super) fn step_review_page(&mut self, tasks: bool, delta: isize, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let (count, per_page) = match dialog.draft.as_ref() {
            Some(draft) if tasks => (draft.tasks().len(), TASKS_PER_PAGE),
            Some(draft) => (draft.stories().len(), STORIES_PER_PAGE),
            None => (0, 1),
        };
        let pages = page_count(count, per_page) as isize;
        let page = if tasks {
            &mut dialog.task_page
        } else {
            &mut dialog.story_page
        };
        *page = (*page as isize + delta).clamp(0, pages - 1) as usize;
        cx.notify();
    }
}

/// The pager under a list: previous, the page and its count, and next.
fn pager(
    tasks: bool,
    page: usize,
    pages: usize,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let page = page.min(pages - 1);
    let key = usize::from(tasks);
    let previous =
        button(("review-page-previous", key), "‹ Previous", false, theme)
            .when(page == 0, |b| b.opacity(0.5).cursor_default())
            .when(page > 0, |b| {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.step_review_page(tasks, -1, cx)
                }))
            });
    let next = button(("review-page-next", key), "Next ›", false, theme)
        .when(page + 1 >= pages, |b| b.opacity(0.5).cursor_default())
        .when(page + 1 < pages, |b| {
            b.on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.step_review_page(tasks, 1, cx)),
            )
        });
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .pt(theme.spacing.sm)
        .border_t_1()
        .border_color(theme.palette.border)
        .child(previous)
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("Page {} of {}", page + 1, pages)),
        )
        .child(next)
        .into_any_element()
}

/// A card in the review's grid: a heading, the scrolling content, and whatever sits under it.
fn review_card(
    id: &'static str,
    heading: AnyElement,
    content: impl IntoElement,
    footer: AnyElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .p(theme.spacing.md)
        .gap(theme.spacing.sm)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface_raised)
        .child(heading)
        .child(
            div()
                .id(id)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(theme.spacing.sm)
                .overflow_y_scroll()
                .child(content),
        )
        .child(footer)
}

/// A card's heading: its name, the count, and any action on the right.
fn card_heading(name: String, action: Option<AnyElement>, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(theme.typography.body)
                .child(name),
        )
        .children(action)
        .into_any_element()
}

pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.plan_wizard.as_ref()?;
    let draft = dialog.draft.as_ref()?;
    let labels = display_labels(&dialog.members);
    let assignee_label = |member: Option<TeamMemberId>| match member {
        None => "Unassigned".to_string(),
        Some(id) => dialog
            .members
            .iter()
            .position(|candidate: &TeamMember| candidate.id() == id)
            .and_then(|index| labels.get(index).cloned())
            .unwrap_or_else(|| "Unassigned".to_string()),
    };
    let project = view
        .projects
        .active()
        .map(|project| project.name().to_string())
        .unwrap_or_default();

    // The summary: the counts, the total estimate, and how many tasks are high priority.
    let total_minutes: u32 = draft
        .tasks()
        .iter()
        .filter_map(|task| task.estimate.map(|estimate| estimate.minutes()))
        .sum();
    let high = draft
        .tasks()
        .iter()
        .filter(|task| task.priority == Priority::High)
        .count();
    let chip = |text: String| {
        div()
            .px(theme.spacing.sm)
            .py(px(3.))
            .rounded(px(12.))
            .bg(theme.palette.surface_hover)
            .text_size(theme.typography.caption)
            .child(text)
    };
    let summary = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(theme.spacing.xs)
        .child(chip(format!(
            "{} {}",
            draft.stories().len(),
            if draft.stories().len() == 1 {
                "story"
            } else {
                "stories"
            }
        )))
        .child(chip(format!(
            "{} {}",
            draft.tasks().len(),
            if draft.tasks().len() == 1 {
                "task"
            } else {
                "tasks"
            }
        )))
        .child(chip(format!("{} h planned", hours_text(total_minutes))))
        .child(chip(format!("{high} high priority")));

    let header = div()
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(theme.spacing.md)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .min_w_0()
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(format!("{project} / Drafts")),
                )
                .child(
                    div()
                        .text_size(theme.typography.heading)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Review the plan"),
                )
                .child(
                    div()
                        .text_color(theme.palette.text_secondary)
                        .child("Each change is saved as you make it. Nothing reaches the board until you publish."),
                ),
        )
        .child(summary);

    // The spec, on the left, scrolls on its own so a long one doesn't push the lists off the page.
    let spec = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .p(theme.spacing.md)
        .gap(theme.spacing.sm)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface_raised)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(card_heading("Spec".to_string(), None, theme))
                .children(draft.spec_edited().then(|| edited_mark(theme)))
                .child(refine_button(
                    ("review-refine-spec", 0),
                    RefineTarget::Spec,
                    theme,
                    cx,
                )),
        )
        .children(plan_refine::form(dialog, RefineTarget::Spec, theme, cx))
        .child(
            div()
                .id("review-spec-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(editable(
                    dialog,
                    Target::Spec,
                    ("review-spec", 0),
                    draft.spec().to_string(),
                    "No spec",
                    theme,
                    cx,
                )),
        );

    // The stories: one page at a time.
    let story_count = draft.stories().len();
    let story_pages = page_count(story_count, STORIES_PER_PAGE);
    let story_page = dialog.story_page.min(story_pages - 1);
    let mut story_list = div().flex().flex_col().gap(theme.spacing.sm);
    if draft.stories().is_empty() {
        story_list = story_list.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("No user stories."),
        );
    }
    for index in page_range(story_page, story_count, STORIES_PER_PAGE) {
        let story = &draft.stories()[index];
        story_list = story_list.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .p(theme.spacing.sm)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(theme.typography.caption)
                                .text_color(theme.palette.text_secondary)
                                .child(format!("Story {}", index + 1)),
                        )
                        .children(story.edited.then(|| edited_mark(theme))),
                )
                .child(editable(
                    dialog,
                    Target::StoryTitle(index),
                    ("review-story-title", index),
                    story.title.clone(),
                    "Untitled story",
                    theme,
                    cx,
                ))
                .child(editable(
                    dialog,
                    Target::StoryBody(index),
                    ("review-story-body", index),
                    story.body.clone(),
                    "No description",
                    theme,
                    cx,
                ))
                .children(plan_refine::form(
                    dialog,
                    RefineTarget::Story(index),
                    theme,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(refine_button(
                            ("review-refine-story", index),
                            RefineTarget::Story(index),
                            theme,
                            cx,
                        ))
                        .child(
                            button(("review-remove-story", index), "Remove story", false, theme)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.plan_change(PlanEdit::RemoveStory(index), cx)
                                })),
                        ),
                ),
        );
    }
    let stories_card = review_card(
        "review-stories",
        card_heading(format!("User stories ({story_count})"), None, theme),
        story_list,
        pager(false, story_page, story_pages, theme, cx),
        theme,
    );

    // The tasks: one page at a time, each with its detail and its actions.
    let task_count = draft.tasks().len();
    let task_pages = page_count(task_count, TASKS_PER_PAGE);
    let task_page = dialog.task_page.min(task_pages - 1);
    let last = task_count.saturating_sub(1);
    let mut task_list = div().flex().flex_col().gap(theme.spacing.sm);
    if draft.tasks().is_empty() {
        task_list =
            task_list.child(div().text_color(theme.palette.text_secondary).child(
                "The backlog is empty. Add a task, or close this and generate a plan again.",
            ));
    }
    for index in page_range(task_page, task_count, TASKS_PER_PAGE) {
        let task = &draft.tasks()[index];
        let estimate = task
            .estimate
            .map(|estimate| format!("{} h", hours_text(estimate.minutes())))
            .unwrap_or_else(|| "No estimate".to_string());
        let tags = if task.tags.is_empty() {
            "No tags".to_string()
        } else {
            task.tags.join(", ")
        };
        task_list = task_list.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .p(theme.spacing.sm)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(theme.typography.caption)
                                .text_color(theme.palette.text_secondary)
                                .child(format!("Task {}", index + 1)),
                        )
                        .children(task.edited.then(|| edited_mark(theme))),
                )
                .child(editable(
                    dialog,
                    Target::TaskTitle(index),
                    ("review-task-title", index),
                    task.title.clone(),
                    "Untitled task",
                    theme,
                    cx,
                ))
                .children(plan_refine::form(
                    dialog,
                    RefineTarget::Task(index),
                    theme,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(theme.spacing.xs)
                        .items_center()
                        .child(
                            button(
                                ("review-priority", index),
                                &format!("Priority: {}", task.priority.as_str()),
                                false,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.cycle_plan_priority(index, cx)
                                },
                            )),
                        )
                        .child(
                            button(
                                ("review-assignee", index),
                                &format!("Assignee: {}", assignee_label(task.assignee)),
                                false,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.cycle_plan_assignee(index, cx)
                                },
                            )),
                        )
                        .child(editable(
                            dialog,
                            Target::TaskEstimate(index),
                            ("review-estimate", index),
                            format!("Estimate: {estimate}"),
                            "Estimate in hours",
                            theme,
                            cx,
                        ))
                        .child(editable(
                            dialog,
                            Target::TaskTags(index),
                            ("review-tags", index),
                            format!("Tags: {tags}"),
                            "Tags, separated by commas",
                            theme,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(refine_button(
                            ("review-refine-task", index),
                            RefineTarget::Task(index),
                            theme,
                            cx,
                        ))
                        .child(
                            button(("review-up", index), "Up", false, theme)
                                .when(index == 0, |b| b.opacity(0.5).cursor_default())
                                .when(index > 0, |b| {
                                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.plan_change(
                                            PlanEdit::MoveTask {
                                                from: index,
                                                to: index - 1,
                                            },
                                            cx,
                                        )
                                    }))
                                }),
                        )
                        .child(
                            button(("review-down", index), "Down", false, theme)
                                .when(index == last, |b| b.opacity(0.5).cursor_default())
                                .when(index < last, |b| {
                                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.plan_change(
                                            PlanEdit::MoveTask {
                                                from: index,
                                                to: index + 1,
                                            },
                                            cx,
                                        )
                                    }))
                                }),
                        )
                        .child(
                            button(("review-remove-task", index), "Remove task", false, theme)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.plan_change(PlanEdit::RemoveTask(index), cx)
                                })),
                        ),
                ),
        );
    }
    let add_task = button("review-add-task", "Add task", true, theme)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_plan_task(cx)))
        .into_any_element();
    let tasks_card = review_card(
        "review-tasks",
        card_heading(format!("Tasks ({task_count})"), Some(add_task), theme),
        task_list,
        pager(true, task_page, task_pages, theme, cx),
        theme,
    );

    let warnings = dialog.plan.as_ref().and_then(|plan| {
        (!plan.warnings.is_empty()).then(|| {
            format!(
                "{} assignee(s) didn't match the team and were left unassigned.",
                plan.warnings.len()
            )
        })
    });

    // The grid: the spec on the left; the stories above the tasks on the right.
    let lists = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .gap(theme.spacing.lg)
        .child(stories_card)
        .child(tasks_card);
    let grid = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h_0()
        .gap(theme.spacing.lg)
        .child(spec)
        .child(lists);

    let panel = div()
        .id("plan-review-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .size_full()
        .p(theme.spacing.lg)
        .bg(theme.palette.surface)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(header)
        .when_some(warnings, |panel, warning| {
            panel.child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child(warning),
            )
        })
        .when_some(dialog.notice.clone(), |panel, notice| {
            // A notice that reports a success is plain text. Anything else is a problem, so it's shown in red.
            let colour = if notice.starts_with("Published") {
                theme.palette.text_primary
            } else {
                theme.palette.danger
            };
            panel.child(div().text_color(colour).child(notice))
        })
        .child(grid)
        .child(publish_row(dialog, theme, cx));

    Some(panel.into_any_element())
}
