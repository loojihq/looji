//! The clock and the UTC offset. Preferences sets both for the app, and a project can set its own in its settings.
//! A project's choice wins, then the app's, then the default: 24-hour time at UTC.

use looji_domain::{Clock, LocalMoment, ProjectId, SettingsRepository, Timestamp, UtcOffset};

use super::{ai_settings::project_key, store};
use crate::settings;

/// The offsets the settings offer, in minutes from UTC, from west to east.
const OFFSET_MINUTES: [i32; 14] = [
    -720, -480, -300, -240, -180, 0, 60, 120, 180, 330, 480, 540, 600, 780,
];

pub(super) const PROJECT_CLOCK: &str = "clock";
pub(super) const PROJECT_OFFSET: &str = "utc_offset";

fn project_setting(cx: &gpui::App, project: ProjectId, name: &str) -> Option<String> {
    store(cx)
        .get_setting(&project_key(project, name))
        .ok()
        .flatten()
}

/// The offsets the settings offer.
pub(super) fn offset_choices() -> Vec<UtcOffset> {
    OFFSET_MINUTES
        .iter()
        .filter_map(|minutes| UtcOffset::from_minutes(*minutes))
        .collect()
}

/// The app's clock.
pub(super) fn app_clock(cx: &gpui::App) -> Clock {
    settings::current(cx).clock
}

/// The app's offset from UTC.
pub(super) fn app_offset(cx: &gpui::App) -> UtcOffset {
    settings::current(cx).utc_offset
}

/// The clock a project shows: its own choice, then the app's.
pub(super) fn clock_for(cx: &gpui::App, project: Option<ProjectId>) -> Clock {
    project
        .and_then(|project| project_clock_choice(cx, project))
        .unwrap_or_else(|| app_clock(cx))
}

/// The offset a project shows times in: its own choice, then the app's.
pub(super) fn offset_for(cx: &gpui::App, project: Option<ProjectId>) -> UtcOffset {
    project
        .and_then(|project| project_offset_choice(cx, project))
        .unwrap_or_else(|| app_offset(cx))
}

/// The current date and clock time in the project's offset, for the calculations that count from today.
pub(super) fn local_now(cx: &gpui::App, project: Option<ProjectId>) -> LocalMoment {
    let offset = offset_for(cx, project);
    let shifted = Timestamp::now().unix_millis() + i64::from(offset.minutes()) * 60_000;
    LocalMoment::from_timestamp_utc(Timestamp::from_unix_millis(shifted))
}

/// A project's own clock, if it has one.
pub(super) fn project_clock_choice(cx: &gpui::App, project: ProjectId) -> Option<Clock> {
    project_setting(cx, project, PROJECT_CLOCK).and_then(|text| Clock::from_stored(&text))
}

/// A project's own offset, if it has one.
pub(super) fn project_offset_choice(cx: &gpui::App, project: ProjectId) -> Option<UtcOffset> {
    project_setting(cx, project, PROJECT_OFFSET).and_then(|text| UtcOffset::from_stored(&text))
}
