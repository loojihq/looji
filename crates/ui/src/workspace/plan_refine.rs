//! Refining one part of a ready plan: the spec, a user story, or a task. A follow-up instruction asks the model for a
//! replacement for that part only. The reply is read as that part alone, and it replaces that part alone. Every other
//! part of the plan is left as it was.
//!
//! A part a person edited by hand isn't replaced until the user has said so. The question is asked before the
//! request is sent, and a refusal from the store (the same check, once more) asks again. Nothing is replaced while a
//! refinement is running, since its target would otherwise change under it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{prelude::*, *};
use looji_ai::{
    CancellationToken, ChatMessage, ChatRequest,
    generation::{Update, run_request},
    models::DEFAULT_MODEL,
    planning::{RefineKind, Refinement, build_refinement_prompt, parse_refinement},
};
use looji_domain::{
    DomainError, PlanDraft, PlanDraftError, PlanDraftRepository, PlanStory, PlanTask, RefineTarget,
    RefinedPart, RepoError, TeamMember, Timestamp, display_labels,
};

use super::fields::text_box;
use super::plan_wizard::{PlanWizardDialog, SPINNER};
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

const TICK: Duration = Duration::from_millis(100);
const MAX_TOKENS: u32 = 4096;

/// Where a refinement is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// The instruction is being typed.
    Writing,
    /// The part was edited by hand, so the user has to say whether to replace it.
    Confirming,
    /// The request is in flight.
    Running,
}

/// One refinement in flight. Dropping it cancels the stream, as for a generation.
pub(super) struct RefineRun {
    cancel: CancellationToken,
    queue: Arc<Mutex<Vec<Update>>>,
    _reader: Task<()>,
}

impl Drop for RefineRun {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The refinement form for one part. It's open while the user is writing, asking, or waiting.
pub(super) struct Refine {
    pub target: RefineTarget,
    pub instruction: String,
    pub phase: Phase,
    /// Whether the replacement may overwrite a part edited by hand. Set once the user has answered.
    pub replace_edited: bool,
    pub run: Option<RefineRun>,
    pub error: Option<String>,
}

impl Refine {
    pub fn new(target: RefineTarget) -> Self {
        Self {
            target,
            instruction: String::new(),
            phase: Phase::Writing,
            replace_edited: false,
            run: None,
            error: None,
        }
    }
}

/// The kind of part a target is, for the reader and the prompt.
fn kind_of(target: RefineTarget) -> RefineKind {
    match target {
        RefineTarget::Spec => RefineKind::Spec,
        RefineTarget::Story(_) => RefineKind::Story,
        RefineTarget::Task(_) => RefineKind::Task,
    }
}

/// Whether a part was written or changed by hand.
pub(super) fn is_edited(draft: &PlanDraft, target: RefineTarget) -> bool {
    match target {
        RefineTarget::Spec => draft.spec_edited(),
        RefineTarget::Story(index) => draft.stories().get(index).is_some_and(|story| story.edited),
        RefineTarget::Task(index) => draft.tasks().get(index).is_some_and(|task| task.edited),
    }
}

/// The plan as text, so the model can see the parts it isn't replacing. Assignees are named as the team names them.
fn plan_context(draft: &PlanDraft, members: &[TeamMember]) -> String {
    let labels = display_labels(members);
    let assignee = |id: Option<looji_domain::TeamMemberId>| {
        id.and_then(|id| {
            members
                .iter()
                .position(|member| member.id() == id)
                .and_then(|index| labels.get(index).cloned())
        })
        .unwrap_or_else(|| "unassigned".to_string())
    };
    let mut text = format!("Spec:\n{}\n\nUser stories:\n", draft.spec());
    for story in draft.stories() {
        text.push_str(&format!("- {}: {}\n", story.title, story.body));
    }
    text.push_str("\nTasks:\n");
    for task in draft.tasks() {
        let estimate = task
            .estimate
            .map(|estimate| format!("{:.2} h", f64::from(estimate.minutes()) / 60.0))
            .unwrap_or_else(|| "no estimate".to_string());
        text.push_str(&format!(
            "- {} (priority {}, estimate {}, assignee {}, tags {})\n",
            task.title,
            task.priority.as_str(),
            estimate,
            assignee(task.assignee),
            task.tags.join(", ")
        ));
    }
    text
}

/// The domain's version of a refined part. A task's section is left out, since a refinement never moves a task to
/// another story, and the store keeps the one it has.
fn into_part(part: Refinement) -> RefinedPart {
    match part {
        Refinement::Spec(spec) => RefinedPart::Spec(spec),
        Refinement::Story(story) => RefinedPart::Story(PlanStory {
            title: story.title,
            body: story.body,
            edited: false,
        }),
        Refinement::Task(task) => RefinedPart::Task(PlanTask {
            title: task.title,
            priority: task.priority,
            estimate: task.estimate,
            assignee: task.assignee,
            tags: task.tags,
            section: None,
            edited: false,
        }),
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

impl PlanWizardDialog {
    /// Whether a refinement is running, which blocks other changes to the plan.
    pub fn refining(&self) -> bool {
        self.refine
            .as_ref()
            .is_some_and(|refine| refine.run.is_some())
    }
}

impl WorkspaceView {
    /// Opens the refinement form under one part. Nothing changes until the instruction is sent.
    pub(super) fn open_refine(&mut self, target: RefineTarget, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.refining() {
            return;
        }
        dialog.refine = Some(Refine::new(target));
        cx.notify();
    }

    /// Closes the form. A refinement that's running is cancelled, and its part is left as it was.
    pub(super) fn cancel_refine(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_wizard.as_mut() {
            dialog.refine = None;
        }
        cx.notify();
    }

    /// Sends the instruction. A part edited by hand asks first.
    pub(super) fn send_refine(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(refine) = dialog.refine.as_mut() else {
            return;
        };
        if refine.run.is_some() {
            return;
        }
        if refine.instruction.trim().is_empty() {
            refine.error = Some("Say what should change.".to_string());
            return cx.notify();
        }
        let edited = dialog
            .draft
            .as_ref()
            .is_some_and(|draft| is_edited(draft, refine.target));
        if edited {
            refine.phase = Phase::Confirming;
            refine.error = None;
            return cx.notify();
        }
        self.start_refine(false, cx);
    }

    /// The answer to the question about a part edited by hand. Replacing starts the refinement. Keeping closes the
    /// form, and the part stays as it is.
    pub(super) fn answer_refine_overwrite(&mut self, replace: bool, cx: &mut Context<Self>) {
        if replace {
            self.start_refine(true, cx);
        } else {
            self.cancel_refine(cx);
        }
    }

    /// Starts the request for the instruction. The reply is read when it has all arrived.
    fn start_refine(&mut self, replace_edited: bool, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(draft) = dialog.draft.as_ref() else {
            return;
        };
        let Some(refine) = dialog.refine.as_mut() else {
            return;
        };
        let instruction = refine.instruction.trim().to_string();
        let context = plan_context(draft, &dialog.members);
        let prompt = build_refinement_prompt(
            kind_of(refine.target),
            &instruction,
            &context,
            &dialog.members,
        );
        let request = ChatRequest {
            model: DEFAULT_MODEL.to_string(),
            system: None,
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: MAX_TOKENS,
        };

        let cancel = CancellationToken::new();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&queue);
        let request_future = run_request(request, cancel.clone(), move |update| {
            if let Ok(mut queue) = sink.lock() {
                queue.push(update);
            }
        });
        // Detached. The reader below is what waits for it, and cancelling stops it.
        drop(looji_ai::runtime().spawn(request_future));

        let reader = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_reading = this
                    .update(cx, |view, cx| view.read_refine_updates(cx))
                    .unwrap_or(false);
                if !keep_reading {
                    break;
                }
            }
        });
        refine.phase = Phase::Running;
        refine.replace_edited = replace_edited;
        refine.error = None;
        refine.run = Some(RefineRun {
            cancel,
            queue,
            _reader: reader,
        });
        cx.notify();
    }

    /// Reads what the refinement has sent. Returns whether the reader should keep going. Only the finished reply
    /// matters here, so text that arrives is ignored.
    fn read_refine_updates(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return false;
        };
        let Some(refine) = dialog.refine.as_ref() else {
            return false;
        };
        let Some(run) = refine.run.as_ref() else {
            return false;
        };
        let updates = match run.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        dialog.frame = dialog.frame.wrapping_add(1);
        let outcome = updates.into_iter().find_map(|update| match update {
            Update::Finished(text) => Some(Ok(text)),
            Update::Failed(err) => Some(Err(err.to_string())),
            Update::Text(_) => None,
        });
        match outcome {
            Some(outcome) => {
                self.finish_refine(outcome, cx);
                false
            }
            None => {
                cx.notify();
                true
            }
        }
    }

    /// Reads the reply for one part, and replaces that part in the draft. Any problem leaves the draft as it was and
    /// keeps the form open with the reason.
    fn finish_refine(&mut self, outcome: Result<String, String>, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(refine) = dialog.refine.as_mut() else {
            return;
        };
        refine.run = None;
        let target = refine.target;
        let replace_edited = refine.replace_edited;
        let text = match outcome {
            Ok(text) => text,
            Err(message) => {
                refine.phase = Phase::Writing;
                refine.error = Some(format!("The part wasn't refined: {message}"));
                return cx.notify();
            }
        };
        let refined = match parse_refinement(&text, kind_of(target), &dialog.members) {
            Ok(refined) => refined,
            Err(err) => {
                refine.phase = Phase::Writing;
                refine.error = Some(format!("The reply couldn't be used: {err}"));
                return cx.notify();
            }
        };
        let Some(id) = dialog.draft.as_ref().map(PlanDraft::id) else {
            return;
        };
        let unassigned = !refined.warnings.is_empty();
        let result = PlanDraftRepository::apply_refinement(
            store(cx),
            id,
            target,
            into_part(refined.part),
            replace_edited,
            now(),
        );
        match result {
            Ok(Some(updated)) => {
                dialog.draft = Some(updated);
                dialog.refine = None;
                dialog.notice = unassigned.then(|| {
                    "An assignee didn't match the team, so that task is unassigned.".to_string()
                });
            }
            Ok(None) => {
                dialog.refine = None;
                dialog.notice = Some("This plan is no longer stored.".to_string());
            }
            Err(RepoError::Rejected(DomainError::Draft(PlanDraftError::ManuallyEdited))) => {
                // The part was edited after the question was asked. Ask again rather than replacing it.
                if let Some(refine) = dialog.refine.as_mut() {
                    refine.phase = Phase::Confirming;
                    refine.error = None;
                }
            }
            Err(err) => {
                if let Some(refine) = dialog.refine.as_mut() {
                    refine.phase = Phase::Writing;
                    refine.error = Some(match err {
                        RepoError::Rejected(err) => err.to_string(),
                        other => format!("Could not save the refined part: {other}"),
                    });
                }
            }
        }
        cx.notify();
    }
}

/// The form under a part, when it's the part being refined. Nothing is drawn for any other part.
pub(super) fn form(
    dialog: &PlanWizardDialog,
    target: RefineTarget,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let refine = dialog
        .refine
        .as_ref()
        .filter(|refine| refine.target == target)?;
    let body = match refine.phase {
        Phase::Writing => {
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .child(text_box(
                    &refine.instruction,
                    "What should change? For example: make the estimates smaller",
                    true,
                    theme,
                ))
                .when_some(refine.error.clone(), |column, message| {
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
                        .child(button("refine-cancel", "Cancel", false, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_refine(cx)),
                        ))
                        .child(button("refine-send", "Refine", true, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.send_refine(cx)),
                        )),
                )
                .into_any_element()
        }
        Phase::Confirming => div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(div().child("This was edited by hand. Replace it anyway?"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(theme.spacing.sm)
                    .child(
                        button("refine-keep", "Keep mine", false, theme).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.answer_refine_overwrite(false, cx),
                        )),
                    )
                    .child(
                        button("refine-replace", "Replace", true, theme).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.answer_refine_overwrite(true, cx),
                        )),
                    ),
            )
            .into_any_element(),
        Phase::Running => div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(theme.spacing.sm)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.sm)
                    .child(SPINNER[dialog.frame % SPINNER.len()])
                    .child("Refining this part. The rest of the plan stays as it is."),
            )
            .child(
                button("refine-stop", "Cancel", false, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_refine(cx))),
            )
            .into_any_element(),
    };
    Some(
        div()
            .id("refine-form")
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .p(theme.spacing.sm)
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .child(body)
            .into_any_element(),
    )
}

/// Keys while the refinement form is open: typing goes to the instruction while it's being written, and Escape closes
/// the form. Nothing else takes the keys until it's closed.
pub(super) fn refine_key(
    view: &mut WorkspaceView,
    event: &KeyDownEvent,
    cx: &mut Context<WorkspaceView>,
) {
    let key = event.keystroke.key.as_str();
    let Some(refine) = view
        .plan_wizard
        .as_ref()
        .and_then(|dialog| dialog.refine.as_ref())
    else {
        return;
    };
    match (refine.phase, key) {
        (_, "escape") => view.cancel_refine(cx),
        (Phase::Writing, "enter") => view.send_refine(cx),
        (Phase::Confirming, "enter") => view.answer_refine_overwrite(false, cx),
        (Phase::Writing, "backspace") => {
            if let Some(refine) = view.plan_wizard.as_mut().and_then(|d| d.refine.as_mut()) {
                refine.instruction.pop();
                refine.error = None;
            }
            cx.notify();
        }
        (Phase::Writing, _) => {
            let modifiers = event.keystroke.modifiers;
            if !(modifiers.control || modifiers.alt || modifiers.platform)
                && let Some(text) = typed_text(&event.keystroke)
                && let Some(refine) = view.plan_wizard.as_mut().and_then(|d| d.refine.as_mut())
            {
                refine.instruction.push_str(text);
                refine.error = None;
                cx.notify();
            }
        }
        _ => {}
    }
}
