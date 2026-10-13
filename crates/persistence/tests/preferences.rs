use looji_domain::{
    DefaultView, Preference, Preferences, PreferredTheme, UpdateChannel, preferences,
};
use looji_persistence::{Connection, SqliteStore, open_database};
use tempfile::TempDir;

fn database(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("looji.db")
}

#[test]
fn a_fresh_database_loads_every_default() {
    let store = SqliteStore::open_in_memory().unwrap();

    let loaded = preferences::load(&store).unwrap();

    assert_eq!(loaded, Preferences::default());
}

#[test]
fn choices_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    {
        let store = SqliteStore::open(&path).unwrap();
        preferences::save(&store, &Preference::Theme(PreferredTheme::Light)).unwrap();
        preferences::save(&store, &Preference::DefaultView(DefaultView::Roadmap)).unwrap();
        preferences::save(&store, &Preference::UpdateChannel(UpdateChannel::Beta)).unwrap();
        preferences::save(
            &store,
            &Preference::SelectedModel(Some("claude-sonnet-5-5".into())),
        )
        .unwrap();
    }

    // Reopen the same file, as a relaunch would.
    let store = SqliteStore::open(&path).unwrap();
    let loaded = preferences::load(&store).unwrap();

    assert_eq!(loaded.theme, PreferredTheme::Light);
    assert_eq!(loaded.default_view, DefaultView::Roadmap);
    assert_eq!(loaded.update_channel, UpdateChannel::Beta);
    assert_eq!(loaded.selected_model.as_deref(), Some("claude-sonnet-5-5"));
}

#[test]
fn an_unknown_key_from_a_newer_version_is_ignored_and_kept() {
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    {
        let connection = open_database(&path).unwrap();
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES ('future_setting', 'from v9')",
                [],
            )
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    let loaded = preferences::load(&store).unwrap();
    preferences::save(&store, &Preference::Theme(PreferredTheme::Light)).unwrap();
    drop(store);

    assert_eq!(loaded, Preferences::default());
    let connection = Connection::open(&path).unwrap();
    let kept: String = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'future_setting'",
            [],
            |row| row.get(0),
        )
        .expect("the newer version's row must survive a save");
    assert_eq!(kept, "from v9");
}

#[test]
fn an_invalid_stored_value_falls_back_to_its_default_without_an_error() {
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    {
        let connection = open_database(&path).unwrap();
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES ('theme', 'solarized')",
                [],
            )
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    let loaded = preferences::load(&store);

    assert_eq!(loaded.unwrap().theme, PreferredTheme::Dark);
}

#[test]
fn defaults_are_not_written_on_load() {
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    let store = SqliteStore::open(&path).unwrap();

    preferences::load(&store).unwrap();

    let connection = Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key IN ('theme', 'default_view', 'update_channel', 'selected_model')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn clearing_the_model_restores_the_default_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    {
        let store = SqliteStore::open(&path).unwrap();
        preferences::save(&store, &Preference::SelectedModel(Some("m".into()))).unwrap();
        preferences::save(&store, &Preference::SelectedModel(None)).unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(preferences::load(&store).unwrap().selected_model, None);
}

#[test]
fn an_existing_database_keeps_its_settings_when_reopened_after_upgrade() {
    // Settings written by an earlier build are still there after the app reopens the file,
    // which runs the migrations again. The workspace's active id is one of those rows.
    let dir = TempDir::new().unwrap();
    let path = database(&dir);
    {
        let store = SqliteStore::open(&path).unwrap();
        preferences::save(&store, &Preference::DefaultView(DefaultView::List)).unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();

    assert_eq!(
        preferences::load(&store).unwrap().default_view,
        DefaultView::List
    );
}
