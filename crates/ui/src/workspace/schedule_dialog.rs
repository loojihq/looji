//! The working hours dialog: a project's workdays and daily hours, with the next five due dates for a
//! sample five-hour task. It opens from the Hours button in the header and owns the keyboard while open.
//!
//! Edits go to a draft. Nothing is stored until Save, and Save runs the same checks the store runs. Cancel,
//! Escape, and a click outside the dialog discard the draft, after a prompt when there are unsaved changes.
//! The dialog is modal, so the project can't change while it's open.
//!
//! Existing due dates don't change when the hours do. The dialog says so, because a date set under the old
//! hours stays put.

use gpui::{prelude::*, *};
use looji_domain::{
    DraftIssue, DueDate, LocalMoment, PREVIEW_COUNT, ProjectId, SAMPLE_ESTIMATE_HOURS,
    ScheduleDraft, TIME_FORMAT_MESSAGE, Timestamp, WorkingSchedule, format_time, parse_time,
    preview_due_dates, weekday,
};
use looji_persistence::ScheduleSaveError;

use super::fields::{field, text_box};
use super::task_dialog::button;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

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
/// A time is at most `HH:MM` with the hour `24`, so five characters.
const TIME_CHARS: usize = 5;
const COPY: &str = "Existing due dates don't change. The new hours apply to tasks that move into progress from now on.";

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
    /// The times as typed. The draft only takes a time that reads, so a partial entry stays here.
    pub start_text: String,
    pub end_text: String,
    pub start_unreadable: bool,
    pub end_unreadable: bool,
    /// The moment the preview counts from.
    pub anchor: LocalMoment,
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
    /// Opens a draft copied from the stored schedule. `anchor` is passed in, so tests can fix it.
    pub fn open(project: ProjectId, stored: WorkingSchedule, anchor: LocalMoment) -> Self {
        let draft = ScheduleDraft::from_schedule(&stored);
        let mut dialog = Self {
            project,
            stored: draft,
            draft,
            start_text: format_time(stored.start()),
            end_text: format_time(stored.end()),
            start_unreadable: false,
            end_unreadable: false,
            anchor,
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

    /// Takes a start time only if it reads. Anything else leaves the last valid time in the draft.
    pub fn set_start_text(&mut self, text: String) {
        match parse_time(&text) {
            Some(minutes) => {
                self.draft.start = i64::from(minutes);
                self.start_unreadable = false;
            }
            None => self.start_unreadable = true,
        }
        self.start_text = text;
        self.refresh();
    }

    /// Takes an end time only if it reads. See `set_start_text`.
    pub fn set_end_text(&mut self, text: String) {
        match parse_time(&text) {
            Some(minutes) => {
                self.draft.end = i64::from(minutes);
                self.end_unreadable = false;
            }
            None => self.end_unreadable = true,
        }
        self.end_text = text;
        self.refresh();
    }

    /// Whether the draft differs from what's stored, including a time that's typed but doesn't read.
    pub fn dirty(&self) -> bool {
        self.draft != self.stored || self.start_unreadable || self.end_unreadable
    }

    /// Whether Save can be used: the draft breaks no rule and both times read.
    pub fn can_save(&self) -> bool {
        self.issues.is_empty() && !self.start_unreadable && !self.end_unreadable
    }

    /// The messages to show, in order. Empty when the hours can be saved.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.start_unreadable {
            problems.push(format!("Start time: {TIME_FORMAT_MESSAGE}"));
        }
        if self.end_unreadable {
            problems.push(format!("End time: {TIME_FORMAT_MESSAGE}"));
        }
        problems.extend(self.issues.iter().map(|issue| issue.message().to_string()));
        problems
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

/// The text the Hours button shows.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    crate::components::button_icon(
        "schedule-open",
        "clock",
        "Hours",
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_schedule_dialog(window, cx)))
}

impl WorkspaceView {
    /// Opens the dialog for the selected project, with its stored hours in the draft. Nothing opens when no
    /// project is selected.
    pub(super) fn open_schedule_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let anchor = LocalMoment::from_timestamp_utc(Timestamp::now());
        self.schedule_dialog = Some(ScheduleDialog::open(
            project.id(),
            project.working_schedule(),
            anchor,
        ));
        // The form shows under Working hours on the Preferences screen.
        self.open_preferences(cx);
        self.schedule_focus[0].focus(window);
        cx.notify();
    }

    /// Closes the dialog and gives the keyboard back to the workspace. Discards the draft.
    fn close_schedule_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_dialog = None;
        self.focus_handle.focus(window);
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

    fn focus_schedule_control(&self, control: Control, window: &mut Window) {
        if let Some(index) = CONTROLS.iter().position(|candidate| *candidate == control) {
            self.schedule_focus[index].focus(window);
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
        self.schedule_focus[next].focus(window);
        cx.notify();
    }

    fn toggle_schedule_day(&mut self, day: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.schedule_dialog.as_mut() {
            dialog.toggle_day(day);
        }
        self.focus_schedule_control(Control::Day(day), window);
        cx.notify();
    }

    /// Types into a time. Only digits and a colon are taken, and the text is kept to the length of a time.
    fn edit_schedule_time(
        &mut self,
        control: Control,
        keystroke: &Keystroke,
        modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.schedule_dialog.as_mut() else {
            return;
        };
        let current = match control {
            Control::Start => dialog.start_text.clone(),
            Control::End => dialog.end_text.clone(),
            _ => return,
        };
        let mut next = current;
        if keystroke.key == "backspace" {
            next.pop();
        } else if !modifier
            && let Some(typed) = typed_text(keystroke)
            && typed.chars().all(|c| c.is_ascii_digit() || c == ':')
            && next.chars().count() + typed.chars().count() <= TIME_CHARS
        {
            next.push_str(typed);
        } else {
            return;
        }
        match control {
            Control::Start => dialog.set_start_text(next),
            Control::End => dialog.set_end_text(next),
            _ => {}
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

    /// Keys while the dialog is open. Escape closes, Tab moves between controls, Space or Enter toggles a
    /// day, and typing edits a time.
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
        let modifiers = event.keystroke.modifiers;
        match key {
            "escape" => return self.request_schedule_close(window, cx),
            "tab" => return self.move_schedule_focus(modifiers.shift, window, cx),
            _ => {}
        }
        let modifier = modifiers.control || modifiers.alt || modifiers.platform;
        match self.focused_schedule_control(window) {
            Some(Control::Start | Control::End) if key == "enter" => {
                self.save_schedule_dialog(window, cx)
            }
            Some(control @ (Control::Start | Control::End)) => {
                self.edit_schedule_time(control, &event.keystroke, modifier, cx)
            }
            _ => {}
        }
    }
}

/// A day of the week as a pill. The tick and the label say whether it's a workday; the fill and the focus ring
/// repeat that, so colour isn't the only sign.
fn day_pill(
    day: usize,
    selected: bool,
    focused: bool,
    handle: &FocusHandle,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let label = if selected {
        format!("✓ {}", DAY_NAMES[day])
    } else {
        DAY_NAMES[day].to_string()
    };
    div()
        .id(DAY_IDS[day])
        .track_focus(handle)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if focused {
            theme.palette.accent
        } else {
            theme.palette.border_control
        })
        .text_size(theme.typography.caption)
        .cursor_pointer()
        .when(selected, |pill| {
            pill.bg(theme.palette.surface_selected)
                .text_color(theme.palette.text_primary)
        })
        .when(!selected, |pill| {
            pill.text_color(theme.palette.text_secondary)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.toggle_schedule_day(day, window, cx)
        }))
}

/// A time box for the start or the end. Clicking it gives it the keyboard, and a time that doesn't read shows
/// the format message.
fn time_box(
    control: Control,
    value: &str,
    unreadable: bool,
    focused: bool,
    handle: &FocusHandle,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let (label, id) = match control {
        Control::Start => ("Start", "schedule-start"),
        _ => ("End", "schedule-end"),
    };
    field(
        label,
        unreadable.then_some(TIME_FORMAT_MESSAGE),
        div()
            .id(id)
            .track_focus(handle)
            .child(text_box(value, "HH:MM", focused, theme))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.focus_schedule_control(control, window);
                cx.notify();
            })),
        theme,
    )
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
    let project = view
        .projects
        .find_by_id(dialog.project)
        .map(|project| project.name().to_string())
        .unwrap_or_default();

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
            &dialog.start_text,
            dialog.start_unreadable,
            focused(Control::Start),
            handle_for(focus, Control::Start),
            theme,
            cx,
        ))
        .child(time_box(
            Control::End,
            &dialog.end_text,
            dialog.end_unreadable,
            focused(Control::End),
            handle_for(focus, Control::End),
            theme,
            cx,
        ));

    // Read from the view state, which the dialog wrote when the draft last changed. Nothing is calculated here.
    let preview = (!dialog.preview.is_empty()).then(|| {
        let mut list = div().flex().flex_col().gap(theme.spacing.xs);
        for day in &dialog.preview {
            list = list.child(
                div()
                    .text_size(theme.typography.body)
                    .child(describe_day(*day)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(format!(
                        "Next {PREVIEW_COUNT} due dates for a {SAMPLE_ESTIMATE_HOURS:.0}-hour task"
                    )),
            )
            .child(list)
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
        .p(theme.spacing.lg)
        .bg(theme.palette.surface_raised)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(theme.typography.title)
                .child("Working hours"),
        )
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("{project}. {COPY}")),
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
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.spacing.sm)
                .child(
                    button("schedule-cancel", "Cancel", false, theme)
                        .track_focus(handle_for(focus, Control::Cancel))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.request_schedule_close(window, cx)
                        })),
                )
                .child(save),
        );

    // The panel fills the window. An unsaved-changes prompt is drawn over it, in the same window.
    let mut layer = div().relative().size_full().child(panel);
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
    use super::ScheduleDialog;
    use looji_domain::{DueDate, LocalMoment, ProjectId, ScheduleDraft, Workdays, WorkingSchedule};

    const NO_WORKDAYS_MESSAGE: &str = "Select at least one workday to save.";

    fn anchor() -> LocalMoment {
        // Monday 12 October 2026 at 09:00.
        LocalMoment::new(DueDate::new(2026, 10, 12).unwrap(), 9 * 60)
    }

    fn open(schedule: WorkingSchedule) -> ScheduleDialog {
        ScheduleDialog::open(ProjectId::new(), schedule, anchor())
    }

    #[test]
    fn the_draft_starts_as_a_copy_of_the_stored_hours_and_nothing_is_unsaved() {
        let dialog = open(WorkingSchedule::default());
        assert_eq!(
            dialog.draft,
            ScheduleDraft::from_schedule(&WorkingSchedule::default())
        );
        assert_eq!(dialog.start_text, "09:00");
        assert_eq!(dialog.end_text, "17:00");
        assert!(!dialog.dirty());
        assert!(dialog.can_save());
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
    fn changing_the_hours_changes_the_preview_on_the_same_anchor() {
        // Five hours from 09:00 on Monday finish that day at 14:00, so the first date is Monday. With the day
        // ending at 12:00 there are three hours on Monday, and the rest falls on Tuesday.
        let mut dialog = open(WorkingSchedule::default());
        assert_eq!(dialog.preview[0], DueDate::new(2026, 10, 12).unwrap());

        dialog.set_end_text("12:00".to_string());

        assert_eq!(dialog.preview[0], DueDate::new(2026, 10, 13).unwrap());
    }

    #[test]
    fn a_partial_time_is_not_committed_to_the_draft() {
        let mut dialog = open(WorkingSchedule::default());
        let before = dialog.draft;

        dialog.set_start_text("1".to_string());
        assert_eq!(dialog.draft, before, "\"1\" must not reach the draft");
        assert!(dialog.start_unreadable);
        assert!(!dialog.can_save());

        dialog.set_start_text("1:".to_string());
        assert_eq!(dialog.draft, before, "\"1:\" must not reach the draft");
        assert!(dialog.dirty());
    }

    #[test]
    fn a_time_that_reads_is_committed_and_clears_the_unreadable_flag() {
        let mut dialog = open(WorkingSchedule::default());
        dialog.set_start_text("1:".to_string());
        dialog.set_start_text("08:30".to_string());
        assert_eq!(dialog.draft.start, 8 * 60 + 30);
        assert!(!dialog.start_unreadable);
        assert!(dialog.can_save());
    }

    #[test]
    fn a_start_equal_to_the_end_blocks_save_with_its_message() {
        let mut dialog = open(WorkingSchedule::default());
        dialog.set_start_text("17:00".to_string());
        assert!(!dialog.can_save());
        assert!(
            dialog
                .problems()
                .contains(&"The start time must be before the end time.".to_string())
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
}
