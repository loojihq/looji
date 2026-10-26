//! The working hours dialog: a project's workdays and daily hours, with a preview of when a sample task would be
//! due. Start and end are picked from a list of times, in half hours, on the clock the project shows.
//!
//! Edits go to a draft. Nothing is stored until Save, and Save runs the same checks the store runs. Cancel,
//! Escape, and a click outside the dialog discard the draft, after a prompt when there are unsaved changes.
//!
//! Existing due dates don't change when the hours do. The dialog says so, because a date set under the old
//! hours stays put.

use gpui::{prelude::*, *};
use looji_domain::{
    Clock, DraftIssue, DueDate, LocalMoment, PREVIEW_COUNT, ProjectId, SAMPLE_ESTIMATE_HOURS,
    ScheduleDraft, WorkingSchedule, format_clock_time, preview_due_dates, weekday,
};
use looji_persistence::ScheduleSaveError;

use super::fields::field;
use super::task_dialog::button;
use super::{WorkspaceView, store};
use crate::{components, theme::Theme};

/// Times are offered in steps of this many minutes.
const TIME_STEP: u16 = 30;

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DAY_IDS: [&str; 7] = [
    "schedule-day-mon",
    "schedule-day-tue",
    "schedule-day-wed",
    "schedule-day-thu",
    "schedule-day-fri",
    "schedule-day-sat",
    "schedule-day-sun",
];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub(super) const COPY: &str = "Existing due dates don't change. The new hours apply to tasks that move into progress from now on.";

/// The controls the keyboard reaches, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Day(usize),
    Start,
    End,
    Save,
    Cancel,
}

const CONTROLS: [Control; 11] = [
    Control::Day(0),
    Control::Day(1),
    Control::Day(2),
    Control::Day(3),
    Control::Day(4),
    Control::Day(5),
    Control::Day(6),
    Control::Start,
    Control::End,
    Control::Save,
    Control::Cancel,
];

/// One focus handle per control, in the order of `CONTROLS`.
pub(super) const CONTROL_COUNT: usize = CONTROLS.len();

/// An open working hours dialog. The draft is what the user is editing. `stored` is what the project had
/// when the dialog opened, and the difference between the two is what's unsaved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScheduleDialog {
    pub project: ProjectId,
    pub stored: ScheduleDraft,
    pub draft: ScheduleDraft,
    /// The moment the preview counts from.
    pub anchor: LocalMoment,
    /// The clock the times are shown on.
    pub clock: Clock,
    /// The time box whose list of times is open, if one is.
    pub picker: Option<Control>,
    /// The rules the draft breaks, read from the draft on every change.
    pub issues: Vec<DraftIssue>,
    /// The next due dates for the sample task. Empty while the draft breaks a rule.
    pub preview: Vec<DueDate>,
    /// A save that failed for a reason other than a rule.
    pub error: Option<String>,
    /// The discard prompt is showing.
    pub confirming_discard: bool,
}

impl ScheduleDialog {
    /// Opens a draft copied from the stored schedule. `anchor` is passed in, so tests can fix it, and `clock` is how
    /// the times are shown.
    pub fn open(
        project: ProjectId,
        stored: WorkingSchedule,
        anchor: LocalMoment,
        clock: Clock,
    ) -> Self {
        let draft = ScheduleDraft::from_schedule(&stored);
        let mut dialog = Self {
            project,
            stored: draft,
            draft,
            anchor,
            clock,
            picker: None,
            issues: Vec::new(),
            preview: Vec::new(),
            error: None,
            confirming_discard: false,
        };
        dialog.refresh();
        dialog
    }

    /// Adds or removes one weekday. Days past Sunday are ignored.
    pub fn toggle_day(&mut self, day: usize) {
        if day < 7 {
            self.draft.workdays ^= 1 << day;
            self.refresh();
        }
    }

    /// Sets the start to a time from the list, and closes the list.
    pub fn set_start(&mut self, minutes: u16) {
        self.draft.start = i64::from(minutes);
        self.picker = None;
        self.refresh();
    }

    /// Sets the end to a time from the list, and closes the list.
    pub fn set_end(&mut self, minutes: u16) {
        self.draft.end = i64::from(minutes);
        self.picker = None;
        self.refresh();
    }

    /// Whether the draft differs from what's stored.
    pub fn dirty(&self) -> bool {
        self.draft != self.stored
    }

    /// Whether Save can be used: the draft breaks no rule.
    pub fn can_save(&self) -> bool {
        self.issues.is_empty()
    }

    /// The messages to show, in order. Empty when the hours can be saved.
    pub fn problems(&self) -> Vec<String> {
        self.issues
            .iter()
            .map(|issue| issue.message().to_string())
            .collect()
    }

    /// Reads the draft again: the rules it breaks, and the preview if it has none. This is the only place the
    /// calculator runs for the dialog. Rendering reads what this leaves behind.
    fn refresh(&mut self) {
        match self.draft.to_schedule() {
            Ok(schedule) => {
                self.issues.clear();
                // The schedule has just been checked, and the sample estimate is a positive number of hours, so
                // the calculator can't refuse it. A refusal would leave the preview empty.
                self.preview =
                    preview_due_dates(&schedule, self.anchor, PREVIEW_COUNT).unwrap_or_default();
            }
            Err(issues) => {
                self.issues = issues;
                self.preview.clear();
            }
        }
    }
}

impl WorkspaceView {
    /// Closes the dialog and gives the keyboard back to the workspace. Discards the draft.
    fn close_schedule_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_dialog = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Closes at once when nothing is unsaved. Otherwise shows the discard prompt, which keeps editing by default.
    pub(super) fn request_schedule_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.schedule_dialog.as_mut() else {
            return;
        };
        if dialog.dirty() {
            dialog.confirming_discard = true;
            cx.notify();
        } else {
            self.close_schedule_dialog(window, cx);
        }
    }

    fn keep_editing_schedule(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            dialog.confirming_discard = false;
        }
        cx.notify();
    }

    /// The control that has the keyboard, if one of the dialog's controls does.
    fn focused_schedule_control(&self, window: &Window) -> Option<Control> {
        CONTROLS
            .iter()
            .zip(&self.schedule_focus)
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(control, _)| *control)
    }

    fn focus_schedule_control(&self, control: Control, window: &mut Window, cx: &mut App) {
        if let Some(index) = CONTROLS.iter().position(|candidate| *candidate == control) {
            self.schedule_focus[index].focus(window, cx);
        }
    }

    /// Moves the keyboard to the next control, or the previous one with Shift. It wraps, so Tab never leaves the dialog.
    fn move_schedule_focus(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.schedule_focus.len();
        let current = self
            .schedule_focus
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        let next = if backwards {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.schedule_focus[next].focus(window, cx);
        cx.notify();
    }

    fn toggle_schedule_day(&mut self, day: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            dialog.toggle_day(day);
        }
        self.focus_schedule_control(Control::Day(day), window, cx);
        cx.notify();
    }

    /// Opens the list of times for one time box, or closes it when it's already open.
    fn toggle_time_picker(
        &mut self,
        control: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_schedule_control(control, window, cx);
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            dialog.picker = if dialog.picker == Some(control) {
                None
            } else {
                Some(control)
            };
        }
        cx.notify();
    }

    /// Closes the list of times, if one is open.
    pub(super) fn close_time_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut()
            && dialog.picker.take().is_some()
        {
            cx.notify();
        }
    }

    /// Picks a time from the list for one time box.
    fn choose_schedule_time(&mut self, control: Control, minutes: u16, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            match control {
                Control::Start => dialog.set_start(minutes),
                _ => dialog.set_end(minutes),
            }
        }
        cx.notify();
    }

    fn set_schedule_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            dialog.error = Some(message);
        }
        cx.notify();
    }

    /// Saves the draft. The store checks it again, so a draft that skipped the screen is still refused. On
    /// success the project in memory takes the new hours and the dialog closes.
    fn save_schedule_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.schedule_dialog.as_ref() else {
            return;
        };
        if !dialog.can_save() {
            return;
        }
        let (project, draft) = (dialog.project, dialog.draft);
        match store(cx).save_working_schedule(project, &draft) {
            Ok(schedule) => {
                if let Some(mut updated) = self.projects.find_by_id(project).cloned() {
                    updated.set_working_schedule(schedule);
                    self.projects.replace(updated);
                }
                self.close_schedule_dialog(window, cx);
            }
            Err(ScheduleSaveError::Invalid(issues)) => {
                if let Some(dialog) = self.schedule_dialog.as_mut() {
                    dialog.issues = issues;
                    dialog.preview.clear();
                }
                cx.notify();
            }
            Err(ScheduleSaveError::ProjectNotFound) => {
                self.set_schedule_error("That project no longer exists.".to_string(), cx);
            }
            Err(err) => {
                eprintln!("could not save working hours: {err}");
                self.set_schedule_error(
                    "Could not save the working hours. Try again.".to_string(),
                    cx,
                );
            }
        }
    }

    /// Keys while the dialog is open. Escape closes the list of times first and then the dialog, Tab moves between
    /// controls, and Space or Enter opens a time's list or toggles a day.
    pub(super) fn schedule_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let Some(dialog) = self.schedule_dialog.as_ref() else {
            return;
        };
        if dialog.confirming_discard {
            // Keep editing is the default, so Enter and Escape both keep the draft.
            if matches!(key, "enter" | "escape") {
                self.keep_editing_schedule(cx);
            }
            return;
        }
        let picking = dialog.picker.is_some();
        let shift = event.keystroke.modifiers.shift;
        match key {
            "escape" if picking => return self.close_time_picker(cx),
            "escape" => return self.request_schedule_close(window, cx),
            "tab" => return self.move_schedule_focus(shift, window, cx),
            _ => {}
        }
        match self.focused_schedule_control(window) {
            Some(control @ (Control::Start | Control::End)) if matches!(key, "enter" | "space") => {
                self.toggle_time_picker(control, window, cx)
            }
            Some(Control::Day(day)) if matches!(key, "enter" | "space") => {
                self.toggle_schedule_day(day, window, cx)
            }
            Some(Control::Save) if key == "enter" => self.save_schedule_dialog(window, cx),
            _ => {}
        }
    }
}

/// A day of the week as a square: filled when it's a workday, outlined when it isn't. There's no tick, so the
/// state is the fill alone.
fn day_pill(
    day: usize,
    selected: bool,
    focused: bool,
    handle: &FocusHandle,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(DAY_IDS[day])
        .track_focus(handle)
        .flex()
        .items_center()
        .justify_center()
        .w(px(48.))
        .h(px(36.))
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if focused {
            theme.palette.accent
        } else {
            theme.palette.border
        })
        .text_size(theme.typography.caption)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(selected, |pill| {
            pill.bg(theme.palette.accent)
                .text_color(theme.palette.on_accent)
        })
        .when(!selected, |pill| {
            pill.bg(theme.palette.surface)
                .text_color(theme.palette.text_secondary)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(DAY_NAMES[day])
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.toggle_schedule_day(day, window, cx)
        }))
}

/// The times a time box offers, in order. A start can be any step before midnight, and an end any step after
/// the start of the day, up to 24:00.
pub(super) fn time_choices(control: Control) -> Vec<u16> {
    match control {
        Control::End => (TIME_STEP..=24 * 60)
            .step_by(usize::from(TIME_STEP))
            .collect(),
        _ => (0..24 * 60).step_by(usize::from(TIME_STEP)).collect(),
    }
}

/// A time box for the start or the end. It shows the time, and a click opens the list of times to pick from.
fn time_box(
    control: Control,
    dialog: &ScheduleDialog,
    handle: &FocusHandle,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let (label, id, value) = match control {
        Control::Start => ("Start", "schedule-start", dialog.draft.start),
        _ => ("End", "schedule-end", dialog.draft.end),
    };
    let minutes = u16::try_from(value).unwrap_or(0);
    let open = dialog.picker == Some(control);

    let trigger = components::select(id, &format_clock_time(minutes, dialog.clock), open, theme)
        .w_full()
        .track_focus(handle)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.toggle_time_picker(control, window, cx)
        }));
    let mut wrapper = div()
        .relative()
        .w(px(150.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);
    if open {
        let rows: Vec<AnyElement> = time_choices(control)
            .into_iter()
            .enumerate()
            .map(|(index, choice)| {
                components::dropdown_row(
                    format!("{id}-time-{index}"),
                    &format_clock_time(choice, dialog.clock),
                    choice == minutes,
                    false,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_schedule_time(control, choice, cx)
                }))
                .into_any_element()
            })
            .collect();
        wrapper = wrapper.child(deferred(components::dropdown_menu(
            format!("{id}-menu"),
            rows,
            theme,
        )));
    }
    field(label, None, wrapper, theme)
}

/// The handle of one control, found by its place in `CONTROLS`.
fn handle_for(focus: &[FocusHandle], control: Control) -> &FocusHandle {
    let index = CONTROLS
        .iter()
        .position(|candidate| *candidate == control)
        .expect("every control is in CONTROLS");
    &focus[index]
}

/// The day a due date falls on, in words: "Mon 19 Oct 2026".
fn describe_day(day: DueDate) -> String {
    format!(
        "{} {} {} {}",
        DAY_NAMES[weekday(day)],
        day.day(),
        MONTH_NAMES[(day.month() - 1) as usize],
        day.year()
    )
}

/// The working hours dialog, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.schedule_dialog.as_ref()?;
    let focus = &view.schedule_focus;
    let focused = |control: Control| {
        CONTROLS
            .iter()
            .position(|candidate| *candidate == control)
            .is_some_and(|index| focus[index].is_focused(window))
    };

    let mut pills = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
    for (day, handle) in focus.iter().enumerate().take(DAY_NAMES.len()) {
        pills = pills.child(day_pill(
            day,
            dialog.draft.workdays & (1 << day) != 0,
            focused(Control::Day(day)),
            handle,
            theme,
            cx,
        ));
    }
    let days = pills;

    let times = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.md)
        .child(time_box(
            Control::Start,
            dialog,
            handle_for(focus, Control::Start),
            theme,
            cx,
        ))
        .child(time_box(
            Control::End,
            dialog,
            handle_for(focus, Control::End),
            theme,
            cx,
        ));

    // One sentence, from the view state the dialog wrote when the draft last changed. The list of dates is gone:
    // the first one is enough to show what the hours do.
    let preview = dialog.preview.first().map(|day| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(format!(
                "A {SAMPLE_ESTIMATE_HOURS:.0}-hour task that starts today is due {}.",
                describe_day(*day)
            ))
    });
    let problems = dialog.problems();
    let blocked = !dialog.can_save();
    let save_focus = handle_for(focus, Control::Save);
    let save = button("schedule-save", "Save", true, theme).track_focus(save_focus);
    let save = if blocked {
        save.opacity(0.5).cursor_default()
    } else {
        save.on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.save_schedule_dialog(window, cx)),
        )
    };

    let panel = div()
        .id("schedule-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w_full()
        .h_full()
        .p(theme.spacing.lg)
        .bg(theme.palette.surface_raised)
        .on_click(|_, _, cx| cx.stop_propagation())
        // A press outside the time boxes closes an open list of times.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_time_picker(cx)),
        )
        .child(field("Workdays", None, days, theme))
        .child(times)
        .when_some(preview, |panel, preview| panel.child(preview))
        .children(problems.into_iter().map(|message| {
            div()
                .text_size(theme.typography.body)
                .text_color(theme.palette.danger)
                .child(message)
        }))
        .when(dialog.dirty(), |panel| {
            panel.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child("Unsaved changes"),
            )
        })
        .when_some(dialog.error.clone(), |panel, message| {
            panel.child(
                div()
                    .text_size(theme.typography.body)
                    .text_color(theme.palette.danger)
                    .child(message),
            )
        })
        // The buttons sit at the bottom of the section, wherever the hours end.
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.spacing.sm)
                .child(
                    button("schedule-discard", "Discard changes", false, theme)
                        .when(!dialog.dirty(), |button| {
                            button.opacity(0.5).cursor_default()
                        })
                        .when(dialog.dirty(), |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.reset_working_hours(window, cx)
                            }))
                        }),
                )
                .child(save),
        );

    // The panel fills the window. An unsaved-changes prompt is drawn over it, in the same window.
    let mut layer = div().relative().flex_1().min_h_0().w_full().child(panel);
    if dialog.confirming_discard {
        layer = layer.child(discard_prompt(theme, cx));
    }
    Some(layer.into_any_element())
}

/// The prompt that stands between the user and losing unsaved hours. Keep editing is the default.
fn discard_prompt(theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    div()
        .id("schedule-discard-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .child(
            div()
                .id("schedule-discard-panel")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .w(px(380.))
                .p(theme.spacing.lg)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .shadow(theme.elevation.overlay.clone())
                .border_1()
                .border_color(theme.palette.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(div().child("Discard the unsaved working hours?"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(
                            button("schedule-keep", "Keep editing", true, theme).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.keep_editing_schedule(cx)
                                }),
                            ),
                        )
                        .child(
                            button("schedule-discard", "Discard", false, theme).on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.close_schedule_dialog(window, cx)
                                }),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{Control, ScheduleDialog, time_choices};
    use looji_domain::{
        Clock, DueDate, LocalMoment, ProjectId, ScheduleDraft, Workdays, WorkingSchedule,
    };

    const NO_WORKDAYS_MESSAGE: &str = "Select at least one workday to save.";
    const START_BEFORE_END_MESSAGE: &str = "The start time must be before the end time.";

    fn anchor() -> LocalMoment {
        // Monday 12 October 2026 at 09:00.
        LocalMoment::new(DueDate::new(2026, 10, 12).unwrap(), 9 * 60)
    }

    fn open(schedule: WorkingSchedule) -> ScheduleDialog {
        ScheduleDialog::open(ProjectId::new(), schedule, anchor(), Clock::TwentyFour)
    }

    #[test]
    fn the_draft_starts_as_a_copy_of_the_stored_hours_and_nothing_is_unsaved() {
        let dialog = open(WorkingSchedule::default());
        assert_eq!(
            dialog.draft,
            ScheduleDraft::from_schedule(&WorkingSchedule::default())
        );
        assert!(!dialog.dirty());
        assert!(dialog.can_save());
        assert_eq!(dialog.picker, None);
    }

    #[test]
    fn toggling_every_day_off_shows_the_message_and_blocks_save() {
        let mut dialog = open(WorkingSchedule::default());
        for day in 0..5 {
            dialog.toggle_day(day);
        }
        assert_eq!(dialog.problems(), vec![NO_WORKDAYS_MESSAGE.to_string()]);
        assert!(!dialog.can_save());
        assert!(
            dialog.preview.is_empty(),
            "the calculation is skipped for an invalid draft"
        );
    }

    #[test]
    fn toggling_a_day_back_on_makes_the_draft_saveable_again() {
        let mut dialog = open(WorkingSchedule::default());
        for day in 0..5 {
            dialog.toggle_day(day);
        }
        dialog.toggle_day(0);
        assert!(dialog.can_save());
        assert_eq!(dialog.preview.len(), 5);
    }

    #[test]
    fn a_picked_end_changes_the_preview_on_the_same_anchor() {
        // Five hours from 09:00 on Monday finish that day at 14:00, so the first date is Monday. With the day
        // ending at 12:00 there are three hours on Monday, and the rest falls on Tuesday.
        let mut dialog = open(WorkingSchedule::default());
        assert_eq!(dialog.preview[0], DueDate::new(2026, 10, 12).unwrap());

        dialog.set_end(12 * 60);

        assert_eq!(dialog.preview[0], DueDate::new(2026, 10, 13).unwrap());
    }

    #[test]
    fn a_picked_time_is_saved_to_the_draft_and_closes_the_list() {
        let mut dialog = open(WorkingSchedule::default());
        dialog.picker = Some(Control::Start);
        dialog.set_start(8 * 60 + 30);
        assert_eq!(dialog.draft.start, 8 * 60 + 30);
        assert_eq!(dialog.picker, None);
        assert!(dialog.dirty());
    }

    #[test]
    fn a_start_equal_to_the_end_blocks_save_with_its_message() {
        let mut dialog = open(WorkingSchedule::default());
        dialog.set_start(17 * 60);
        assert!(!dialog.can_save());
        assert!(
            dialog
                .problems()
                .contains(&START_BEFORE_END_MESSAGE.to_string())
        );
    }

    #[test]
    fn the_draft_is_unsaved_once_it_differs_and_clean_again_when_it_matches() {
        let mut dialog = open(WorkingSchedule::default());
        dialog.toggle_day(5);
        assert!(dialog.dirty());
        dialog.toggle_day(5);
        assert!(
            !dialog.dirty(),
            "a draft that matches the stored hours isn't unsaved"
        );
    }

    #[test]
    fn the_preview_for_a_valid_draft_is_the_calculator_output() {
        // The dialog's preview and the calculator are the same function, so they agree for the same inputs.
        let dialog = open(WorkingSchedule::default());
        let schedule =
            WorkingSchedule::try_new(9 * 60, 17 * 60, Workdays::from_mask(0b0001_1111).unwrap())
                .unwrap();
        let expected = looji_domain::preview_due_dates(&schedule, anchor(), 5).unwrap();
        assert_eq!(dialog.preview, expected);
    }

    #[test]
    fn a_start_can_be_any_half_hour_before_midnight_and_an_end_can_reach_midnight() {
        let starts = time_choices(Control::Start);
        assert_eq!(starts.first(), Some(&0));
        assert_eq!(starts.last(), Some(&(23 * 60 + 30)));
        let ends = time_choices(Control::End);
        assert_eq!(ends.first(), Some(&30));
        assert_eq!(ends.last(), Some(&(24 * 60)));
        assert!(starts.iter().chain(&ends).all(|minute| minute % 30 == 0));
    }
}
