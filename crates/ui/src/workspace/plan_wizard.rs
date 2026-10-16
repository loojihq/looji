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
    models::DEFAULT_MODEL,
    planning::{ParsedPlan, build_prompt, parse_plan},
};
use looji_domain::{
    DraftState, DueDate, Generation, PlanDraft, PlanDraftId, PlanDraftRepository, PlanSection,
    PlanTask, PlanWizard, ProjectId, RepoError, TeamMember, TeamMemberId, TeamMemberRepository,
    Timestamp, WizardPhase, display_labels,
};

use super::fields::{field, text_box};
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

/// How often the UI reads what the stream has sent. Several reads a second is enough for a counter, and it
/// keeps a burst of tokens from repainting the panel once per token.
const TICK: Duration = Duration::from_millis(100);
const MAX_TOKENS: u32 = 8192;
/// Most characters a date can take, as in `YYYY-MM-DD`, with room for a mistake.
const DATE_CHARS: usize = 10;
/// The frames of the progress indicator. The stream gives no percentage, so it spins.
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

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
    /// A failure outside the plan itself, such as a store that couldn't be written.
    pub notice: Option<String>,
    /// Counts reads while generating, to turn the spinner.
    pub frame: usize,
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
    pub(super) fn open_plan_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let mut notice = None;
        let members = match TeamMemberRepository::list_for_project(store(cx), project) {
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
            wizard: PlanWizard::new(DueDate::today_utc(), None),
            draft: None,
            run: None,
            interrupted,
            plan: None,
            members,
            confirming_discard: false,
            notice,
            frame: 0,
        });
        self.plan_focus[0].focus(window);
        cx.notify();
    }

    /// Closes the wizard and gives the keyboard back. Any generation still running is cancelled first.
    fn close_plan_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_plan_generation(cx);
        self.plan_wizard = None;
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Escape, Close, and a click outside. Closes at once when nothing would be lost. Otherwise asks first.
    fn request_plan_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        match TeamMemberRepository::list_for_project(store(cx), dialog.project) {
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
                let sections: Vec<PlanSection> = std::iter::once(PlanSection {
                    title: "Spec".to_string(),
                    body: plan.spec.clone(),
                })
                .chain(plan.user_stories.iter().map(|story| PlanSection {
                    title: story.title.clone(),
                    body: story.body.clone(),
                }))
                .collect();
                let tasks: Vec<PlanTask> = plan
                    .tasks
                    .iter()
                    .map(|task| PlanTask {
                        title: task.title.clone(),
                        section: None,
                    })
                    .collect();
                if let Some(draft) = dialog.draft.as_mut()
                    && let Err(err) = draft.complete(sections, tasks, now())
                {
                    dialog.notice = Some(format!("Could not record the plan: {err}"));
                }
                dialog.wizard.succeed();
                dialog.plan = Some(plan);
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
    fn resume_interrupted_plan(&mut self, cx: &mut Context<Self>) {
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

    fn cycle_plan_team(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        let mut options: Vec<Option<TeamMemberId>> = vec![None];
        options.extend(dialog.member_ids().into_iter().map(Some));
        let current = options
            .iter()
            .position(|option| *option == dialog.wizard.team())
            .unwrap_or(0);
        dialog
            .wizard
            .set_team(options[(current + 1) % options.len()]);
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

    fn focus_plan_control(&self, control: Control, window: &mut Window) {
        if let Some(index) = CONTROLS.iter().position(|candidate| *candidate == control) {
            self.plan_focus[index].focus(window);
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
                self.plan_focus[next].focus(window);
                return cx.notify();
            }
            _ => {}
        }
        let focused = self.focused_plan_control(window);
        let modifier = modifiers.control || modifiers.alt || modifiers.platform;
        match focused {
            Some(Control::Goal) if key == "enter" => self.submit_plan(cx),
            Some(Control::Date) if key == "enter" => self.submit_plan(cx),
            Some(Control::Goal) => {
                self.edit_plan_text(Control::Goal, &event.keystroke, modifier, cx)
            }
            Some(Control::Date) => {
                self.edit_plan_text(Control::Date, &event.keystroke, modifier, cx)
            }
            Some(Control::Team) if matches!(key, "enter" | "space") => self.cycle_plan_team(cx),
            Some(Control::Primary) if matches!(key, "enter" | "space") => {
                self.primary_plan_action(window, cx)
            }
            Some(Control::Close) if key == "enter" => self.request_plan_close(window, cx),
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
        let mut text = match control {
            Control::Goal => dialog.wizard.goal().to_string(),
            Control::Date => dialog.wizard.target_text().to_string(),
            _ => return,
        };
        if keystroke.key == "backspace" {
            text.pop();
        } else if !modifier
            && let Some(typed) = typed_text(keystroke)
            && (control == Control::Goal || {
                typed.chars().all(|c| c.is_ascii_digit() || c == '-')
                    && text.chars().count() + typed.chars().count() <= DATE_CHARS
            })
        {
            text.push_str(typed);
        } else {
            return;
        }
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
    div()
        .id("plan-open")
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.body)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("Plan")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_plan_wizard(window, cx)))
}

/// The wizard, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.plan_wizard.as_ref()?;
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
    let team_label = {
        let labels = display_labels(&dialog.members);
        match dialog.wizard.team() {
            None => "No one".to_string(),
            Some(id) => dialog
                .members
                .iter()
                .position(|member| member.id() == id)
                .and_then(|index| labels.get(index).cloned())
                .unwrap_or_else(|| "No one".to_string()),
        }
    };

    let goal = div()
        .id("plan-goal")
        .track_focus(&focus[0])
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.focus_plan_control(Control::Goal, window);
            cx.notify();
        }))
        .child(text_box(
            dialog.wizard.goal(),
            "For example: launch the beta to the first group of users",
            focused(Control::Goal),
            theme,
        ));

    let team = div()
        .id("plan-team")
        .track_focus(&focus[2])
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if focused(Control::Team) {
            theme.palette.accent
        } else {
            theme.palette.border_control
        })
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(format!("Team: {team_label}"))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.focus_plan_control(Control::Team, window);
            this.cycle_plan_team(cx);
        }));

    let date = div()
        .id("plan-date")
        .track_focus(&focus[1])
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.focus_plan_control(Control::Date, window);
            cx.notify();
        }))
        .child(text_box(
            dialog.wizard.target_text(),
            "YYYY-MM-DD",
            focused(Control::Date),
            theme,
        ));

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
        WizardPhase::Editing => ("Generate", dialog.wizard.can_generate()),
        WizardPhase::Generating { .. } => ("Cancel", true),
        WizardPhase::Failed { .. } => ("Retry", dialog.wizard.can_generate()),
        WizardPhase::Done => ("Done", true),
    };
    let primary_focus = &focus[3];
    let primary = button("plan-primary", primary_label, true, theme).track_focus(primary_focus);
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

    let panel = div()
        .id("plan-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w(px(520.))
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
                .child("Plan from a goal"),
        )
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(project),
        )
        .children(interrupted)
        .child(field("Goal", goal_error.as_deref(), goal, theme))
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(theme.spacing.md)
                .child(team)
                .child(div().w(px(180.)).child(field(
                    "Target date",
                    date_error.as_deref(),
                    date,
                    theme,
                ))),
        )
        .when_some(date_warning, |panel, warning| {
            panel.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(warning),
            )
        })
        .children(body)
        .when_some(dialog.notice.clone(), |panel, notice| {
            panel.child(div().text_color(theme.palette.danger).child(notice))
        })
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.spacing.sm)
                .child(
                    button("plan-close", "Close", false, theme)
                        .track_focus(&focus[4])
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.request_plan_close(window, cx)
                        })),
                )
                .child(primary),
        );

    let mut layer = div()
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
        .child(panel);
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
