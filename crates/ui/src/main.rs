// Release builds are GUI-only; debug builds keep a console for logs (same as Zed).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod theme;
mod title_bar;
mod workspace;

use gpui::{App, Application};
use looji_persistence::{self as persistence, SqliteStore};

use database::AppDatabase;
use workspace::{WorkspaceList, WorkspaceView};

pub const APP_NAME: &str = "Looji";

fn main() {
    let Some(data_dir) = persistence::data_dir() else {
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

    Application::new().run(move |cx: &mut App| {
        theme::init(cx);
        cx.set_global(AppDatabase {
            store,
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
