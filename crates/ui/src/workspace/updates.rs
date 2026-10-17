//! Checks GitHub for a newer looji when the app starts, and then every ten minutes while it runs. The check itself only
//! happens when the saved schedule says it's due: a successful check is good for a day, and after a failure the next
//! attempt waits an hour. A failed check is never shown to the user. When a newer release exists, a small notice
//! appears in the corner, and the user can open the release page or dismiss it.

use std::time::Duration;

use gpui::{prelude::*, *};
use looji_domain::{SettingsRepository, Timestamp};
use looji_updates::{Schedule, UpdateStatus, is_due, latest_release_url};

use super::{WorkspaceView, store, task_dialog::button};
use crate::theme::Theme;

/// How often the app looks at the schedule while it runs.
const LOOP_TICK: Duration = Duration::from_secs(600);
const SUCCESS_KEY: &str = "update.last_success_ms";
const FAILURE_KEY: &str = "update.last_failure_ms";

impl WorkspaceView {
    /// Starts the checks. The first one happens at once, if the schedule says it's due.
    pub(super) fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            loop {
                let due = this
                    .update(cx, |view, cx| view.begin_update_check(cx))
                    .ok()
                    .flatten();
                if let Some((url, current)) = due {
                    let outcome = cx
                        .background_executor()
                        .spawn(async move {
                            looji_ai::runtime().block_on(looji_updates::check(&url, &current))
                        })
                        .await;
                    let _ = this.update(cx, |view, cx| view.finish_update_check(outcome, cx));
                }
                cx.background_executor().timer(LOOP_TICK).await;
            }
        });
        self.update_task = Some(task);
    }

    /// The address and the running version, when a check is due now. Nothing is returned when it isn't.
    fn begin_update_check(&mut self, cx: &mut Context<Self>) -> Option<(String, String)> {
        let saved = |key: &str| {
            store(cx)
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|value| value.parse::<i64>().ok())
        };
        let schedule = Schedule {
            last_success_ms: saved(SUCCESS_KEY),
            last_failure_ms: saved(FAILURE_KEY),
        };
        if !is_due(schedule, Timestamp::now().unix_millis()) {
            return None;
        }
        Some((update_url(), env!("CARGO_PKG_VERSION").to_string()))
    }

    fn finish_update_check(
        &mut self,
        outcome: Result<UpdateStatus, looji_updates::CheckError>,
        cx: &mut Context<Self>,
    ) {
        let now = Timestamp::now().unix_millis().to_string();
        match outcome {
            Ok(status) => {
                let _ = store(cx).set_setting(SUCCESS_KEY, &now);
                let _ = store(cx).remove_setting(FAILURE_KEY);
                self.update_notice = match status {
                    UpdateStatus::Available { .. } => Some(status),
                    UpdateStatus::UpToDate => None,
                };
            }
            Err(err) => {
                // Kept out of the UI: the check is tried again after an hour, and the reason goes to the log.
                eprintln!("looji: update check failed: {err}");
                let _ = store(cx).set_setting(FAILURE_KEY, &now);
            }
        }
        cx.notify();
    }

    fn dismiss_update_notice(&mut self, cx: &mut Context<Self>) {
        self.update_notice = None;
        cx.notify();
    }
}

/// Where the check asks. A debug build can point it at a local server with `LOOJI_UPDATE_URL`, so the notice can
/// be checked without a real release.
fn update_url() -> String {
    #[cfg(debug_assertions)]
    if let Some(url) = std::env::var_os("LOOJI_UPDATE_URL") {
        return url.to_string_lossy().into_owned();
    }
    latest_release_url()
}

/// The notice in the corner, when a newer release is known. It doesn't block anything.
pub(super) fn notice(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let UpdateStatus::Available { version, url } = view.update_notice.as_ref()? else {
        return None;
    };
    let url = url.clone();
    Some(
        div()
            .id("update-notice")
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .px(theme.spacing.md)
            .py(theme.spacing.sm)
            .rounded(theme.radius.lg)
            .bg(theme.palette.surface_raised)
            .shadow(theme.elevation.overlay.clone())
            .border_1()
            .border_color(theme.palette.border)
            .child(
                div()
                    .text_size(theme.typography.caption)
                    .child(format!("looji {version} is available.")),
            )
            .child(
                button("update-open", "View release", true, theme)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&url))),
            )
            .child(button("update-dismiss", "Dismiss", false, theme).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_update_notice(cx)),
            ))
            .into_any_element(),
    )
}
