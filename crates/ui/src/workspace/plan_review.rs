//! The backlog review: the spec, the user stories, and the tasks of a ready plan, each one editable. Every change is
//! applied through the store as soon as it's made, and the screen shows the draft the store returns, so what's on
//! screen is what was saved.
//!
//! One text edit is open at a time. Opening another one saves the open one first, and if that fails, the new one
//! doesn't open. Removing or moving a task never asks first, since a removed task can be added back.

use gpui::{prelude::*, *};
use looji_domain::{
    PlanDraft, PlanDraftRepository, PlanEdit, Priority, RefineTarget, RepoError, TeamMember,
    TeamMemberId, Timestamp, display_labels, parse_estimate_hours,
};

use super::fields::text_box;
use super::plan_refine;
use super::plan_wizard::PlanWizardDialog;
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

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
    pub text: String,
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
    let text = editor.text.clone();
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
    pub(super) fn begin_plan_edit(&mut self, target: Target, cx: &mut Context<Self>) {
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
            text,
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
            "backspace" => {
                if let Some(open) = self.plan_wizard.as_mut().and_then(|d| d.editor.as_mut()) {
                    open.text.pop();
                    open.error = None;
                }
                cx.notify();
            }
            _ => {
                let typed = if modifiers.control || modifiers.alt || modifiers.platform {
                    None
                } else {
                    typed_text(&event.keystroke)
                };
                if let Some(text) = typed {
                    self.append_to_plan_editor(text, cx);
                }
            }
        }
    }

    fn append_to_plan_editor(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(open) = self.plan_wizard.as_mut().and_then(|d| d.editor.as_mut()) {
            open.text.push_str(text);
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
            text_box(&editor.text, placeholder, true, theme).into_any_element()
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
    } else {
        div().child(shown)
    };
    div()
        .id(id)
        .cursor_pointer()
        .rounded(theme.radius.sm)
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(display)
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.begin_plan_edit(target.clone(), cx)
            }),
        )
        .into_any_element()
}

fn section_heading(text: String, theme: &Theme) -> Div {
    div()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(text)
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
    button(id, "Refine", false, theme)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_refine(target, cx)))
}

/// The review, drawn over the window in place of the wizard's form. Nothing is drawn when the wizard isn't open.
pub(super) fn overlay(
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

    let spec = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(section_heading("Spec".to_string(), theme))
                .children(draft.spec_edited().then(|| edited_mark(theme)))
                .child(refine_button(
                    ("review-refine-spec", 0),
                    RefineTarget::Spec,
                    theme,
                    cx,
                )),
        )
        .children(plan_refine::form(dialog, RefineTarget::Spec, theme, cx))
        .child(editable(
            dialog,
            Target::Spec,
            ("review-spec", 0),
            draft.spec().to_string(),
            "No spec",
            theme,
            cx,
        ));

    let mut stories = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .child(section_heading(
            format!("User stories ({})", draft.stories().len()),
            theme,
        ));
    if draft.stories().is_empty() {
        stories = stories.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("No user stories."),
        );
    }
    for (index, story) in draft.stories().iter().enumerate() {
        stories = stories.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .p(theme.spacing.sm)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .children(story.edited.then(|| edited_mark(theme)))
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

    let mut backlog = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .child(section_heading(
            format!("Backlog ({} tasks)", draft.tasks().len()),
            theme,
        ));
    if draft.tasks().is_empty() {
        backlog =
            backlog.child(div().text_color(theme.palette.text_secondary).child(
                "The backlog is empty. Add a task, or close this and generate a plan again.",
            ));
    }
    let last = draft.tasks().len().saturating_sub(1);
    for (index, task) in draft.tasks().iter().enumerate() {
        let estimate = task
            .estimate
            .map(|estimate| format!("{} h", hours_text(estimate.minutes())))
            .unwrap_or_else(|| "No estimate".to_string());
        let tags = if task.tags.is_empty() {
            "No tags".to_string()
        } else {
            task.tags.join(", ")
        };
        backlog = backlog.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .p(theme.spacing.sm)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .children(task.edited.then(|| edited_mark(theme)))
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
    backlog = backlog.child(
        div().flex().flex_row().justify_end().child(
            button("review-add-task", "Add task", true, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_plan_task(cx))),
        ),
    );

    let warnings = dialog.plan.as_ref().and_then(|plan| {
        (!plan.warnings.is_empty()).then(|| {
            format!(
                "{} assignee(s) didn't match the team and were left unassigned.",
                plan.warnings.len()
            )
        })
    });

    let panel = div()
        .id("plan-review-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w(px(640.))
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
                .child("Review the plan"),
        )
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Each change is saved as you make it."),
        )
        .when_some(warnings, |panel, warning| {
            panel.child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child(warning),
            )
        })
        .when_some(dialog.notice.clone(), |panel, notice| {
            panel.child(div().text_color(theme.palette.danger).child(notice))
        })
        .child(
            div()
                .id("plan-review-scroll")
                .flex()
                .flex_col()
                .gap(theme.spacing.lg)
                .max_h(px(520.))
                .overflow_y_scroll()
                .child(spec)
                .child(stories)
                .child(backlog),
        )
        .child(div().flex().flex_row().justify_end().child(
            button("plan-close", "Close", false, theme).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.request_plan_close(window, cx)),
            ),
        ));

    Some(
        div()
            .id("plan-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.palette.scrim)
            .occlude()
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.request_plan_close(window, cx)),
            )
            .child(panel)
            .into_any_element(),
    )
}
