//! The first-run tour: a short walk through what looji does, shown once to a new user. It can be skipped at any
//! screen, with the button or Escape. Skipping and finishing are saved alike, and either one keeps the tour from
//! coming back. The words and the rules for when it shows are in the domain.

use gpui::{prelude::*, *};
use looji_domain::{
    Advance, DISMISSED, FINISHED, SettingsRepository, TOUR_SETTING, Tour, should_show,
};

use super::{WorkspaceView, store, task_dialog::button};
use crate::theme::Theme;

impl WorkspaceView {
    /// Starts the tour for a user who has never seen it. Called once, when the app opens.
    pub(super) fn start_tour_if_new(&mut self, cx: &mut Context<Self>) {
        let saved = store(cx).get_setting(TOUR_SETTING).ok().flatten();
        if should_show(saved.as_deref()) {
            self.tour = Some(Tour::start());
        }
    }

    /// Shows the tour from the start, from the sidebar, for someone who wants it again.
    pub(super) fn replay_tour(&mut self, cx: &mut Context<Self>) {
        self.tour = Some(Tour::start());
        cx.notify();
    }

    /// Ends the tour and saves that it was seen, so it doesn't come back.
    fn end_tour(&mut self, finished: bool, cx: &mut Context<Self>) {
        self.tour = None;
        let value = if finished { FINISHED } else { DISMISSED };
        if let Err(err) = store(cx).set_setting(TOUR_SETTING, value) {
            eprintln!("looji: could not save that the tour was seen: {err}");
        }
        cx.notify();
    }

    fn tour_next(&mut self, cx: &mut Context<Self>) {
        let Some(tour) = self.tour.as_mut() else {
            return;
        };
        if tour.advance() == Advance::Finished {
            return self.end_tour(true, cx);
        }
        cx.notify();
    }

    fn tour_back(&mut self, cx: &mut Context<Self>) {
        if let Some(tour) = self.tour.as_mut() {
            tour.retreat();
            cx.notify();
        }
    }

    /// Keys while the tour is open: Escape skips it, and Enter or the right arrow goes on.
    pub(super) fn tour_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.end_tour(false, cx),
            "enter" | "right" => self.tour_next(cx),
            "left" => self.tour_back(cx),
            _ => {}
        }
    }
}

/// The tour, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let tour = view.tour.as_ref()?;
    let step = tour.step();
    let last = tour.is_last();

    let mut buttons = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .child(
            button("tour-skip", "Skip tour", false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.end_tour(false, cx))),
        )
        .child(div().flex_1());
    if !tour.is_first() {
        buttons = buttons.child(
            button("tour-back", "Back", false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.tour_back(cx))),
        );
    }
    buttons = buttons.child(
        button("tour-next", if last { "Done" } else { "Next" }, true, theme)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.tour_next(cx))),
    );

    let panel = div()
        .id("tour-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w(px(440.))
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .shadow(theme.elevation.overlay.clone())
        .border_1()
        .border_color(theme.palette.border)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("{} of {}", tour.position(), tour.count())),
        )
        .child(div().text_size(theme.typography.title).child(step.title))
        .child(
            div()
                .text_color(theme.palette.text_primary)
                .child(step.body),
        )
        .child(buttons);

    Some(
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("tour-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.palette.scrim)
                    .occlude()
                    .child(panel),
            )
            .into_any_element(),
    )
}
