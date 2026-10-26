//! User preferences: typed values, the keys they're stored under, and their defaults.
//!
//! Stored keys and stored values are a public contract. Renaming a key or a value strands
//! choices people have already saved, so treat them as permanent once shipped.
//!
//! Only explicit choices are written. Defaults are never saved, so a changed default reaches
//! everyone who hasn't chosen a value.

use crate::{Clock, RepoResult, SettingsRepository, UtcOffset};

/// Stored keys.
pub mod keys {
    pub const THEME: &str = "theme";
    pub const DEFAULT_VIEW: &str = "default_view";
    pub const SELECTED_MODEL: &str = "selected_model";
    pub const UPDATE_CHANNEL: &str = "update_channel";
    pub const NOTIFY_DUE_TODAY: &str = "notify_due_today";
    pub const NOTIFY_OVERDUE: &str = "notify_overdue";
    pub const CLOCK: &str = "clock";
    pub const UTC_OFFSET: &str = "utc_offset";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    Light,
    #[default]
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefaultView {
    #[default]
    Board,
    List,
    Roadmap,
    Workload,
}

impl DefaultView {
    pub fn as_str(self) -> &'static str {
        match self {
            DefaultView::Board => "board",
            DefaultView::List => "list",
            DefaultView::Roadmap => "roadmap",
            DefaultView::Workload => "workload",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "board" => Some(DefaultView::Board),
            "list" => Some(DefaultView::List),
            "roadmap" => Some(DefaultView::Roadmap),
            "workload" => Some(DefaultView::Workload),
            _ => None,
        }
    }
}

/// Which builds to offer. Anything unrecognised is read as `Stable`, never a guessed channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "stable",
            UpdateChannel::Beta => "beta",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "stable" => Some(UpdateChannel::Stable),
            "beta" => Some(UpdateChannel::Beta),
            _ => None,
        }
    }
}

/// Every preference, with defaults for anything not chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub theme: Theme,
    /// The fallback view. A project with its own saved view ignores this.
    pub default_view: DefaultView,
    pub update_channel: UpdateChannel,
    /// A provider model ID. `None` means the feature picks its own default, and the model list
    /// is not checked here.
    pub selected_model: Option<String>,
    /// Whether the sidebar's Today entry counts tasks due today.
    pub notify_due_today: bool,
    /// Whether the sidebar's Today entry counts tasks that are overdue.
    pub notify_overdue: bool,
    /// How times are shown and typed.
    pub clock: Clock,
    /// The offset from UTC that times are shown in.
    pub utc_offset: UtcOffset,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            default_view: DefaultView::default(),
            update_channel: UpdateChannel::default(),
            selected_model: None,
            // Both reminders start on, so a new install shows what's due without a setting.
            notify_due_today: true,
            notify_overdue: true,
            clock: Clock::default(),
            utc_offset: UtcOffset::UTC,
        }
    }
}

/// Reads an on or off setting. A missing or unrecognised value keeps the default, which is on.
fn parse_flag(text: Option<&str>) -> bool {
    !matches!(text, Some("off"))
}

fn flag_text(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// One preference to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preference {
    Theme(Theme),
    DefaultView(DefaultView),
    UpdateChannel(UpdateChannel),
    /// `None` clears the choice, so the default applies again.
    SelectedModel(Option<String>),
    NotifyDueToday(bool),
    NotifyOverdue(bool),
    Clock(Clock),
    UtcOffset(UtcOffset),
}

impl Preferences {
    /// Applies a change to memory. The caller writes it to storage with [`save`].
    pub fn apply(&mut self, preference: &Preference) {
        match preference {
            Preference::Theme(theme) => self.theme = *theme,
            Preference::DefaultView(view) => self.default_view = *view,
            Preference::UpdateChannel(channel) => self.update_channel = *channel,
            Preference::SelectedModel(model) => self.selected_model = model.clone(),
            Preference::NotifyDueToday(on) => self.notify_due_today = *on,
            Preference::NotifyOverdue(on) => self.notify_overdue = *on,
            Preference::Clock(clock) => self.clock = *clock,
            Preference::UtcOffset(offset) => self.utc_offset = *offset,
        }
    }
}

/// Reads the known keys only. Unknown keys are never looked at, so a newer version's keys are
/// ignored. A missing or unrecognised value falls back to its default. Nothing is written.
pub fn load<S: SettingsRepository>(settings: &S) -> RepoResult<Preferences> {
    let theme = settings.get_setting(keys::THEME)?;
    let view = settings.get_setting(keys::DEFAULT_VIEW)?;
    let channel = settings.get_setting(keys::UPDATE_CHANNEL)?;
    let model = settings.get_setting(keys::SELECTED_MODEL)?;
    let due_today = settings.get_setting(keys::NOTIFY_DUE_TODAY)?;
    let overdue = settings.get_setting(keys::NOTIFY_OVERDUE)?;
    let clock = settings.get_setting(keys::CLOCK)?;
    let offset = settings.get_setting(keys::UTC_OFFSET)?;

    Ok(Preferences {
        theme: theme.as_deref().and_then(Theme::parse).unwrap_or_default(),
        default_view: view
            .as_deref()
            .and_then(DefaultView::parse)
            .unwrap_or_default(),
        update_channel: channel
            .as_deref()
            .and_then(UpdateChannel::parse)
            .unwrap_or_default(),
        selected_model: model
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty()),
        notify_due_today: parse_flag(due_today.as_deref()),
        notify_overdue: parse_flag(overdue.as_deref()),
        clock: clock
            .as_deref()
            .and_then(Clock::from_stored)
            .unwrap_or_default(),
        utc_offset: offset
            .as_deref()
            .and_then(UtcOffset::from_stored)
            .unwrap_or(UtcOffset::UTC),
    })
}

/// Writes one preference. Rows for other keys, including unknown ones, are left alone.
pub fn save<S: SettingsRepository>(settings: &S, preference: &Preference) -> RepoResult<()> {
    match preference {
        Preference::Theme(theme) => settings.set_setting(keys::THEME, theme.as_str()),
        Preference::DefaultView(view) => settings.set_setting(keys::DEFAULT_VIEW, view.as_str()),
        Preference::UpdateChannel(channel) => {
            settings.set_setting(keys::UPDATE_CHANNEL, channel.as_str())
        }
        Preference::SelectedModel(Some(id)) => settings.set_setting(keys::SELECTED_MODEL, id),
        Preference::SelectedModel(None) => settings.remove_setting(keys::SELECTED_MODEL),
        Preference::NotifyDueToday(on) => {
            settings.set_setting(keys::NOTIFY_DUE_TODAY, flag_text(*on))
        }
        Preference::NotifyOverdue(on) => settings.set_setting(keys::NOTIFY_OVERDUE, flag_text(*on)),
        Preference::Clock(clock) => settings.set_setting(keys::CLOCK, clock.stored()),
        Preference::UtcOffset(offset) => settings.set_setting(keys::UTC_OFFSET, &offset.stored()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{RepoResult as Result, WorkspaceId};

    /// An in-memory settings store, enough to exercise the rules without SQLite.
    #[derive(Default)]
    struct Map(std::cell::RefCell<HashMap<String, String>>);

    impl SettingsRepository for Map {
        fn active_workspace(&self) -> Result<Option<WorkspaceId>> {
            Ok(None)
        }
        fn set_active_workspace(&self, _: WorkspaceId) -> Result<()> {
            Ok(())
        }
        fn get_setting(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn set_setting(&self, key: &str, value: &str) -> Result<()> {
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn remove_setting(&self, key: &str) -> Result<()> {
            self.0.borrow_mut().remove(key);
            Ok(())
        }
    }

    #[test]
    fn missing_keys_load_as_defaults() {
        assert_eq!(load(&Map::default()).unwrap(), Preferences::default());
    }

    #[test]
    fn unrecognised_values_load_as_defaults() {
        let map = Map::default();
        map.set_setting(keys::THEME, "solarized").unwrap();
        map.set_setting(keys::UPDATE_CHANNEL, "nightly").unwrap();
        map.set_setting(keys::DEFAULT_VIEW, "").unwrap();

        let loaded = load(&map).unwrap();

        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.update_channel, UpdateChannel::Stable);
        assert_eq!(loaded.default_view, DefaultView::Board);
    }

    #[test]
    fn unknown_keys_are_ignored_and_left_in_place() {
        let map = Map::default();
        map.set_setting("future_setting", "whatever").unwrap();

        load(&map).unwrap();
        save(&map, &Preference::Theme(Theme::Light)).unwrap();

        assert_eq!(
            map.get_setting("future_setting").unwrap().as_deref(),
            Some("whatever")
        );
    }

    #[test]
    fn saved_choices_load_back() {
        let map = Map::default();
        save(&map, &Preference::Theme(Theme::Light)).unwrap();
        save(&map, &Preference::DefaultView(DefaultView::Roadmap)).unwrap();
        save(&map, &Preference::UpdateChannel(UpdateChannel::Beta)).unwrap();
        save(
            &map,
            &Preference::SelectedModel(Some("claude-sonnet-5-5".into())),
        )
        .unwrap();

        let loaded = load(&map).unwrap();

        assert_eq!(loaded.theme, Theme::Light);
        assert_eq!(loaded.default_view, DefaultView::Roadmap);
        assert_eq!(loaded.update_channel, UpdateChannel::Beta);
        assert_eq!(loaded.selected_model.as_deref(), Some("claude-sonnet-5-5"));
    }

    #[test]
    fn clearing_the_model_removes_the_row() {
        let map = Map::default();
        save(&map, &Preference::SelectedModel(Some("m".into()))).unwrap();
        save(&map, &Preference::SelectedModel(None)).unwrap();

        assert_eq!(map.get_setting(keys::SELECTED_MODEL).unwrap(), None);
    }

    #[test]
    fn loading_writes_nothing() {
        let map = Map::default();
        load(&map).unwrap();
        assert!(map.0.borrow().is_empty(), "defaults must not be persisted");
    }

    #[test]
    fn every_stored_value_round_trips_through_its_parser() {
        for theme in [Theme::Light, Theme::Dark] {
            assert_eq!(Theme::parse(theme.as_str()), Some(theme));
        }
        for view in [
            DefaultView::Board,
            DefaultView::List,
            DefaultView::Roadmap,
            DefaultView::Workload,
        ] {
            assert_eq!(DefaultView::parse(view.as_str()), Some(view));
        }
        for channel in [UpdateChannel::Stable, UpdateChannel::Beta] {
            assert_eq!(UpdateChannel::parse(channel.as_str()), Some(channel));
        }
    }
}
