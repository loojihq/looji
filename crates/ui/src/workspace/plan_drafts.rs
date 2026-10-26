//! A project's unfinished plan. A project has one at a time: a plan is saved as it's made, and it waits in the
//! project page until it's opened, finished, or discarded. Published plans are history, so they aren't shown here.
//!
//! Opening an unfinished plan goes to its review. A plan that was interrupted or failed resumes, which starts a new
//! generation from its goal. Generate with AI opens the unfinished plan rather than starting a second one, so
//! discarding is the only way to begin again.

use gpui::{prelude::*, *};
use looji_domain::{DraftState, LocalMoment, PlanDraft, PlanDraftRepository, Timestamp};

use super::task_dialog::button;
use super::{WorkspaceView, store};
use crate::theme::Theme;

/// When a plan was last updated, in UTC, to the minute.
fn describe_time(at: Timestamp) -> String {
    let moment = LocalMoment::from_timestamp_utc(at);
    format!(
        "{} {:02}:{:02} UTC",
        moment.day,
        moment.minute / 60,
        moment.minute % 60
    )
}

/// What the plan is, in a few words, for the project page.
fn state_label(draft: &PlanDraft) -> &'static str {
    match draft.state() {
        DraftState::Generating => "Generating",
        DraftState::Interrupted => "Interrupted",
        DraftState::Failed => "Failed",
        DraftState::Ready => "Ready to review",
    }
}

impl WorkspaceView {
    /// The selected project's unfinished plan: its newest draft that hasn't been published.
    pub(super) fn unfinished_plan(&self, cx: &App) -> Option<PlanDraft> {
        let project = self.projects.active_id()?;
        PlanDraftRepository::list_for_project(store(cx), project)
            .ok()?
            .into_iter()
            .find(|draft| draft.published_at().is_none())
    }

    /// Opens the unfinished plan in its review, or resumes it if it was interrupted or failed.
    pub(super) fn open_unfinished_plan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.unfinished_plan(cx) else {
            return;
        };
        let state = draft.state();
        if state == DraftState::Generating {
            // Only a request running in this process is generating, and that one is already on screen.
            self.status = Some("That plan is still being generated.".to_string());
            return cx.notify();
        }
        self.start_plan_wizard(window, cx);
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if state == DraftState::Ready {
            self.focus_handle.focus(window, cx);
            dialog.wizard = looji_domain::PlanWizard::finished(looji_domain::DueDate::today_utc());
            dialog.draft = Some(draft);
            cx.notify();
        } else {
            dialog.interrupted = Some(draft);
            self.resume_interrupted_plan(cx);
        }
    }

    /// Discards the project's unfinished plan, and any older unpublished drafts with it. Published plans stay.
    pub(super) fn discard_unfinished_plan(&mut self, cx: &mut Context<Self>) {
        self.plan_discard = false;
        let Some(project) = self.projects.active_id() else {
            return cx.notify();
        };
        let result = PlanDraftRepository::list_for_project(store(cx), project).and_then(|drafts| {
            drafts
                .iter()
                .filter(|draft| draft.published_at().is_none())
                .try_for_each(|draft| PlanDraftRepository::delete(store(cx), draft.id()))
        });
        if let Err(err) = result {
            self.status = Some(format!("Could not discard the plan: {err}"));
        }
        cx.notify();
    }
}

/// The project page's card for an unfinished plan: its title and state, with Open and Discard. Nothing is drawn
/// when there is no unfinished plan, or while the planning window is open.
pub(super) fn card(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    if view.plan_wizard.is_some() {
        return None;
    }
    let draft = view.unfinished_plan(cx)?;
    let title = draft.title();

    let actions = if view.plan_discard {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child("Discard this plan?"),
            )
            .child(
                button("plan-discard-keep", "Keep", false, theme).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.plan_discard = false;
                        cx.notify();
                    },
                )),
            )
            .child(
                button("plan-discard-confirm", "Discard", true, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.discard_unfinished_plan(cx)),
                ),
            )
    } else {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .child(
                button("plan-open", "Open", true, theme).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| this.open_unfinished_plan(window, cx),
                )),
            )
            .child(
                button("plan-discard", "Discard", false, theme).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.plan_discard = true;
                        cx.notify();
                    },
                )),
            )
    };

    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(theme.spacing.md)
            .p(theme.spacing.md)
            .rounded(theme.radius.lg)
            .border_1()
            .border_color(theme.palette.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(theme.typography.caption)
                            .text_color(theme.palette.text_secondary)
                            .child("Unfinished plan"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(theme.typography.caption)
                            .text_color(theme.palette.text_secondary)
                            .child(format!(
                                "{} · updated {}",
                                state_label(&draft),
                                describe_time(draft.updated_at())
                            )),
                    ),
            )
            .child(actions)
            .into_any_element(),
    )
}
