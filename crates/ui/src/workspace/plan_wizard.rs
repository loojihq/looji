//! The planning wizard: describe a goal, pick a team and a target date, then watch the plan generate. It opens from
//! the Plan button in the header and owns the keyboard while it's open.
//!
//! The inputs and the phase live in the domain's `PlanWizard`. This file moves text between the request, the
//! store, and the wizard, and draws what the wizard says. Each generation runs on the AI runtime, and the UI reads
//! what it sends about a few times a second, so a token doesn't repaint the panel on its own.
//!
//! Closing with a goal typed in asks first. Cancel stops the stream and marks the draft as failed, with the
//! reason, and the inputs stay put.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{prelude::*, *};
use looji_ai::{
    CancellationToken, ChatMessage, ChatRequest,
    generation::{Update, run_request},
    planning::{ParsedPlan, build_prompt, parse_plan},
};
use looji_domain::{
    DraftState, DueDate, Generation, PlanDraft, PlanDraftId, PlanDraftRepository, PlanStory,
    PlanTask, PlanWizard, ProjectId, RepoError, TeamMember, TeamMemberId, TeamMemberRepository,
    Timestamp, WizardPhase, display_labels,
};

use super::fields::{field, member_mark};
use super::plan_review;
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::{components, text_edit::TextEdit, theme::Theme};

/// How often the UI reads what the stream has sent. Several reads a second is enough for a counter, and it
/// keeps a burst of tokens from repainting the panel once per token.
const TICK: Duration = Duration::from_millis(100);
const MAX_TOKENS: u32 = 8192;
/// Most characters a date can take, as in `YYYY-MM-DD`, with room for a mistake.
const DATE_CHARS: usize = 10;
/// The frames of the progress indicator. The stream gives no percentage, so it spins.
pub(super) const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// The controls the keyboard reaches, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Goal,
    Date,
    Team,
    /// Generate, Cancel, Retry, or Done, depending on the phase.
    Primary,
    Close,
}

pub(super) const CONTROLS: [Control; 5] = [
    Control::Goal,
    Control::Date,
    Control::Team,
    Control::Primary,
    Control::Close,
];

/// One generation in flight. Dropping it cancels the stream and stops the reader, so a dropped wizard can't leave
/// a request running.
pub(super) struct Run {
    cancel: CancellationToken,
    queue: Arc<Mutex<Vec<Update>>>,
    _reader: Task<()>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(super) struct PlanWizardDialog {
    pub project: ProjectId,
    pub wizard: PlanWizard,
    /// The caret and selection of the goal or date being typed. See `edit_text`.
    pub input: TextEdit,
    /// The draft for the current attempt. Its text is written as it arrives.
    pub draft: Option<PlanDraft>,
    pub run: Option<Run>,
    /// A draft the app was closed during. Offered for resume or discard.
    pub interrupted: Option<PlanDraft>,
    /// The plan once it has been read, for the next step to show.
    pub plan: Option<ParsedPlan>,
    /// The project's team, read when the wizard opened and again when generation starts.
    pub members: Vec<TeamMember>,
    pub confirming_discard: bool,
    /// The review edit that is open, if any.
    pub editor: Option<super::plan_review::Editor>,
    /// The refinement form, when one part is being refined.
    pub refine: Option<super::plan_refine::Refine>,
    /// A failure outside the plan itself, such as a store that couldn't be written.
    pub notice: Option<String>,
    /// Counts reads while generating, to turn the spinner.
    pub frame: usize,
    /// The page each review list shows, counted from zero. Stories and tasks page separately.
    pub story_page: usize,
    pub task_page: usize,
}

impl PlanWizardDialog {
    fn member_ids(&self) -> Vec<TeamMemberId> {
        self.members.iter().map(TeamMember::id).collect()
    }

    /// Whether closing now would lose something: a goal typed in, and no generation running.
    fn dirty(&self) -> bool {
        !self.wizard.goal().trim().is_empty()
            && self.run.is_none()
            && matches!(
                self.wizard.phase(),
                WizardPhase::Editing | WizardPhase::Failed { .. }
            )
    }

    fn inputs_editable(&self) -> bool {
        matches!(
            self.wizard.phase(),
            WizardPhase::Editing | WizardPhase::Failed { .. }
        )
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

impl WorkspaceView {
    /// Opens the wizard for the selected project. Nothing opens when no project is selected. An interrupted draft
    /// from an earlier session is offered for resume.
    /// Generate with AI. An unfinished plan opens instead, so there is only ever one.
    pub(super) fn open_plan_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unfinished_plan(cx).is_some() {
            return self.open_unfinished_plan(window, cx);
        }
        self.start_plan_wizard(window, cx);
    }

    /// Opens the planning window with a new plan.
    pub(super) fn start_plan_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        // One inline form at a time. A task form with edits asks before it closes.
        if self
            .task_dialog
            .as_ref()
            .is_some_and(|dialog| matches!(dialog.form.mode, super::task_form::Mode::Create))
        {
            self.request_task_dialog_close(cx);
            if self.task_dialog.is_some() {
                return;
            }
        }
        let mut notice = None;
        let members = match TeamMemberRepository::list_for_workspace(
            store(cx),
            self.list.active_workspace_id(),
        ) {
            Ok(members) => members,
            Err(err) => {
                notice = Some(format!("Could not load the team: {err}"));
                Vec::new()
            }
        };
        let interrupted = match PlanDraftRepository::list_for_project(store(cx), project) {
            Ok(drafts) => drafts
                .into_iter()
                .find(|draft| draft.state() == DraftState::Interrupted),
            Err(err) => {
                notice = Some(format!("Could not load earlier plans: {err}"));
                None
            }
        };
        self.plan_wizard = Some(PlanWizardDialog {
            project,
            wizard: default_plan_wizard(),
            input: TextEdit::default(),
            draft: None,
            run: None,
            interrupted,
            plan: None,
            members,
            confirming_discard: false,
            editor: None,
            refine: None,
            notice,
            frame: 0,
            story_page: 0,
            task_page: 0,
        });
        let main = cx.entity();
        cx.defer(move |cx| super::dialog_window::open(cx, main, super::dialog_window::Kind::Plan));
        self.plan_focus[0].focus(window, cx);
        cx.notify();
    }

    /// Closes the wizard and gives the keyboard back. Any generation still running is cancelled first.
    fn close_plan_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_plan_generation(cx);
        self.plan_wizard = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Escape, Close, and a click outside. Closes at once when nothing would be lost. Otherwise asks first.
    pub(super) fn request_plan_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .plan_wizard
            .as_ref()
            .is_some_and(|dialog| dialog.editor.is_some())
            && !self.commit_plan_edit(cx)
        {
            return;
        }
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.dirty() {
            dialog.confirming_discard = true;
            cx.notify();
        } else {
            self.close_plan_wizard(window, cx);
        }
    }

    /// Generate, and Retry after a failure. The inputs are checked in the domain, and nothing is sent when they
    /// fail. A second submit while generating does nothing.
    pub(super) fn submit_plan(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.run.is_some() {
            return;
        }
        match TeamMemberRepository::list_for_workspace(store(cx), self.list.active_workspace_id()) {
            Ok(members) => dialog.members = members,
            Err(err) => {
                dialog.notice = Some(format!("Could not check the team: {err}"));
                return cx.notify();
            }
        }
        let members = dialog.member_ids();
        let attempt = PlanDraftId::new();
        let generation = match dialog.wizard.begin(&members, attempt) {
            Ok(generation) => generation,
            Err(err) => {
                // The wizard keeps the inputs and the error is shown beside the field, so nothing else changes.
                dialog.notice = Some(err.to_string());
                return cx.notify();
            }
        };
        dialog.notice = None;
        let project = dialog.project;
        match PlanDraft::start_as(attempt, project, &generation.goal, now()) {
            Ok(draft) => self.start_plan_run(generation, draft, cx),
            Err(err) => {
                let message = err.to_string();
                if let Some(dialog) = self.plan_wizard.as_mut() {
                    dialog.wizard.fail(&message);
                    dialog.notice = Some(message);
                }
                cx.notify();
            }
        }
    }

    /// Stores the new draft, then starts the request. A draft that can't be saved fails the attempt, and the
    /// inputs stay.
    fn start_plan_run(&mut self, generation: Generation, draft: PlanDraft, cx: &mut Context<Self>) {
        if let Err(err) = PlanDraftRepository::save(store(cx), &draft) {
            if let Some(dialog) = self.plan_wizard.as_mut() {
                dialog.wizard.fail("the plan draft could not be saved");
                dialog.notice = Some(format!("Could not save the plan draft: {err}"));
            }
            return cx.notify();
        }
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let target = generation
            .target
            .map(|date| format!("\n\nTarget date: {date}."))
            .unwrap_or_default();
        let brief = format!("{}{target}", generation.goal);
        let prompt = build_prompt(&brief, &dialog.members);
        let request = ChatRequest {
            model: super::ai_settings::model_for(cx, self.projects.active_id()),
            system: None,
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: MAX_TOKENS,
        };

        let cancel = CancellationToken::new();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&queue);
        let request_future = run_request(
            super::ai_settings::provider_for(cx, self.projects.active_id()),
            request,
            cancel.clone(),
            move |update| {
                if let Ok(mut queue) = sink.lock() {
                    queue.push(update);
                }
            },
        );
        // The task is detached: the reader below is the only thing that waits for it, and cancelling stops it.
        drop(looji_ai::runtime().spawn(request_future));

        let reader = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_reading = this
                    .update(cx, |view, cx| view.read_plan_updates(cx))
                    .unwrap_or(false);
                if !keep_reading {
                    break;
                }
            }
        });
        dialog.draft = Some(draft);
        dialog.run = Some(Run {
            cancel,
            queue,
            _reader: reader,
        });
        cx.notify();
    }

    /// Reads what the stream has sent since the last read, and applies it in order. Returns whether the reader
    /// should keep going. Each read saves the draft once, however many tokens arrived.
    fn read_plan_updates(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return false;
        };
        let Some(run) = dialog.run.as_ref() else {
            return false;
        };
        let updates = match run.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        dialog.frame = dialog.frame.wrapping_add(1);
        let mut arrived = false;
        for update in updates {
            // A chunk after a cancel is dropped here, because the run is gone and the phase isn't generating.
            if self
                .plan_wizard
                .as_ref()
                .and_then(|d| d.run.as_ref())
                .is_none()
            {
                break;
            }
            match update {
                Update::Text(text) => {
                    if let Some(dialog) = self.plan_wizard.as_mut()
                        && dialog.wizard.receive(text.chars().count())
                    {
                        if let Some(draft) = dialog.draft.as_mut() {
                            let _ = draft.append_response(&text, now());
                        }
                        arrived = true;
                    }
                }
                Update::Step(_) | Update::Source(_) => {}
                Update::Finished(full) => {
                    self.finish_plan_run(&full, cx);
                    break;
                }
                Update::Failed(err) => {
                    self.fail_plan_run(&err.to_string(), cx);
                    break;
                }
            }
        }
        if arrived {
            self.save_plan_draft(cx);
        }
        cx.notify();
        self.plan_wizard.as_ref().is_some_and(|d| d.run.is_some())
    }

    /// The whole response arrived. It's parsed once here, against the team as it stands now.
    fn finish_plan_run(&mut self, full: &str, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        dialog.run = None;
        let outcome = parse_plan(full, &dialog.members);
        match outcome {
            Ok(plan) => {
                let stories: Vec<PlanStory> = plan
                    .user_stories
                    .iter()
                    .map(|story| PlanStory {
                        title: story.title.clone(),
                        body: story.body.clone(),
                        edited: false,
                    })
                    .collect();
                let tasks: Vec<PlanTask> = plan
                    .tasks
                    .iter()
                    .map(|task| PlanTask {
                        title: task.title.clone(),
                        priority: task.priority,
                        estimate: task.estimate,
                        assignee: task.assignee,
                        tags: task.tags.clone(),
                        section: None,
                        edited: false,
                    })
                    .collect();
                let recorded = dialog.draft.as_mut().map_or(Ok(()), |draft| {
                    draft.complete(&plan.spec, stories, tasks, now())
                });
                match recorded {
                    Ok(()) => {
                        dialog.wizard.succeed();
                        dialog.plan = Some(plan);
                    }
                    Err(err) => {
                        let message = format!("Could not record the plan: {err}");
                        dialog.wizard.fail(&message);
                        dialog.notice = Some(message);
                    }
                }
            }
            Err(err) => {
                let message = err.to_string();
                if let Some(draft) = dialog.draft.as_mut() {
                    let _ = draft.fail(&message, now());
                }
                dialog.wizard.fail(&message);
            }
        }
        self.save_plan_draft(cx);
        cx.notify();
    }

    fn fail_plan_run(&mut self, message: &str, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        dialog.run = None;
        if let Some(draft) = dialog.draft.as_mut() {
            let _ = draft.fail(message, now());
        }
        dialog.wizard.fail(message);
        self.save_plan_draft(cx);
        cx.notify();
    }

    /// Writes the current draft. A failure is shown in the panel, and the generation carries on.
    fn save_plan_draft(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(draft) = dialog.draft.as_ref() else {
            return;
        };
        if let Err(err) = PlanDraftRepository::save(store(cx), draft) {
            dialog.notice = Some(save_message(&err));
        }
    }

    /// Stops the running generation, and marks its draft failed with the reason. The text received so far stays on
    /// the draft, and the wizard goes back to editing with its inputs as they were.
    fn cancel_plan_generation(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.run.take().is_none() {
            return;
        }
        dialog.wizard.cancel();
        if let Some(mut draft) = dialog.draft.take() {
            let _ = draft.fail("Cancelled before the plan was finished.", now());
            if let Err(err) = PlanDraftRepository::save(store(cx), &draft) {
                dialog.notice = Some(save_message(&err));
            }
        }
        cx.notify();
    }

    /// Resumes an interrupted draft. Its earlier text moves to the recovered field, and a new generation starts from
    /// its goal, with the team and date now in the wizard.
    pub(super) fn resume_interrupted_plan(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(mut draft) = dialog.interrupted.take() else {
            return;
        };
        if let Err(err) = draft.resume(now()) {
            dialog.notice = Some(format!("Could not resume the plan: {err}"));
            return cx.notify();
        }
        dialog.wizard.set_goal(draft.prompt());
        let members = dialog.member_ids();
        let generation = match dialog.wizard.begin(&members, draft.id()) {
            Ok(generation) => generation,
            Err(err) => {
                dialog.notice = Some(err.to_string());
                return cx.notify();
            }
        };
        dialog.notice = None;
        self.start_plan_run(generation, draft, cx);
    }

    /// Discards an interrupted draft.
    fn discard_interrupted_plan(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let Some(draft) = dialog.interrupted.take() else {
            return;
        };
        if let Err(err) = PlanDraftRepository::delete(store(cx), draft.id()) {
            dialog.notice = Some(save_message(&err));
            dialog.interrupted = Some(draft);
        }
        cx.notify();
    }

    /// Enter, and the primary button, do what the phase needs: generate, cancel, or close once done.
    fn primary_plan_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_ref() else {
            return;
        };
        match dialog.wizard.phase() {
            WizardPhase::Editing | WizardPhase::Failed { .. } => self.submit_plan(cx),
            WizardPhase::Generating { .. } => self.cancel_plan_generation(cx),
            WizardPhase::Done => self.close_plan_wizard(window, cx),
        }
    }

    fn focus_plan_control(&self, control: Control, window: &mut Window, cx: &mut App) {
        if let Some(index) = CONTROLS.iter().position(|candidate| *candidate == control) {
            self.plan_focus[index].focus(window, cx);
        }
    }

    fn focused_plan_control(&self, window: &Window) -> Option<Control> {
        CONTROLS
            .iter()
            .zip(&self.plan_focus)
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(control, _)| *control)
    }

    /// Keys while the wizard is open. Escape closes, Tab moves between controls, and typing goes to the focused
    /// text field while its inputs are editable.
    pub(super) fn plan_wizard_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if dialog.confirming_discard {
            // Keep editing is the default, so Enter and Escape both keep the draft.
            if matches!(key, "enter" | "escape") {
                dialog.confirming_discard = false;
                cx.notify();
            }
            return;
        }
        if matches!(dialog.wizard.phase(), WizardPhase::Done) {
            return self.handle_review_key(event, window, cx);
        }
        match key {
            "escape" => return self.request_plan_close(window, cx),
            "tab" => {
                let count = CONTROLS.len();
                let current = self.plan_position(window).unwrap_or(0);
                let next = if modifiers.shift {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                self.plan_focus[next].focus(window, cx);
                return cx.notify();
            }
            _ => {}
        }
        let focused = self.focused_plan_control(window);
        let modifier = modifiers.control || modifiers.alt || modifiers.platform;
        match focused {
            Some(Control::Goal) if key == "enter" && event.keystroke.modifiers.control => {
                self.submit_plan(cx)
            }
            Some(Control::Date) if key == "enter" => self.submit_plan(cx),
            Some(Control::Goal) => {
                self.edit_plan_text(Control::Goal, &event.keystroke, modifier, cx)
            }
            Some(Control::Date) => {
                self.edit_plan_text(Control::Date, &event.keystroke, modifier, cx)
            }
            _ => {}
        }
    }

    fn plan_position(&self, window: &Window) -> Option<usize> {
        self.plan_focus
            .iter()
            .position(|handle| handle.is_focused(window))
    }

    /// Types into the goal or the date, while the inputs are editable. The date is capped at the length of a day.
    fn edit_plan_text(
        &mut self,
        control: Control,
        keystroke: &Keystroke,
        modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if !dialog.inputs_editable() {
            return;
        }
        let current = match control {
            Control::Goal => dialog.wizard.goal().to_string(),
            Control::Date => dialog.wizard.target_text().to_string(),
            _ => return,
        };
        // The wizard keeps the text itself. The caret and selection live here, and start over when the
        // text was changed from outside, such as by switching fields.
        if dialog.input.as_str() != current {
            dialog.input = TextEdit::new(current);
        }
        if keystroke.key == "enter" && control == Control::Goal {
            dialog.input.insert("\n");
        } else if keystroke.key == "enter" || keystroke.key == "escape" {
            return;
        } else if control == Control::Date
            && !modifier
            && let Some(typed) = typed_text(keystroke)
            && (!typed.chars().all(|c| c.is_ascii_digit() || c == '-')
                || dialog.input.chars().count() + typed.chars().count() > DATE_CHARS)
        {
            return;
        } else if !dialog.input.handle_key(keystroke) {
            return;
        }
        let text = dialog.input.as_str().to_string();
        match control {
            Control::Goal => dialog.wizard.set_goal(&text),
            Control::Date => dialog.wizard.set_target_text(&text),
            _ => {}
        }
        cx.notify();
    }
}

fn save_message(err: &RepoError) -> String {
    format!("Could not save the plan draft: {err}")
}

/// The Plan button in the header.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    crate::components::button_icon(
        "plan-open",
        "sparkles",
        "Generate with AI",
        crate::components::Variant::Primary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_plan_wizard(window, cx)))
}

/// The wizard, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.plan_wizard.as_ref()?;
    if matches!(dialog.wizard.phase(), WizardPhase::Done) {
        return plan_review::panel(view, theme, cx);
    }
    let focus = &view.plan_focus;
    let focused = |control: Control| {
        CONTROLS
            .iter()
            .position(|candidate| *candidate == control)
            .is_some_and(|index| focus[index].is_focused(window))
    };
    let project = view
        .projects
        .find_by_id(dialog.project)
        .map(|project| project.name().to_string())
        .unwrap_or_default();
    let phase = dialog.wizard.phase().clone();
    let editable = dialog.inputs_editable();

    // The goal's reason shows beside its field once the field is empty and editable.
    let goal_error = if editable {
        dialog.wizard.goal_problem().map(|err| err.to_string())
    } else {
        None
    };
    let date_error = match dialog.wizard.target_date() {
        Err(err) => Some(err.to_string()),
        Ok(_) => None,
    };

    // The caret belongs to the text it was typed in. When the wizard holds other text, show it plainly.
    let goal_text = if dialog.input.as_str() == dialog.wizard.goal() {
        dialog.input.clone()
    } else {
        TextEdit::new(dialog.wizard.goal())
    };
    let goal = div()
        .id("plan-goal")
        .h_full()
        .track_focus(&focus[0])
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.focus_plan_control(Control::Goal, window, cx);
            cx.notify();
        }))
        .child(components::edit_lines(
            &goal_text,
            "For example: launch the beta to the first group of users",
            focused(Control::Goal),
            240.,
            theme,
        ));

    let team = team_choice(&dialog.members, dialog.wizard.team(), &focus[2], theme, cx);

    let date = super::date_picker::date_field(
        super::date_picker::DateTarget::PlanTarget,
        dialog.wizard.target_text(),
        view.open_calendar,
        theme,
        cx,
    );

    let body = match &phase {
        WizardPhase::Editing => None,
        WizardPhase::Generating { received_chars, .. } => Some(
            div()
                .flex()
                .flex_row()
                .gap(theme.spacing.sm)
                .items_center()
                .child(SPINNER[dialog.frame % SPINNER.len()])
                .child(format!(
                    "Generating a plan. Received {received_chars} characters."
                ))
                .into_any_element(),
        ),
        WizardPhase::Failed { message } => Some(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .child(
                    div()
                        .text_color(theme.palette.danger)
                        .child(format!("The plan wasn't generated: {message}")),
                )
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child("Your goal, team, and date are unchanged. Retry to try again."),
                )
                .into_any_element(),
        ),
        WizardPhase::Done => Some(match dialog.plan.as_ref() {
            Some(plan) => {
                let mut lines = vec![format!(
                    "The plan is ready: {} user stories, {} tasks.",
                    plan.user_stories.len(),
                    plan.tasks.len()
                )];
                if !plan.warnings.is_empty() {
                    lines.push(format!(
                        "{} assignee(s) didn't match the team and were left unassigned.",
                        plan.warnings.len()
                    ));
                }
                let mut column = div().flex().flex_col().gap(theme.spacing.xs);
                for line in lines {
                    column = column.child(line);
                }
                column.into_any_element()
            }
            None => div().child("The plan is ready.").into_any_element(),
        }),
    };

    let date_warning = (dialog.wizard.target_is_past() && date_error.is_none())
        .then(|| "This date is in the past. You can still generate a plan.".to_string());

    let (primary_label, primary_enabled) = match &phase {
        WizardPhase::Editing => ("Generate plan", dialog.wizard.can_generate()),
        WizardPhase::Generating { .. } => ("Cancel", true),
        WizardPhase::Failed { .. } => ("Retry", dialog.wizard.can_generate()),
        WizardPhase::Done => ("Done", true),
    };
    let primary_focus = &focus[3];
    let primary = if matches!(phase, WizardPhase::Editing | WizardPhase::Failed { .. }) {
        crate::components::button_icon(
            "plan-primary",
            "sparkles",
            primary_label,
            crate::components::Variant::Primary,
            theme,
        )
    } else {
        button("plan-primary", primary_label, true, theme)
    }
    .track_focus(primary_focus);
    let primary = if primary_enabled {
        primary.on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.primary_plan_action(window, cx)),
        )
    } else {
        primary.opacity(0.5).cursor_default()
    };

    let interrupted = dialog.interrupted.as_ref().map(|draft| {
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .p(theme.spacing.sm)
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .child(div().child(format!(
                "An earlier plan for \"{}\" was interrupted when the app closed. Its text is kept.",
                draft.prompt()
            )))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(theme.spacing.sm)
                    .child(button("plan-resume", "Resume", true, theme).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.resume_interrupted_plan(cx)),
                    ))
                    .child(
                        button("plan-discard-interrupted", "Discard", false, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.discard_interrupted_plan(cx)
                            }),
                        ),
                    ),
            )
    });

    let header = div()
        .text_color(theme.palette.text_secondary)
        .child(format!(
            "Describe what you want to build for {project}. The AI drafts the spec, the user stories and the tasks, and you review every change before anything reaches the board."
        ));

    // The grid. The goal takes the main column and fills its height; the side column holds each setting in a card.
    let goal_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .gap(theme.spacing.md)
        .children(interrupted)
        .child(div().flex().flex_col().flex_1().min_h_0().child(field(
            "What do you want to build?",
            goal_error.as_deref(),
            div().flex_1().min_h_0().child(goal),
            theme,
        )));

    let schedule = card(
        "Schedule",
        theme,
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.sm)
            .child(field("Target date", date_error.as_deref(), date, theme))
            .children(date_warning.map(|warning| {
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(warning)
            })),
    );
    let team_card = card("Team", theme, field("Who works on it", None, team, theme));
    let side = div()
        .flex()
        .flex_col()
        .w(px(300.))
        .flex_none()
        .gap(theme.spacing.md)
        .child(schedule)
        .child(team_card);

    let grid = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.lg)
        .flex_1()
        .min_h_0()
        .child(goal_column)
        .child(side);

    // The footer: what the plan is doing or what went wrong, then the action.
    let status_line = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .children(body)
        .when_some(dialog.notice.clone(), |line, notice| {
            // A notice that reports a success is plain text. Anything else is a problem, so it's shown in red.
            let colour = if notice.starts_with("Published") {
                theme.palette.text_primary
            } else {
                theme.palette.danger
            };
            line.child(div().text_color(colour).child(notice))
        });
    let footer = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.md)
        .pt(theme.spacing.md)
        .border_t_1()
        .border_color(theme.palette.border)
        .child(status_line)
        .child(primary);

    let panel = div()
        .id("plan-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.lg)
        .size_full()
        .p(theme.spacing.lg)
        .bg(theme.palette.surface)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(header)
        .child(grid)
        .child(footer);

    // The panel fills the window. An unsaved-changes prompt is drawn over it, in the same window.
    let mut layer = div().relative().size_full().child(panel);
    if dialog.confirming_discard {
        layer = layer.child(discard_prompt(theme, cx));
    }
    Some(layer.into_any_element())
}

/// The prompt before closing with a goal typed in. Keep editing is the default.
fn discard_prompt(theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    div()
        .id("plan-discard-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .child(
            div()
                .id("plan-discard-panel")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .w(px(380.))
                .p(theme.spacing.lg)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .shadow(theme.elevation.overlay.clone())
                .border_1()
                .border_color(theme.palette.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(div().child("Close and discard the goal you typed?"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(button("plan-keep", "Keep editing", true, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Some(dialog) = this.plan_wizard.as_mut() {
                                    dialog.confirming_discard = false;
                                }
                                cx.notify();
                            }),
                        ))
                        .child(button("plan-discard", "Discard", false, theme).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close_plan_wizard(window, cx)
                            }),
                        )),
                ),
        )
        .into_any_element()
}

/// One choice in the team box: a chip, filled when it's the chosen one.
fn team_chip(
    id: impl Into<ElementId>,
    label: &str,
    mark: Option<AnyElement>,
    selected: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.xs)
        .h(px(28.))
        .px(theme.spacing.sm)
        .rounded(px(14.))
        .cursor_pointer()
        .when(selected, |chip| {
            chip.bg(theme.palette.surface_selected)
                .text_color(theme.palette.text_primary)
        })
        .when(!selected, |chip| {
            chip.text_color(theme.palette.text_secondary)
                .hover(|style| {
                    style
                        .bg(theme.palette.surface_hover)
                        .text_color(theme.palette.text_primary)
                })
        })
        .children(mark)
        .child(label.to_string())
}

/// The team, in a dotted muted box. With no team it says so. With one it offers "No one" and each member.
fn team_choice(
    members: &[TeamMember],
    selected: Option<TeamMemberId>,
    focus: &FocusHandle,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let base = div()
        .id("plan-team")
        .track_focus(focus)
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(theme.spacing.sm)
        .min_h(px(44.))
        .px(theme.spacing.sm)
        .py(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_dashed()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.body);
    if members.is_empty() {
        return base
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_color(theme.palette.text_secondary)
                    .child("No team members yet. Generated tasks will be unassigned."),
            )
            .into_any_element();
    }
    let labels = display_labels(members);
    let mut row = base.child(
        team_chip("plan-team-none", "No one", None, selected.is_none(), theme)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_plan_team(None, cx))),
    );
    for (index, member) in members.iter().enumerate() {
        let id = member.id();
        let mark = member_mark(&member.initials(), member.colour(), px(18.), theme);
        let label = labels.get(index).cloned().unwrap_or_default();
        row = row.child(
            team_chip(
                ("plan-team-member", index as u64),
                &label,
                Some(mark.into_any_element()),
                selected == Some(id),
                theme,
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_plan_team(Some(id), cx)),
            ),
        );
    }
    row.into_any_element()
}

impl WorkspaceView {
    /// Chooses who the plan's team is, or no one.
    fn set_plan_team(&mut self, team: Option<TeamMemberId>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_wizard.as_mut() {
            dialog.wizard.set_team(team);
        }
        cx.notify();
    }
}

/// A settings card in the side column: a small title over its content, inside a border.
fn card(title: &str, theme: &Theme, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .p(theme.spacing.md)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .text_size(theme.typography.caption)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.palette.text_secondary)
                .child(title.to_string()),
        )
        .child(body)
}

/// A new plan's wizard. Its target date is a month from today, so a plan is planned ahead without typing a date.
fn default_plan_wizard() -> PlanWizard {
    let mut wizard = PlanWizard::new(DueDate::today_utc(), None);
    wizard.set_target_text(&super::date_picker::one_month_ahead());
    wizard
}
