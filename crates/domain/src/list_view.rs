//! The list view's settings for a project: which columns show, in what order, and how the rows are sorted.
//!
//! The columns and the sort fields are closed enums. Each maps to one fixed database expression, so the
//! text a user or a stored row contains never reaches a query. Stored values are read back through the
//! `parse` functions here. An unknown name is dropped, and if that leaves nothing usable the defaults are
//! used, so a value saved by a newer version can't break the list.

/// A column the list can show. Every column can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListColumn {
    Title,
    Status,
    Priority,
    Assignee,
    Tags,
    DueDate,
    Estimate,
}

impl ListColumn {
    /// Every column, in the order they're shown by default.
    pub const ALL: [ListColumn; 7] = [
        ListColumn::Title,
        ListColumn::Status,
        ListColumn::Priority,
        ListColumn::Assignee,
        ListColumn::Tags,
        ListColumn::DueDate,
        ListColumn::Estimate,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ListColumn::Title => "title",
            ListColumn::Status => "status",
            ListColumn::Priority => "priority",
            ListColumn::Assignee => "assignee",
            ListColumn::Tags => "tags",
            ListColumn::DueDate => "due_date",
            ListColumn::Estimate => "estimate",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        ListColumn::ALL
            .into_iter()
            .find(|column| column.name() == name)
    }

    /// The sort field a column sorts by.
    pub fn sort(self) -> SortColumn {
        match self {
            ListColumn::Title => SortColumn::Title,
            ListColumn::Status => SortColumn::Status,
            ListColumn::Priority => SortColumn::Priority,
            ListColumn::Assignee => SortColumn::Assignee,
            ListColumn::Tags => SortColumn::Tags,
            ListColumn::DueDate => SortColumn::DueDate,
            ListColumn::Estimate => SortColumn::Estimate,
        }
    }
}

/// What the rows are sorted by. `Manual` is the board's order: the position each task has in its
/// column. It's the default, and it isn't a column the user can choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Manual,
    Title,
    Status,
    Priority,
    Assignee,
    Tags,
    DueDate,
    Estimate,
}

impl SortColumn {
    pub fn name(self) -> &'static str {
        match self {
            SortColumn::Manual => "manual",
            SortColumn::Title => "title",
            SortColumn::Status => "status",
            SortColumn::Priority => "priority",
            SortColumn::Assignee => "assignee",
            SortColumn::Tags => "tags",
            SortColumn::DueDate => "due_date",
            SortColumn::Estimate => "estimate",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [
            SortColumn::Manual,
            SortColumn::Title,
            SortColumn::Status,
            SortColumn::Priority,
            SortColumn::Assignee,
            SortColumn::Tags,
            SortColumn::DueDate,
            SortColumn::Estimate,
        ]
        .into_iter()
        .find(|column| column.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn name(self) -> &'static str {
        match self {
            SortDirection::Ascending => "asc",
            SortDirection::Descending => "desc",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "asc" => Some(SortDirection::Ascending),
            "desc" => Some(SortDirection::Descending),
            _ => None,
        }
    }

    fn flipped(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

/// The sort: a field and a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListSort {
    pub column: SortColumn,
    pub direction: SortDirection,
}

/// A project's list settings: the sort, and the columns shown in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListView {
    pub sort: ListSort,
    /// The visible columns, in the order they're shown. Never empty.
    pub columns: Vec<ListColumn>,
}

impl Default for ListView {
    fn default() -> Self {
        Self {
            sort: ListSort::default(),
            columns: ListColumn::ALL.to_vec(),
        }
    }
}

impl ListView {
    /// Reads stored values back. The sort falls back to manual order and the columns to all of them when a
    /// stored value is unknown, so a value written by another version can't break the list. A column
    /// listed twice is shown once. A sort on a column that isn't shown is reset to manual order.
    pub fn from_stored(sort_column: &str, sort_direction: &str, columns: &[String]) -> Self {
        let mut columns_shown: Vec<ListColumn> = Vec::new();
        for name in columns {
            if let Some(column) = ListColumn::parse(name)
                && !columns_shown.contains(&column)
            {
                columns_shown.push(column);
            }
        }
        if columns_shown.is_empty() {
            columns_shown = ListColumn::ALL.to_vec();
        }
        let sort = ListSort {
            column: SortColumn::parse(sort_column).unwrap_or_default(),
            direction: SortDirection::parse(sort_direction).unwrap_or_default(),
        };
        let mut view = Self {
            sort,
            columns: columns_shown,
        };
        view.keep_sort_visible();
        view
    }

    /// The column names, in order, as they're stored.
    pub fn stored_columns(&self) -> Vec<&'static str> {
        self.columns.iter().map(|column| column.name()).collect()
    }

    /// Clicking a column header sorts by it. Clicking the column already sorted flips the direction.
    pub fn sort_by(&mut self, column: ListColumn) {
        let field = column.sort();
        self.sort = if self.sort.column == field {
            ListSort {
                column: field,
                direction: self.sort.direction.flipped(),
            }
        } else {
            ListSort {
                column: field,
                direction: SortDirection::Ascending,
            }
        };
    }

    /// Whether a column is shown.
    pub fn shows(&self, column: ListColumn) -> bool {
        self.columns.contains(&column)
    }

    /// Shows or hides a column. The last shown column can't be hidden, so this returns `false` and changes
    /// nothing. Hiding the column being sorted by resets the sort to manual order, and this returns `true`
    /// in that case so the caller can say so.
    pub fn toggle_column(&mut self, column: ListColumn) -> ToggleOutcome {
        if self.shows(column) {
            if self.columns.len() == 1 {
                return ToggleOutcome::Refused;
            }
            self.columns.retain(|shown| *shown != column);
            if self.sort.column == column.sort() {
                self.sort = ListSort::default();
                return ToggleOutcome::HiddenAndSortReset;
            }
            ToggleOutcome::Hidden
        } else {
            // Shown in the order of the default list, so turning a column back on puts it where it was.
            self.columns = ListColumn::ALL
                .into_iter()
                .filter(|candidate| self.columns.contains(candidate) || *candidate == column)
                .collect();
            ToggleOutcome::Shown
        }
    }

    /// A sort on a hidden column is confusing, so it's reset to manual order.
    fn keep_sort_visible(&mut self) {
        let sorted_column_shown = self.sort.column == SortColumn::Manual
            || self
                .columns
                .iter()
                .any(|column| column.sort() == self.sort.column);
        if !sorted_column_shown {
            self.sort = ListSort::default();
        }
    }
}

/// What happened when a column was shown or hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleOutcome {
    Shown,
    Hidden,
    /// Hidden, and the sort was on it, so the sort went back to manual order.
    HiddenAndSortReset,
    /// Not hidden: it's the last column shown.
    Refused,
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::list_view::{
        ListColumn, ListSort, ListView, SortColumn, SortDirection, ToggleOutcome,
    };

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_default_is_manual_order_with_every_column() {
        let view = ListView::default();
        assert_eq!(view.sort, ListSort::default());
        assert_eq!(view.columns, ListColumn::ALL);
    }

    #[test]
    fn a_column_name_round_trips_and_an_unknown_one_is_refused() {
        for column in ListColumn::ALL {
            assert_eq!(ListColumn::parse(column.name()), Some(column));
        }
        assert_eq!(ListColumn::parse("drop table tasks"), None);
        assert_eq!(SortColumn::parse("1; DROP"), None);
    }

    #[test]
    fn stored_names_for_unknown_columns_are_dropped_and_duplicates_shown_once() {
        let view = ListView::from_stored(
            "title",
            "desc",
            &names(&["status", "removed_in_a_later_version", "status", "title"]),
        );
        assert_eq!(view.columns, [ListColumn::Status, ListColumn::Title]);
    }

    #[test]
    fn when_no_stored_column_is_usable_all_columns_return() {
        let view = ListView::from_stored("manual", "asc", &names(&["nonsense"]));
        assert_eq!(view.columns, ListColumn::ALL);
        assert_eq!(
            ListView::from_stored("manual", "asc", &[]).columns,
            ListColumn::ALL
        );
    }

    #[test]
    fn an_unknown_sort_falls_back_to_manual_order() {
        let view = ListView::from_stored("bogus", "sideways", &names(&["title"]));
        assert_eq!(view.sort, ListSort::default());
    }

    #[test]
    fn a_stored_sort_on_a_hidden_column_is_reset() {
        let view = ListView::from_stored("due_date", "desc", &names(&["title", "status"]));
        assert_eq!(view.sort, ListSort::default());
    }

    #[test]
    fn a_header_click_sorts_ascending_then_flips() {
        let mut view = ListView::default();
        view.sort_by(ListColumn::DueDate);
        assert_eq!(view.sort.column, SortColumn::DueDate);
        assert_eq!(view.sort.direction, SortDirection::Ascending);
        view.sort_by(ListColumn::DueDate);
        assert_eq!(view.sort.direction, SortDirection::Descending);
        view.sort_by(ListColumn::Title);
        assert_eq!(view.sort.column, SortColumn::Title);
        assert_eq!(view.sort.direction, SortDirection::Ascending);
    }

    #[test]
    fn the_last_column_cannot_be_hidden() {
        let mut view = ListView::from_stored("manual", "asc", &names(&["title"]));
        assert_eq!(
            view.toggle_column(ListColumn::Title),
            ToggleOutcome::Refused
        );
        assert_eq!(view.columns, [ListColumn::Title]);
    }

    #[test]
    fn hiding_the_sorted_column_resets_the_sort_and_says_so() {
        let mut view = ListView::default();
        view.sort_by(ListColumn::Priority);
        assert_eq!(
            view.toggle_column(ListColumn::Priority),
            ToggleOutcome::HiddenAndSortReset
        );
        assert_eq!(view.sort, ListSort::default());
        assert!(!view.shows(ListColumn::Priority));
    }

    #[test]
    fn a_shown_column_returns_to_its_place_in_the_list() {
        let mut view = ListView::default();
        view.toggle_column(ListColumn::Tags);
        assert_eq!(view.toggle_column(ListColumn::Tags), ToggleOutcome::Shown);
        assert_eq!(view.columns, ListColumn::ALL);
    }
}
