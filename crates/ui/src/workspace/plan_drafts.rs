//! The planning drafts of the selected project: a list with the title, the state, and when each was last updated, and
//! the actions for each draft. It opens from the Drafts button in the header.
//!
//! A ready draft opens in the review. A draft that was interrupted or failed resumes, which starts a new generation
//! from its goal. A published plan is read-only, so it opens in the review with its editing turned off. Duplicate
//! copies a plan, or a goal for an unfinished draft. Delete asks first, and it never touches the backlog. The list
//! only ever shows the selected project's drafts, so a project that's been deleted has none to show.

use gpui::{prelude::*, *};
use looji_domain::{
    DraftState, LocalMoment, PlanDraft, PlanDraftId, PlanDraftRepository, ProjectId, Timestamp,
};

use super::task_dialog::button;
use super::{WorkspaceView, store};
use crate::theme::Theme;

/// The list's state: the drafts as they were last read, and the delete waiting for an answer.
#[derive(Debug, Clone)]
pub(super) struct PlanDraftsDialog {
    pub project: ProjectId,
    pub drafts: Vec<PlanDraft>,
    pub pending_delete: Option<PlanDraftId>,
    pub notice: Option<String>,
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// When a draft was last updated, in UTC, to the minute.
fn describe_time(at: Timestamp) -> String {
    let moment = LocalMoment::from_timestamp_utc(at);
    format!(
        "{} {:02}:{:02} UTC",
        moment.day,
        moment.minute / 60,
        moment.minute % 60
    )
}

/// The state a person reads. A published plan is shown as published, not as ready, since it can't change.
fn state_label(draft: &PlanDraft) -> &'static str {
    match draft.state() {
        DraftState::Generating => "Generating",
        DraftState::Interrupted => "Interrupted",
        DraftState::Failed => "Failed",
        DraftState::Ready if draft.published_at().is_some() => "Published",
        DraftState::Ready => "Ready",
    }
}

impl WorkspaceView {
    /// Opens the list for the selected project. Nothing opens when no project is selected.
    pub(super) fn open_plan_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        self.plan_drafts = Some(PlanDraftsDialog {
            project,
            drafts: Vec::new(),
            pending_delete: None,
            notice: None,
        });
        self.reload_plan_drafts(cx);
    }

    pub(super) fn close_plan_drafts(&mut self, cx: &mut Context<Self>) {
        self.plan_drafts = None;
        cx.notify();
    }

    /// Closes the list when its project is no longer the selected one, so a deleted project's drafts can't be acted on.
    pub(super) fn close_plan_drafts_if_gone(&mut self) {
        let active = self.projects.active_id();
        if self
            .plan_drafts
            .as_ref()
            .is_some_and(|dialog| Some(dialog.project) != active)
        {
            self.plan_drafts = None;
        }
    }

    /// Reads the project's drafts, newest update first.
    fn reload_plan_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.plan_drafts.as_ref().map(|dialog| dialog.project) else {
            return;
        };
        let loaded = PlanDraftRepository::list_for_project(store(cx), project);
        let Some(dialog) = self.plan_drafts.as_mut() else {
            return;
        };
        match loaded {
            Ok(mut drafts) => {
                drafts.sort_by_key(|draft| std::cmp::Reverse(draft.updated_at()));
                dialog.drafts = drafts;
            }
            Err(err) => dialog.notice = Some(format!("Could not load the plans: {err}")),
        }
        cx.notify();
    }

    /// Opens a draft. A ready plan opens in the review, read-only if it's published. An unfinished one resumes.
    pub(super) fn open_plan_draft(
        &mut self,
        id: PlanDraftId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = match PlanDraftRepository::find(store(cx), id) {
            Ok(Some(draft)) => draft,
            Ok(None) => return self.plan_drafts_notice("That plan is no longer stored.", cx),
            Err(err) => {
                return self.plan_drafts_notice(&format!("Could not open the plan: {err}"), cx);
            }
        };
        let state = draft.state();
        if state == DraftState::Generating {
            // Only a request running in this process is generating, and that one is already on screen.
            return self.plan_drafts_notice("That plan is still being generated.", cx);
        }
        self.close_plan_drafts(cx);
        self.open_plan_wizard(window, cx);
        let Some(dialog) = self.plan_wizard.as_mut() else {
            return;
        };
        if state == DraftState::Ready {
            self.focus_handle.focus(window);
            dialog.wizard = looji_domain::PlanWizard::finished(looji_domain::DueDate::today_utc());
            dialog.draft = Some(draft);
            cx.notify();
        } else {
            dialog.interrupted = Some(draft);
            self.resume_interrupted_plan(cx);
        }
    }

    /// Copies a draft. The copy is a new draft of its own, and the original is left as it was.
    pub(super) fn duplicate_plan_draft(&mut self, id: PlanDraftId, cx: &mut Context<Self>) {
        let outcome = PlanDraftRepository::find(store(cx), id).and_then(|found| match found {
            Some(draft) => {
                let copy = draft
                    .duplicate(PlanDraftId::new(), now())
                    .map_err(|err| looji_domain::RepoError::Rejected(err.into()))?;
                PlanDraftRepository::save(store(cx), &copy)?;
                Ok(())
            }
            None => Ok(()),
        });
        match outcome {
            Ok(()) => {
                self.plan_drafts_notice("Copied the plan.", cx);
                self.reload_plan_drafts(cx);
            }
            Err(err) => self.plan_drafts_notice(&format!("Could not copy the plan: {err}"), cx),
        }
    }

    /// Asks before deleting. Nothing is deleted until the answer is yes.
    pub(super) fn ask_delete_plan_draft(&mut self, id: PlanDraftId, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_drafts.as_mut() {
            dialog.pending_delete = Some(id);
        }
        cx.notify();
    }

    pub(super) fn cancel_delete_plan_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_drafts.as_mut() {
            dialog.pending_delete = None;
        }
        cx.notify();
    }

    /// Deletes the draft and its stories and tasks. The backlog isn't touched, so a published plan's tasks stay.
    pub(super) fn confirm_delete_plan_draft(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .plan_drafts
            .as_mut()
            .and_then(|dialog| dialog.pending_delete.take())
        else {
            return;
        };
        match PlanDraftRepository::delete(store(cx), id) {
            Ok(()) => {
                self.plan_drafts_notice("Deleted the plan.", cx);
                self.reload_plan_drafts(cx);
            }
            Err(err) => self.plan_drafts_notice(&format!("Could not delete the plan: {err}"), cx),
        }
    }

    fn plan_drafts_notice(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(dialog) = self.plan_drafts.as_mut() {
            dialog.notice = Some(message.to_string());
        }
        cx.notify();
    }
}

/// The Drafts button in the header.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    button("plan-drafts-open", "Drafts", false, theme)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_plan_drafts(cx)))
}

/// One draft's row: its title, state, and update time, and the actions it offers.
fn row(
    draft: &PlanDraft,
    pending: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let id = draft.id();
    let primary = match draft.state() {
        DraftState::Ready => Some(("Open", true)),
        DraftState::Interrupted | DraftState::Failed => Some(("Resume", true)),
        DraftState::Generating => None,
    };
    let mut actions = div().flex().flex_row().gap(theme.spacing.sm).justify_end();
    if let Some((label, _)) = primary {
        actions = actions.child(
            button(
                ("draft-open", draft.id().as_uuid().as_u128() as usize),
                label,
                true,
                theme,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_plan_draft(id, window, cx)
            })),
        );
    }
    actions = actions.child(
        button(
            ("draft-copy", draft.id().as_uuid().as_u128() as usize),
            "Duplicate",
            false,
            theme,
        )
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| this.duplicate_plan_draft(id, cx)),
        ),
    );
    actions = if pending {
        actions
            .child(
                button(
                    ("draft-keep", draft.id().as_uuid().as_u128() as usize),
                    "Keep",
                    false,
                    theme,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_delete_plan_draft(cx)),
                ),
            )
            .child(
                button(
                    ("draft-confirm", draft.id().as_uuid().as_u128() as usize),
                    "Delete",
                    true,
                    theme,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.confirm_delete_plan_draft(cx)),
                ),
            )
    } else {
        actions.child(
            button(
                ("draft-delete", draft.id().as_uuid().as_u128() as usize),
                "Delete",
                false,
                theme,
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_delete_plan_draft(id, cx)),
            ),
        )
    };

    let mut column = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .child(div().child(draft.title()))
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!(
                    "{} · updated {}",
                    state_label(draft),
                    describe_time(draft.updated_at())
                )),
        );
    if let Some(reason) = draft.failure() {
        column = column.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(reason.to_string()),
        );
    }
    column.child(actions).into_any_element()
}

/// The list, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.plan_drafts.as_ref()?;
    let project = view
        .projects
        .find_by_id(dialog.project)
        .map(|project| project.name().to_string())
        .unwrap_or_default();

    let mut list = div().flex().flex_col().gap(theme.spacing.sm);
    if dialog.drafts.is_empty() {
        list = list.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child("No planning drafts yet. Use Plan to write one."),
        );
    }
    for draft in &dialog.drafts {
        let pending = dialog.pending_delete == Some(draft.id());
        list = list.child(row(draft, pending, theme, cx));
    }

    let panel =
        div()
            .id("plan-drafts-panel")
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
                    .child("Planning drafts"),
            )
            .child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(project),
            )
            .when_some(dialog.notice.clone(), |panel, notice| {
                panel.child(div().text_color(theme.palette.text_secondary).child(notice))
            })
            .child(
                div()
                    .id("plan-drafts-scroll")
                    .max_h(px(520.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(div().flex().flex_row().justify_end().child(
                button("plan-drafts-close", "Close", false, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.close_plan_drafts(cx)),
                ),
            ));

    Some(
        div()
            .id("plan-drafts-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.palette.scrim)
            .occlude()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_plan_drafts(cx)))
            .child(panel)
            .into_any_element(),
    )
}
