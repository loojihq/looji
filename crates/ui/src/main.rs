// Release builds are GUI-only; debug builds keep a console for logs (same as Zed).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod components;
mod database;
mod settings;
mod theme;
mod title_bar;
mod window_move;
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
    wait_for_previous_version();
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

    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            theme::init(cx, preferences.theme.into());
            cx.set_global(settings::AppPreferences(preferences));
            cx.set_global(AppDatabase {
                store,
                database_path: database_path.clone(),
                icons_dir: icons_dir.clone(),
            });
            quit_when_last_window_closes(cx);

            WorkspaceView::open_window(cx, workspaces).expect("failed to open main window");
            // The window is up, so this upgrade worked. Until now its backup was kept in case it hadn't.
            if let Err(err) = persistence::confirm_startup(&database_path) {
                eprintln!("looji: could not clear the upgrade backup marker: {err}");
            }
            // After an update the old program was kept beside this one in case this one didn't start. It did.
            if let Ok(current) = std::env::current_exe() {
                let _ = looji_updates::apply::remove_previous(&current);
            }
            cx.activate(true);
        });
}

/// A program started by an update is given the old program's process id, and waits for it to exit. Otherwise both would
/// have the database open at once while the old one was still closing it.
fn wait_for_previous_version() {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--after-update"
            && let Some(pid) = args.next().and_then(|pid| pid.parse::<u32>().ok())
        {
            looji_updates::apply::wait_for_exit(pid, std::time::Duration::from_secs(30));
        }
    }
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
