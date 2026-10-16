// Release builds are GUI-only; debug builds keep a console for logs (same as Zed).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod settings;
mod theme;
mod title_bar;
mod workspace;

use gpui::{App, Application};
use looji_persistence::{self as persistence, SqliteStore};

use database::AppDatabase;
use workspace::{WorkspaceList, WorkspaceView};

pub const APP_NAME: &str = "Looji";

/// The folder the database and icons live in. A debug build reads `LOOJI_DATA_DIR` first, so a manual run can use a
/// scratch folder and leave the real data alone. Release builds always use the user's data folder.
fn data_dir() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("LOOJI_DATA_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    persistence::data_dir()
}

fn main() {
    let Some(data_dir) = data_dir() else {
        fail("looji could not find your user data folder.");
    };
    let database_path = persistence::database_path(&data_dir);
    let icons_dir = persistence::icons::icons_dir(&data_dir);

    // Opening creates the folder and an empty database on first run.
    let store = match SqliteStore::open(&database_path) {
        Ok(store) => store,
        Err(err) => fail(&format!(
            "looji could not open its database at {}.\n\n{err}\n\n\
             If another copy of looji is running, close it and try again.",
            database_path.display()
        )),
    };

    // Loads before the window opens, so the first frame already shows the right workspace.
    let workspaces = match WorkspaceList::load(&store) {
        Ok(list) => list,
        Err(err) => fail(&format!("looji could not load your workspaces.\n\n{err}")),
    };

    // Preferences load before the window too, so the first frame uses the saved theme.
    let preferences = match looji_domain::preferences::load(&store) {
        Ok(preferences) => preferences,
        Err(err) => fail(&format!(
            "looji could not read your settings.

{err}"
        )),
    };

    Application::new().run(move |cx: &mut App| {
        theme::init(cx, preferences.theme.into());
        cx.set_global(settings::AppPreferences(preferences));
        cx.set_global(AppDatabase {
            store,
            database_path: database_path.clone(),
            icons_dir: icons_dir.clone(),
        });
        quit_when_last_window_closes(cx);

        WorkspaceView::open_window(cx, workspaces).expect("failed to open main window");
        cx.activate(true);
    });
}

/// Windows already exits when the last window closes; macOS and Linux don't.
fn quit_when_last_window_closes(cx: &mut App) {
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Reports a startup failure and exits. Release builds have no console, so on
/// Windows the message also goes to a message box.
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    #[cfg(target_os = "windows")]
    show_message_box(message);
    std::process::exit(1)
}

#[cfg(target_os = "windows")]
fn show_message_box(message: &str) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::HSTRING,
    };

    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            &HSTRING::from(APP_NAME),
            MB_OK | MB_ICONERROR,
        );
    }
}
