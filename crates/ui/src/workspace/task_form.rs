//! The state of the create and edit task dialog, without any window. The dialog holds the values
//! being typed, a snapshot of the values it opened with, and one error per field. Dirty means the
//! values differ from the snapshot, computed when asked rather than tracked on each keystroke, so it
//! can't drift.
//!
//! Required: the title. Everything else is optional. Blank estimate and due date mean none.

use looji_domain::{
    DueDate, Estimate, Priority, Task, TaskId, normalize_tags, parse_due_date,
    parse_estimate_hours, validate_title,
};

/// Create a new task, or edit an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Create,
    Edit(TaskId),
}

/// The fields in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Description,
    Priority,
    Estimate,
    Due,
    Tags,
}

impl Field {
    const ORDER: [Field; 6] = [
        Field::Title,
        Field::Description,
        Field::Priority,
        Field::Estimate,
        Field::Due,
        Field::Tags,
    ];

    pub fn next(self) -> Field {
        let index = Field::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Field::ORDER[(index + 1) % Field::ORDER.len()]
    }

    pub fn previous(self) -> Field {
        let index = Field::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Field::ORDER[(index + Field::ORDER.len() - 1) % Field::ORDER.len()]
    }
}

/// The values typed into the dialog. Text fields hold what was typed, and are only read as numbers
/// or dates on save.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Values {
    pub title: String,
    pub description: String,
    pub priority: Option<Priority>,
    pub estimate: String,
    pub due: String,
    /// Committed tags, as chips.
    pub tags: Vec<String>,
    /// Text typed for the next tag, not yet a chip.
    pub tag_draft: String,
}

impl Values {
    /// The same values with tags sorted, so the order chips were added in doesn't count as a change.
    fn comparable(&self) -> Values {
        let mut values = self.clone();
        values.tags.sort();
        values
    }
}

/// One message per field, if that field failed validation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Errors {
    pub title: Option<String>,
    pub estimate: Option<String>,
    pub due: Option<String>,
    pub tags: Option<String>,
}

/// The fields after validation, ready to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    pub title: String,
    pub description: String,
    pub priority: Option<Priority>,
    pub estimate: Option<Estimate>,
    pub due: Option<DueDate>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskForm {
    pub mode: Mode,
    pub values: Values,
    original: Values,
    pub errors: Errors,
    pub active: Field,
    /// A failed write. The dialog stays open with the edits, so this is shown above the buttons.
    pub save_error: Option<String>,
}

impl TaskForm {
    /// A blank form for a new task.
    pub fn create() -> Self {
        Self::with(Mode::Create, Values::default())
    }

    /// A form for an existing task, prefilled from it.
    pub fn edit(task: &Task) -> Self {
        let values = Values {
            title: task.title().to_string(),
            description: task.description().to_string(),
            priority: task.priority(),
            estimate: task
                .estimate()
                .map(|estimate| format_hours(estimate.minutes()))
                .unwrap_or_default(),
            due: task.due_on().map(|due| due.to_string()).unwrap_or_default(),
            tags: task.tags().to_vec(),
            tag_draft: String::new(),
        };
        Self::with(Mode::Edit(task.id()), values)
    }

    fn with(mode: Mode, values: Values) -> Self {
        Self {
            mode,
            original: values.clone(),
            values,
            errors: Errors::default(),
            active: Field::Title,
            save_error: None,
        }
    }

    /// Whether the values differ from the ones the dialog opened with. Undoing an edit makes it clean
    /// again, and reordering tags doesn't count.
    pub fn is_dirty(&self) -> bool {
        self.values.comparable() != self.original.comparable()
    }

    /// Checks every field and returns the values to write, or `None` after setting the errors.
    /// Every field is checked, so the user sees all the problems at once.
    pub fn validate(&mut self) -> Option<Validated> {
        self.errors = Errors::default();
        self.save_error = None;

        let title = match validate_title(&self.values.title) {
            Ok(title) => Some(title),
            Err(err) => {
                self.errors.title = Some(err.to_string());
                None
            }
        };
        let estimate = match parse_estimate_hours(&self.values.estimate) {
            Ok(estimate) => Some(estimate),
            Err(err) => {
                self.errors.estimate = Some(err.to_string());
                None
            }
        };
        let due = match parse_due_date(&self.values.due) {
            Ok(due) => Some(due),
            Err(err) => {
                self.errors.due = Some(err.to_string());
                None
            }
        };
        // A tag still being typed counts, so saving doesn't silently drop it.
        let pending = Some(self.values.tag_draft.as_str()).filter(|draft| !draft.trim().is_empty());
        let tags = match normalize_tags(self.values.tags.iter().map(String::as_str).chain(pending))
        {
            Ok(tags) => Some(tags),
            Err(err) => {
                self.errors.tags = Some(err.to_string());
                None
            }
        };

        Some(Validated {
            title: title?,
            description: self.values.description.trim().to_string(),
            priority: self.values.priority,
            estimate: estimate?,
            due: due?,
            tags: tags?,
        })
    }

    /// Types `text` into the active field. The estimate and due date only take the characters a
    /// number or date uses, so the common mistakes can't be typed at all. Validation still checks
    /// what gets through.
    pub fn type_text(&mut self, text: &str) {
        let accepted: String = match self.active {
            Field::Estimate => text
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect(),
            Field::Due => text
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '-')
                .collect(),
            _ => text.to_string(),
        };
        match self.active {
            Field::Title => self.values.title.push_str(&accepted),
            Field::Description => self.values.description.push_str(&accepted),
            Field::Estimate => self.values.estimate.push_str(&accepted),
            Field::Due => self.values.due.push_str(&accepted),
            Field::Tags => {
                // A comma ends the tag being typed.
                // Each comma ends the tag before it. The text after the last comma stays as the draft.
                let parts: Vec<&str> = accepted.split(',').collect();
                let last = parts.len() - 1;
                for (index, part) in parts.into_iter().enumerate() {
                    self.values.tag_draft.push_str(part);
                    if index < last {
                        self.commit_tag();
                    }
                }
                return;
            }
            Field::Priority => return,
        }
        self.errors = Errors::default();
    }

    /// Removes the last character of the active field. In the tags field, with nothing typed,
    /// removes the last chip.
    pub fn backspace(&mut self) {
        match self.active {
            Field::Title => {
                self.values.title.pop();
            }
            Field::Description => {
                self.values.description.pop();
            }
            Field::Estimate => {
                self.values.estimate.pop();
            }
            Field::Due => {
                self.values.due.pop();
            }
            Field::Tags => {
                if self.values.tag_draft.is_empty() {
                    self.values.tags.pop();
                } else {
                    self.values.tag_draft.pop();
                }
            }
            Field::Priority => {}
        }
    }

    /// Turns the tag being typed into a chip. Blank text does nothing, and a bad tag leaves the
    /// text in place with the reason shown.
    pub fn commit_tag(&mut self) {
        let draft = self.values.tag_draft.trim().to_string();
        if draft.is_empty() {
            return;
        }
        match normalize_tags(
            self.values
                .tags
                .iter()
                .map(String::as_str)
                .chain(Some(draft.as_str())),
        ) {
            Ok(tags) => {
                self.values.tags = tags;
                self.values.tag_draft.clear();
                self.errors.tags = None;
            }
            Err(err) => self.errors.tags = Some(err.to_string()),
        }
    }

    /// Moves the priority one step along, or back, without wrapping around. Nothing selected is the
    /// step before Low.
    pub fn step_priority(&mut self, forward: bool) {
        let current = match self.values.priority {
            None => None,
            Some(priority) => Priority::ALL.iter().position(|p| *p == priority),
        };
        let next = match (current, forward) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(index), true) => Some((index + 1).min(Priority::ALL.len() - 1)),
            (Some(0), false) => None,
            (Some(index), false) => Some(index - 1),
        };
        self.values.priority = next.map(|index| Priority::ALL[index]);
    }

    pub fn focus_next(&mut self) {
        self.active = self.active.next();
    }

    pub fn focus_previous(&mut self) {
        self.active = self.active.previous();
    }

    pub fn focus(&mut self, field: Field) {
        self.active = field;
    }
}

/// Hours for an estimate in minutes, without trailing zeros: 60 is `1`, 90 is `1.5`, 45 is `0.75`.
pub fn format_hours(minutes: u32) -> String {
    if minutes.is_multiple_of(60) {
        return (minutes / 60).to_string();
    }
    let text = format!("{:.2}", f64::from(minutes) / 60.0);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{Estimate, Position, Priority, ProjectId, Status, Task};

    use super::{Field, Mode, TaskForm, format_hours};

    fn filled() -> TaskForm {
        let mut form = TaskForm::create();
        form.values.title = "Write copy".to_string();
        // The snapshot is what the dialog opened with, so a filled form starts clean.
        form.original = form.values.clone();
        form
    }

    #[test]
    fn a_blank_title_blocks_saving_and_says_why() {
        let mut form = TaskForm::create();
        assert!(form.validate().is_none());
        assert_eq!(form.errors.title.as_deref(), Some("a task needs a title"));
    }

    #[test]
    fn a_title_of_only_spaces_is_blank() {
        let mut form = filled();
        form.values.title = "   ".to_string();
        assert!(form.validate().is_none());
        assert!(form.errors.title.is_some());
    }

    #[test]
    fn a_complete_form_validates_and_trims_the_title() {
        let mut form = filled();
        form.values.title = "  Write copy  ".to_string();
        let valid = form.validate().expect("only the title is required");
        assert_eq!(valid.title, "Write copy");
        assert_eq!(valid.estimate, None);
        assert_eq!(valid.due, None);
    }

    #[test]
    fn negative_estimates_are_rejected_and_only_that_field_is_marked() {
        let mut form = filled();
        form.values.estimate = "-1".to_string();
        assert!(form.validate().is_none());
        assert!(form.errors.estimate.is_some());
        assert!(form.errors.title.is_none());
    }

    #[test]
    fn zero_is_accepted_as_an_estimate() {
        let mut form = filled();
        form.values.estimate = "0".to_string();
        assert_eq!(
            form.validate().unwrap().estimate.map(|e| e.minutes()),
            Some(0)
        );
    }

    #[test]
    fn decimal_estimates_are_accepted() {
        let mut form = filled();
        form.values.estimate = "2.5".to_string();
        assert_eq!(
            form.validate().unwrap().estimate.map(|e| e.minutes()),
            Some(150)
        );
    }

    #[test]
    fn text_is_rejected_as_an_estimate() {
        let mut form = filled();
        form.values.estimate = "abc".to_string();
        assert!(form.validate().is_none());
        assert!(form.errors.estimate.is_some());
    }

    #[test]
    fn an_empty_estimate_means_none() {
        let mut form = filled();
        form.values.estimate = String::new();
        assert_eq!(form.validate().unwrap().estimate, None);
    }

    #[test]
    fn the_estimate_field_only_takes_number_characters() {
        let mut form = filled();
        form.focus(Field::Estimate);
        form.type_text("-2a.5");
        assert_eq!(form.values.estimate, "2.5");
    }

    #[test]
    fn a_bad_due_date_is_rejected() {
        let mut form = filled();
        form.values.due = "2026-02-30".to_string();
        assert!(form.validate().is_none());
        assert!(form.errors.due.is_some());
    }

    #[test]
    fn a_tag_still_being_typed_is_saved_with_the_task() {
        let mut form = filled();
        form.values.tag_draft = "Web".to_string();
        assert_eq!(form.validate().unwrap().tags, ["web"]);
    }

    #[test]
    fn a_new_form_is_clean_and_any_typing_makes_it_dirty() {
        let mut form = TaskForm::create();
        assert!(!form.is_dirty());
        form.type_text("x");
        assert!(form.is_dirty());
    }

    #[test]
    fn undoing_an_edit_is_clean_again() {
        let mut form = filled();
        form.values.title.push('x');
        assert!(form.is_dirty());
        form.backspace();
        assert!(!form.is_dirty());
    }

    #[test]
    fn reordering_tags_is_not_a_change() {
        let mut form = TaskForm::create();
        form.values.tags = vec!["a".to_string(), "b".to_string()];
        let original = form.values.clone();
        form.values.tags = vec!["b".to_string(), "a".to_string()];
        assert_eq!(form.values.comparable(), original.comparable());
        form.values.tags.push("c".to_string());
        assert!(form.is_dirty(), "adding a tag is a change");
    }

    #[test]
    fn a_form_opened_for_a_task_is_clean_until_it_changes() {
        let mut task = Task::new(
            ProjectId::new(),
            "Write copy",
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap();
        task.set_estimate(Some(Estimate::from_minutes(90).unwrap()));
        task.set_priority(Some(Priority::High));
        let form = TaskForm::edit(&task);
        assert_eq!(form.mode, Mode::Edit(task.id()));
        assert_eq!(form.values.estimate, "1.5");
        assert!(!form.is_dirty());
    }

    #[test]
    fn hours_print_without_trailing_zeros() {
        assert_eq!(format_hours(60), "1");
        assert_eq!(format_hours(90), "1.5");
        assert_eq!(format_hours(45), "0.75");
        assert_eq!(format_hours(120), "2");
    }

    #[test]
    fn priority_steps_stop_at_each_end_without_wrapping() {
        let mut form = filled();
        form.step_priority(false);
        assert_eq!(form.values.priority, None, "nothing is before Low");
        form.step_priority(true);
        assert_eq!(form.values.priority, Some(Priority::Low));
        form.step_priority(true);
        form.step_priority(true);
        form.step_priority(true);
        assert_eq!(
            form.values.priority,
            Some(Priority::High),
            "no wrap past High"
        );
    }

    #[test]
    fn tab_moves_through_the_fields_and_wraps_at_the_ends() {
        let mut form = filled();
        assert_eq!(form.active, Field::Title);
        form.focus_previous();
        assert_eq!(form.active, Field::Tags);
        form.focus_next();
        assert_eq!(form.active, Field::Title);
    }

    #[test]
    fn a_comma_turns_the_tag_being_typed_into_a_chip() {
        let mut form = filled();
        form.focus(Field::Tags);
        form.type_text("web,");
        assert_eq!(form.values.tags, ["web"]);
        assert!(form.values.tag_draft.is_empty());
    }

    #[test]
    fn backspace_in_an_empty_tag_field_removes_the_last_chip() {
        let mut form = filled();
        form.values.tags = vec!["a".to_string(), "b".to_string()];
        form.focus(Field::Tags);
        form.backspace();
        assert_eq!(form.values.tags, ["a"]);
    }
}
