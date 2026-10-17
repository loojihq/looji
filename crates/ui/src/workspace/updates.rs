//! Checks GitHub for a newer looji when the app starts, and then every ten minutes while it runs. The check itself only
//! happens when the saved schedule says it's due: a successful check is good for a day, and after a failure the next
//! attempt waits an hour. A failed check is never shown to the user. When a newer release exists, a small notice
//! appears in the corner, and the user can open the release page or dismiss it.

use std::time::Duration;

use gpui::{prelude::*, *};
use looji_domain::{SettingsRepository, Timestamp};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use looji_updates::{Schedule, UpdateStatus, download::Progress, is_due, latest_release_url};

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
        // A debug build can skip the schedule, so the notice can be checked without waiting a day.
        let forced = cfg!(debug_assertions) && std::env::var_os("LOOJI_UPDATE_NOW").is_some();
        if !forced && !is_due(schedule, Timestamp::now().unix_millis()) {
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

    /// Downloads the update, checks it, and swaps it in. The download runs in the background and its progress is
    /// read on a timer. A file that fails its checksum is deleted and never run. If the swap fails, this version
    /// keeps running and says why.
    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(UpdateStatus::Available {
            artifact: Some(artifact),
            ..
        }) = self.update_notice.clone()
        else {
            return;
        };
        if matches!(
            self.update_install,
            Some(Install::Downloading(_) | Install::Applying)
        ) {
            return;
        }
        let folder = cx
            .global::<crate::database::AppDatabase>()
            .database_path
            .parent()
            .map(|parent| parent.join("updates"))
            .unwrap_or_else(|| std::path::PathBuf::from("updates"));
        let start = Progress {
            downloaded: 0,
            total: artifact.size,
        };
        self.update_install = Some(Install::Downloading(start));
        cx.notify();

        let shared = Arc::new(Mutex::new(start));
        let finished = Arc::new(AtomicBool::new(false));
        let (reader, done) = (Arc::clone(&shared), Arc::clone(&finished));
        let ticker = cx.spawn(async move |this, cx| {
            while !done.load(Ordering::Relaxed) {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;
                let progress = reader.lock().map(|progress| *progress).unwrap_or(start);
                let _ = this.update(cx, |view, cx| {
                    if matches!(view.update_install, Some(Install::Downloading(_))) {
                        view.update_install = Some(Install::Downloading(progress));
                        cx.notify();
                    }
                });
            }
        });
        let worker = cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    looji_ai::runtime().block_on(looji_updates::download::download(
                        &artifact,
                        &folder,
                        |progress| {
                            if let Ok(mut shared) = shared.lock() {
                                *shared = progress;
                            }
                        },
                    ))
                })
                .await;
            finished.store(true, Ordering::Relaxed);
            let _ = this.update(cx, |view, cx| view.finish_download(outcome, cx));
            drop(ticker);
        });
        self.update_worker = Some(worker);
    }

    fn finish_download(
        &mut self,
        outcome: Result<std::path::PathBuf, looji_updates::download::DownloadError>,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(staged) => self.apply_update(staged, cx),
            Err(err) => {
                self.update_install = Some(Install::Failed(err.to_string()));
                cx.notify();
            }
        }
    }

    /// Closes what is open, swaps the verified file in for this program, and starts it. The new program waits for
    /// this one to exit before it opens the database. If the swap or the start fails, the previous program is put
    /// back and keeps running.
    fn apply_update(&mut self, staged: std::path::PathBuf, cx: &mut Context<Self>) {
        self.update_install = Some(Install::Applying);
        // A reply still streaming is stopped first, so what arrived is saved before the app closes.
        self.close_chat(cx);
        cx.notify();
        let target = match std::env::current_exe() {
            Ok(target) => target,
            Err(err) => {
                self.update_install = Some(Install::Failed(err.to_string()));
                return cx.notify();
            }
        };
        let args = vec!["--after-update".to_string(), std::process::id().to_string()];
        match looji_updates::apply::replace_and_start(&target, &staged, &args) {
            Ok(_) => cx.quit(),
            Err(err) => {
                self.update_install = Some(Install::Failed(err.to_string()));
                cx.notify();
            }
        }
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

/// Where the one-click update has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Install {
    Downloading(Progress),
    Applying,
    Failed(String),
}

fn progress_text(progress: Progress) -> String {
    match progress.total {
        Some(total) if total > 0 => format!(
            "Downloading the update… {}%",
            progress.downloaded * 100 / total
        ),
        _ => format!(
            "Downloading the update… {} MB",
            progress.downloaded / (1024 * 1024)
        ),
    }
}

/// The notice in the corner, when a newer release is known. It doesn't block anything.
pub(super) fn notice(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let UpdateStatus::Available {
        version,
        url,
        artifact,
    } = view.update_notice.as_ref()?
    else {
        return None;
    };
    let url = url.clone();
    let can_install = artifact.is_some();
    let busy = matches!(
        view.update_install,
        Some(Install::Downloading(_) | Install::Applying)
    );
    let failed = matches!(view.update_install, Some(Install::Failed(_)));
    let message = match &view.update_install {
        None => format!("looji {version} is available."),
        Some(Install::Downloading(progress)) => progress_text(*progress),
        Some(Install::Applying) => "Installing the update. looji will restart.".to_string(),
        Some(Install::Failed(reason)) => format!("The update didn't install: {reason}"),
    };

    let mut row = div()
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
        .max_w(px(640.))
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .shadow(theme.elevation.overlay.clone())
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(theme.typography.caption)
                .when(failed, |text| text.text_color(theme.palette.danger))
                .child(message),
        );
    if !busy {
        if can_install {
            let label = if failed { "Try again" } else { "Update now" };
            row = row.child(
                button("update-install", label, true, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.install_update(cx))),
            );
        }
        row = row
            .child(
                button("update-open", "View release", !can_install, theme)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&url))),
            )
            .child(button("update-dismiss", "Dismiss", false, theme).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_update_notice(cx)),
            ));
    }
    Some(row.into_any_element())
}
