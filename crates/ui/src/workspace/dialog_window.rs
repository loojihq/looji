//! The dialogs that still open in their own window, as JLC's do. Smaller ones, such as creating a
//! project, deleting a workspace, and the team, now show inline on their screen.
//!
//! The window draws its dialog from the main window's state, and its keys go to the main window's key
//! handler, so the dialog behaves as it always did. The system's title bar is hidden, and the window draws
//! its own header with the dialog's name and a close button, as JLC's dialog windows do. Each window is sized
//! to its dialog. Closing the window closes the dialog, and the dialog closing (Done, Cancel, or Escape)
//! closes the window.

use gpui::{prelude::*, *};

use super::WorkspaceView;
use super::{chat, plan_wizard, preferences_panel, task_dialog, task_form::Mode};
use crate::{components, theme::ActiveTheme, title_bar::TITLE_BAR_HEIGHT, window_move};

/// Which dialog a window shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Task,
    TaskDetail,
    Chat,
    Plan,
    Preferences,
}

impl Kind {
    /// The header's title. The task and chat dialogs name the task they are about.
    fn title(self, view: &WorkspaceView) -> String {
        match self {
            Kind::Task => match view.task_dialog.as_ref().map(|dialog| &dialog.form.mode) {
                Some(Mode::Edit(_)) => "Edit task".to_string(),
                _ => "New task".to_string(),
            },
            Kind::TaskDetail => view
                .panel_task()
                .map(|task| task.title().to_string())
                .unwrap_or_else(|| "Task".to_string()),
            Kind::Preferences => "Preferences".to_string(),
            Kind::Plan => "Generate with AI".to_string(),
            Kind::Chat => match view.chat_dialog.as_ref() {
                Some(dialog) => format!("Chat: {}", dialog.title),
                None => "Chat".to_string(),
            },
        }
    }

    /// The window's size: the dialog's panel plus the body's padding, so the dialog fits without empty
    /// space around it.
    fn size(self) -> Size<Pixels> {
        match self {
            Kind::Task => size(px(552.), px(520.)),
            Kind::TaskDetail => size(px(560.), px(720.)),
            Kind::Chat => size(px(680.), px(640.)),
            Kind::Preferences => size(px(720.), px(680.)),
            Kind::Plan => size(px(1040.), px(800.)),
        }
    }

    /// Whether the window can be resized. The planning window grows with its review.
    fn resizable(self) -> bool {
        matches!(self, Kind::Plan)
    }

    /// Whether this dialog is open in the main window's state.
    fn is_open(self, view: &WorkspaceView) -> bool {
        match self {
            Kind::Task => view.task_dialog.is_some(),
            Kind::TaskDetail => view.panel.is_some(),
            Kind::Chat => view.chat_dialog.is_some(),
            Kind::Preferences => view.preferences_open,
            Kind::Plan => view.plan_wizard.is_some(),
        }
    }

    /// Clears this dialog's state in the main window, for when its window closes by its own button.
    fn clear(self, view: &mut WorkspaceView) {
        match self {
            Kind::Task => view.task_dialog = None,
            Kind::TaskDetail => view.panel = None,
            Kind::Chat => view.chat_dialog = None,
            Kind::Preferences => view.preferences_open = false,
            Kind::Plan => view.plan_wizard = None,
        }
    }

    /// Closes the dialog the way its own Cancel or Done does, so unsaved changes are still asked about.
    fn request_close(
        self,
        view: &mut WorkspaceView,
        window: &mut Window,
        cx: &mut Context<WorkspaceView>,
    ) {
        match self {
            Kind::Task => view.request_task_dialog_close(cx),
            Kind::TaskDetail => {
                // The window closes itself once the panel is gone, so the card's focus stays put.
                view.panel = None;
                cx.notify();
            }
            Kind::Chat => view.close_chat(cx),
            Kind::Preferences => view.close_preferences(cx),
            Kind::Plan => view.request_plan_close(window, cx),
        }
    }

    /// The dialog's panel, drawn from the main window's state.
    fn panel(
        self,
        view: &WorkspaceView,
        theme: &crate::theme::Theme,
        window: &Window,
        cx: &mut Context<WorkspaceView>,
    ) -> Option<AnyElement> {
        match self {
            Kind::Task => task_dialog::panel(view, theme, cx),
            Kind::TaskDetail => super::panel::render(view, theme, cx),
            Kind::Chat => chat::panel(view, theme, window, cx),
            Kind::Plan => plan_wizard::panel(view, theme, window, cx),
            Kind::Preferences => Some(preferences_panel::render(view, theme, window, cx)),
        }
    }
}

/// The window's view. It holds the main window's entity, and everything it draws comes from there.
pub(super) struct DialogWindow {
    main: Entity<WorkspaceView>,
    kind: Kind,
    focus: FocusHandle,
}

/// Opens a dialog in its own window. The main window's state already says which dialog is open. Call this
/// deferred from the main window's update, so the new window doesn't read the main window while it is busy.
pub(super) fn open(cx: &mut App, main: Entity<WorkspaceView>, kind: Kind) {
    let title = kind.title(main.read(cx));
    // A dialog sits centred over the window that opened it, and that window waits until it closes.
    let size = kind.size();
    let bounds = match cx
        .active_window()
        .and_then(|parent| parent.update(cx, |_, window, _| window.bounds()).ok())
    {
        Some(parent) => Bounds {
            origin: point(
                parent.origin.x + (parent.size.width - size.width) / 2.,
                parent.origin.y + (parent.size.height - size.height) / 2.,
            ),
            size,
        },
        None => Bounds::centered(None, size, cx),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        kind: WindowKind::Dialog,
        is_minimizable: false,
        // Created hidden and shown once its first frame is drawn, so the frame and borders never appear empty.
        show: false,
        titlebar: Some(TitlebarOptions {
            title: Some(title.clone().into()),
            // Hides the system's title bar. The window draws its own header.
            appears_transparent: true,
            traffic_light_position: None,
        }),
        is_resizable: kind.resizable(),
        ..Default::default()
    };

    let opened = cx.open_window(options, move |window, cx| {
        let view = cx.new(|cx| {
            // The window redraws when the main window does. It closes when the dialog is closed there.
            cx.observe_in(&main, window, move |_, main, window, cx| {
                if kind.is_open(main.read(cx)) {
                    cx.notify();
                } else {
                    window.remove_window();
                }
            })
            .detach();
            // Closing the window by its own button closes the dialog in the main window.
            let closed = main.clone();
            cx.on_release(move |_, cx| {
                closed.update(cx, |view, cx| {
                    kind.clear(view);
                    cx.notify();
                });
            })
            .detach();
            // A dialog that had another dialog open over it takes the keyboard back when that one closes.
            cx.observe_window_activation(window, |this: &mut DialogWindow, window, cx| {
                if window.is_window_active() {
                    window.focus(&this.focus, cx);
                }
            })
            .detach();
            DialogWindow {
                main,
                kind,
                focus: cx.focus_handle(),
            }
        });
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        view
    });
    match opened {
        // The window has drawn once by now. Showing it puts the finished frame on screen.
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(_) => eprintln!("could not open the {title} window"),
    }
}

impl Render for DialogWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let kind = self.kind;
        let content = self
            .main
            .update(cx, |view, cx| kind.panel(view, &theme, window, cx));
        let title = kind.title(self.main.read(cx));

        // The header drags the window, like a title bar. Its close button is the one control on it.
        let header = div()
            .id("dialog-header")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .flex_none()
            .h(TITLE_BAR_HEIGHT)
            .pl(theme.spacing.md)
            .pr(px(6.))
            .bg(theme.palette.surface_raised)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                window_move::drag_from(event, window)
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(theme.typography.body)
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                components::icon_button("dialog-close", "x", &theme)
                    // The close button keeps its press, so the header doesn't start a drag from it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        let kind = this.kind;
                        this.main
                            .clone()
                            .update(cx, |view, cx| kind.request_close(view, window, cx));
                    })),
            );

        div()
            .id("dialog-window")
            .track_focus(&self.focus)
            // Keys go to the main window's handler, which routes them to whichever dialog is open.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let main = this.main.clone();
                main.update(cx, |view, cx| view.on_key_down(event, window, cx));
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.palette.surface_raised)
            .text_color(theme.palette.text_primary)
            .text_size(theme.typography.body)
            .child(header)
            .child(
                div()
                    .id("dialog-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(content),
            )
    }
}
