//! Reviews. A task that moves into review is checked by an agent against what the task said it was for: each "Done
//! when" point gets a result with the code that shows it, and anything else worth fixing is listed. The review shows in
//! the task's panel with the choices that follow from it. Looji never moves the task itself. The latest review of each
//! task is saved, so it is still there when the task is opened again.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{prelude::*, *};
use looji_ai::{
    CancellationToken,
    generation::{Update, run_with},
    review::testrun,
    review::verdict::{
        MAX_CHANGES_CHARS, Outcome, Rating, TaskReview, build_review_request, done_when,
        parse_review,
    },
};
use looji_domain::{
    BlueprintRepository, CodeSourceRepository, ProjectId, ReviewHistoryRepository, ReviewRecord,
    SettingsRepository, Status, TaskId,
};

use super::{WorkspaceView, store, task_dialog::button};
use crate::{database::AppDatabase, theme::Theme};

/// How often a running review is checked for new steps.
const TICK: Duration = Duration::from_millis(100);

/// The review of one task: running, finished, or failed.
pub(super) enum ReviewRun {
    Running {
        steps: Vec<String>,
        queue: Arc<Mutex<Vec<Update>>>,
        /// The points the review was asked about, so its reply can be matched to them.
        criteria: Vec<String>,
        cancel: CancellationToken,
        _reader: Task<()>,
    },
    Ready(TaskReview),
    Failed(String),
}

/// All the reviews this session knows about.
pub(super) type Reviews = HashMap<TaskId, ReviewRun>;

fn review_key(task: TaskId) -> String {
    format!("review.{}", task.as_uuid())
}

impl WorkspaceView {
    /// Loads the saved review of a task, unless one is already here.
    pub(super) fn load_review(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if self.reviews.contains_key(&task) {
            return;
        }
        let saved = store(cx).get_setting(&review_key(task)).ok().flatten();
        if let Some(review) = saved.as_deref().and_then(TaskReview::from_json) {
            self.reviews.insert(task, ReviewRun::Ready(review));
        }
    }

    /// Reviews the task now. A review already running for it is replaced. Nothing starts when the project has no
    /// repository, and the panel says so when the review was asked for.
    pub(super) fn start_review(&mut self, task_id: TaskId, asked: bool, cx: &mut Context<Self>) {
        let Some(task) = self.board.iter().find(|task| task.id() == task_id).cloned() else {
            return;
        };
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let Some(root) = store(cx).load_path(project).ok().flatten() else {
            if asked {
                self.reviews.insert(
                    task_id,
                    ReviewRun::Failed(
                        "Connect a repository in the project's settings to review its work."
                            .to_string(),
                    ),
                );
                cx.notify();
            }
            return;
        };
        if let Some(ReviewRun::Running { cancel, .. }) = self.reviews.get(&task_id) {
            cancel.cancel();
        }

        let criteria = done_when(task.description());
        let model = super::ai_settings::model_for(cx, Some(project));
        let provider = super::ai_settings::provider_for(cx, Some(project));
        let knowledge_path = cx.global::<AppDatabase>().knowledge_path.clone();
        let (title, description) = (task.title().to_string(), task.description().to_string());
        let test_command = super::ai_settings::tests_for(cx, project);
        let blueprint = store(cx).blueprint(project).unwrap_or_default();

        let cancel = CancellationToken::new();
        let queue: Arc<Mutex<Vec<Update>>> = Arc::new(Mutex::new(Vec::new()));
        let reader = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_going = this
                    .update(cx, |view, cx| view.read_review(task_id, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });
        self.reviews.insert(
            task_id,
            ReviewRun::Running {
                steps: Vec::new(),
                queue: Arc::clone(&queue),
                criteria: criteria.clone(),
                cancel: cancel.clone(),
                _reader: reader,
            },
        );
        cx.notify();

        spawn_review(
            ReviewJob {
                provider,
                knowledge_path,
                project,
                root,
                model,
                title,
                description,
                criteria,
                test_command,
                blueprint,
            },
            cancel,
            queue,
        );
    }

    /// Takes what the running review has reported. Returns whether to keep reading.
    fn read_review(&mut self, task: TaskId, cx: &mut Context<Self>) -> bool {
        let Some(ReviewRun::Running {
            steps,
            queue,
            criteria,
            ..
        }) = self.reviews.get_mut(&task)
        else {
            return false;
        };
        let updates = match queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        let mut outcome = None;
        for update in updates {
            match update {
                Update::Step(step) => steps.push(step),
                Update::Finished(text) => {
                    outcome = Some(parse_review(&text, criteria));
                }
                Update::Failed(err) => outcome = Some(Err(err.to_string())),
                Update::Text(_) | Update::Source(_) => {}
            }
        }
        let Some(outcome) = outcome else {
            cx.notify();
            return true;
        };
        match outcome {
            Ok(review) => {
                if let Err(err) = store(cx).set_setting(&review_key(task), &review.to_json()) {
                    eprintln!(
                        "could not save the review of task {}: {err}",
                        task.as_uuid()
                    );
                }
                let record = ReviewRecord {
                    outcome: review.outcome.stored().to_string(),
                    summary: review.summary.clone(),
                    review_json: review.to_json(),
                    model: Some(super::ai_settings::model_for(cx, self.projects.active_id())),
                    created_at: looji_domain::Timestamp::now(),
                };
                if let Err(err) = store(cx).add_review(task, &record) {
                    eprintln!(
                        "could not keep the review of task {} in its history: {err}",
                        task.as_uuid()
                    );
                }
                if let Some(panel) = self.panel.as_mut().filter(|panel| panel.task == task) {
                    panel.review_history = store(cx).reviews(task).unwrap_or_default();
                }
                self.reviews.insert(task, ReviewRun::Ready(review));
            }
            Err(message) => {
                self.reviews.insert(task, ReviewRun::Failed(message));
            }
        }
        cx.notify();
        false
    }

    /// Stops a review that is running.
    pub(super) fn stop_review(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(ReviewRun::Running { cancel, .. }) = self.reviews.get(&task) {
            cancel.cancel();
        }
        self.reviews.remove(&task);
        self.load_review(task, cx);
        cx.notify();
    }
}

/// Everything a review needs, gathered before it starts.
struct ReviewJob {
    provider: looji_ai::Provider,
    knowledge_path: std::path::PathBuf,
    project: ProjectId,
    root: String,
    model: String,
    title: String,
    description: String,
    criteria: Vec<String>,
    /// The test command to run first, when the project turned that on.
    test_command: Option<String>,
    /// The project's blueprint, to compare with the code.
    blueprint: Vec<looji_domain::BlueprintDoc>,
}

fn spawn_review(job: ReviewJob, cancel: CancellationToken, queue: Arc<Mutex<Vec<Update>>>) {
    let future = async move {
        let sink = queue;
        let push = move |update: Update| {
            if let Ok(mut queue) = sink.lock() {
                queue.push(update);
            }
        };
        let root = job.root.clone();
        let changes = looji_ai::runtime()
            .spawn_blocking(move || {
                looji_knowledge::git::working_changes(Path::new(&root), MAX_CHANGES_CHARS)
            })
            .await
            .unwrap_or_default();
        let tests = match job.test_command.clone() {
            Some(command) => {
                push(Update::Step(format!("Running {command}")));
                let root = job.root.clone();
                looji_ai::runtime()
                    .spawn_blocking(move || {
                        testrun::run(Path::new(&root), &command, testrun::TIMEOUT).report()
                    })
                    .await
                    .ok()
            }
            None => None,
        };
        if cancel.is_cancelled() {
            return;
        }
        let drift = {
            let (root, docs) = (job.root.clone(), job.blueprint.clone());
            looji_ai::runtime()
                .spawn_blocking(move || super::blueprint::review_drift(Path::new(&root), &docs))
                .await
                .ok()
                .flatten()
        };
        let request = build_review_request(
            &job.model,
            &job.title,
            &job.description,
            &job.criteria,
            &changes,
            tests.as_deref(),
            drift.as_deref(),
        );
        match super::agent::agent_for(
            job.provider,
            &job.knowledge_path,
            job.project,
            Path::new(&job.root),
            false,
        ) {
            Ok(agent) => run_with(agent.as_ref(), request, cancel, push).await,
            Err(err) => push(Update::Failed(err)),
        }
    };
    drop(looji_ai::runtime().spawn(future));
}

/// Whether a move put a task into review from somewhere else.
pub(super) fn entered_review(from: Status, to: Status) -> bool {
    to == Status::InReview && from != Status::InReview
}

/// The review in a task's panel. Nothing is drawn for a task that isn't in review and has no review.
pub(super) fn section(
    view: &WorkspaceView,
    task: TaskId,
    status: Status,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let run = view.reviews.get(&task);
    if run.is_none() && status != Status::InReview {
        return None;
    }
    let caption = |text: String| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(text)
    };
    let mut column = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.md)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child("Review".to_string()),
        );

    let actions = |column: Div, offer_decision: bool, cx: &mut Context<WorkspaceView>| {
        let mut row = div().flex().flex_row().flex_wrap().gap(theme.spacing.sm);
        if offer_decision {
            row = row
                .child(
                    button("review-done", "Move to Done", true, theme).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.set_panel_status(Status::Done, cx),
                    )),
                )
                .child(
                    button("review-back", "Send back", false, theme).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.set_panel_status(Status::InProgress, cx)
                        },
                    )),
                );
        }
        column.child(row.child(
            button("review-again", "Review again", false, theme).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.start_review(task, true, cx)),
            ),
        ))
    };

    match run {
        None => {
            column = column
                .child(caption(
                    "This task is in review. Review it to check the work against what the task asked for."
                        .to_string(),
                ))
                .child(
                    button("review-start", "Review now", true, theme).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.start_review(task, true, cx),
                    )),
                );
        }
        Some(ReviewRun::Running { steps, .. }) => {
            column = column.child(caption("Reviewing the work\u{2026}".to_string()));
            let skipped = steps.len().saturating_sub(4);
            for step in &steps[skipped..] {
                column = column.child(caption(format!("\u{2022} {step}")));
            }
            column = column.child(div().child(
                button("review-stop", "Stop", false, theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.stop_review(task, cx)),
                ),
            ));
        }
        Some(ReviewRun::Failed(message)) => {
            column = column.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message.clone()),
            );
            column = actions(column, false, cx);
        }
        Some(ReviewRun::Ready(review)) => {
            let tone = match review.outcome {
                Outcome::Done => theme.palette.accent,
                Outcome::NeedsWork => theme.palette.danger,
                Outcome::Unclear => theme.palette.text_secondary,
            };
            column = column.child(
                div()
                    .text_color(tone)
                    .font_weight(FontWeight::MEDIUM)
                    .child(review.outcome.label().to_string()),
            );
            if !review.summary.is_empty() {
                column = column.child(div().child(review.summary.clone()));
            }
            for criterion in &review.criteria {
                let (mark, colour) = match criterion.rating {
                    Rating::Met => ("Met", theme.palette.accent),
                    Rating::Partly => ("Partly", theme.palette.text_secondary),
                    Rating::NotMet => ("Not met", theme.palette.danger),
                    Rating::Unclear => ("Unclear", theme.palette.text_secondary),
                };
                column = column.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(theme.spacing.sm)
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(52.))
                                        .text_color(colour)
                                        .child(mark.to_string()),
                                )
                                .child(div().flex_1().min_w_0().child(criterion.text.clone())),
                        )
                        .when(!criterion.evidence.is_empty(), |row| {
                            row.child(div().pl(px(60.)).child(caption(criterion.evidence.clone())))
                        }),
                );
            }
            if !review.findings.is_empty() {
                column = column.child(crate::components::section_label("Also found", theme));
                for finding in &review.findings {
                    let place = finding
                        .location
                        .as_deref()
                        .map(|place| format!(" ({place})"))
                        .unwrap_or_default();
                    column = column.child(div().child(format!(
                        "{}: {}{place}",
                        finding.severity.label(),
                        finding.text
                    )));
                }
            }
            column = actions(column, true, cx);
        }
    }
    // Reviews before the one shown, newest first. The record of the review shown above is left out, found by its
    // content, so a review that could not be kept in the history never hides an earlier one.
    if let Some(panel) = view.panel.as_ref().filter(|panel| panel.task == task) {
        let shown = match run {
            Some(ReviewRun::Ready(review)) => Some(review.to_json()),
            _ => None,
        };
        let earlier: Vec<&ReviewRecord> = panel
            .review_history
            .iter()
            .enumerate()
            .filter(|(index, record)| {
                !(*index == 0 && shown.as_deref() == Some(record.review_json.as_str()))
            })
            .map(|(_, record)| record)
            .take(MAX_EARLIER)
            .collect();
        if !earlier.is_empty() {
            column = column.child(crate::components::section_label("Earlier reviews", theme));
            for record in earlier {
                let verdict =
                    Outcome::from_stored(&record.outcome).map_or("Review", Outcome::label);
                column = column.child(caption(format!(
                    "{} \u{00b7} {verdict}{}",
                    super::panel::format_time(record.created_at),
                    if record.summary.is_empty() {
                        String::new()
                    } else {
                        format!(" \u{2014} {}", record.summary)
                    }
                )));
            }
        }
    }
    Some(column.into_any_element())
}

/// How many earlier reviews the task window lists.
const MAX_EARLIER: usize = 5;
