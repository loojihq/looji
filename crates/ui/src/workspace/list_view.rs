//! The list view: one row per task, a page at a time. Each page is read from the database with keyset
//! paging, so a page never loads the whole project, and a page deep in the list costs what the first one
//! does.
//!
//! Reads run on a background connection. Each request takes a sequence number, and a response is applied
//! only if no newer request was made, so a slow page can't overwrite a newer one. The filters and the
//! search are the same ones the board uses, through one query, so the total here is the board's count.
//!
//! The sort and the shown columns belong to the project. A header click sorts, and the column picker
//! shows or hides columns. Both are saved as they change, and loaded when the project opens.

use gpui::{prelude::*, *};
use looji_domain::{
    ListColumn, ListView, ListViewRepository, PAGE_SIZE, PageCursor, SortDirection, Task,
    TaskQuery, TaskRepository, ToggleOutcome,
};
use looji_persistence::SqliteStore;

use super::board::status_label;
use super::fields::member_mark;
use super::members;
use super::task_editors::priority_label;
use super::task_form::format_hours;
use super::{WorkspaceView, store};
use crate::database::AppDatabase;
use crate::theme::Theme;

const ROW_HEIGHT: Pixels = px(36.);

/// Which body the board area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ViewMode {
    #[default]
    Board,
    List,
    Roadmap,
}

/// The list's paging state. `cursors[p]` is where page `p` starts, so the page after the current one is
/// always `cursors[page + 1]`, taken from the last row shown.
#[derive(Debug, Clone)]
pub(super) struct ListState {
    pub page: usize,
    pub cursors: Vec<Option<PageCursor>>,
    pub rows: Vec<Task>,
    pub total: u64,
    pub has_next: bool,
    /// The number of the latest request. Only its response is applied.
    pub seq: u64,
    pub error: Option<String>,
}

impl Default for ListState {
    fn default() -> Self {
        Self {
            page: 0,
            cursors: vec![None],
            rows: Vec::new(),
            total: 0,
            has_next: false,
            seq: 0,
            error: None,
        }
    }
}

impl WorkspaceView {
    /// Goes back to the first page. Done when the project or the query changes.
    pub(super) fn reset_list_position(&mut self) {
        self.task_list = ListState {
            seq: self.task_list.seq,
            ..ListState::default()
        };
    }

    /// Shows the first page of the list again, fetching it.
    pub(super) fn restart_list(&mut self, cx: &mut Context<Self>) {
        self.reset_list_position();
        self.refresh_list(cx);
    }

    /// Fetches the current page again, if the list is showing. Keeps the page the user is on, so an edit
    /// doesn't move them back to the start.
    pub(super) fn refresh_list(&mut self, cx: &mut Context<Self>) {
        if self.view_mode == ViewMode::List {
            self.request_list_page(cx);
        }
    }

    /// Called when the filter or the search changes. The old pages no longer match, so the list starts
    /// again from the first page.
    pub(super) fn list_query_changed(&mut self, cx: &mut Context<Self>) {
        if self.view_mode == ViewMode::List {
            self.restart_list(cx);
        }
    }

    pub(super) fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.view_mode == mode {
            return;
        }
        self.view_mode = mode;
        if mode == ViewMode::List {
            self.restart_list(cx);
        }
        cx.notify();
    }

    pub(super) fn list_next(&mut self, cx: &mut Context<Self>) {
        if self.task_list.has_next {
            self.task_list.page += 1;
            self.request_list_page(cx);
        }
    }

    pub(super) fn list_previous(&mut self, cx: &mut Context<Self>) {
        if self.task_list.page > 0 {
            self.task_list.page -= 1;
            self.request_list_page(cx);
        }
    }

    /// Reads the project's saved list settings. A project that never saved any gets the defaults. Called
    /// when a project opens, before its list is read.
    pub(super) fn load_list_settings(&mut self, cx: &mut Context<Self>) {
        self.list_columns_open = false;
        let Some(project) = self.projects.active_id() else {
            self.list_settings = ListView::default();
            return;
        };
        self.list_settings = match store(cx).load_list_view(project) {
            Ok(view) => view,
            Err(err) => {
                eprintln!("could not load the list settings: {err}");
                self.status = Some("Could not load this board's list settings.".to_string());
                ListView::default()
            }
        };
    }

    /// Sorts the list by a column. The same column again flips the direction. The list starts from its
    /// first page, since the rows are in a new order.
    pub(super) fn sort_list(&mut self, column: ListColumn, cx: &mut Context<Self>) {
        self.list_settings.sort_by(column);
        self.save_list_settings(cx);
        self.restart_list(cx);
    }

    /// Shows or hides a column from the picker. The last shown column can't be hidden. Hiding the column
    /// being sorted by puts the sort back to manual order, and the user is told why.
    pub(super) fn toggle_list_column(&mut self, column: ListColumn, cx: &mut Context<Self>) {
        match self.list_settings.toggle_column(column) {
            ToggleOutcome::Refused => {
                self.status = Some("At least one column must stay visible.".to_string());
            }
            ToggleOutcome::Shown | ToggleOutcome::Hidden => {
                self.save_list_settings(cx);
            }
            ToggleOutcome::HiddenAndSortReset => {
                self.status = Some(
                    "The list was sorted by that column, so it's back in board order.".to_string(),
                );
                self.save_list_settings(cx);
                self.restart_list(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_columns_menu(&mut self, cx: &mut Context<Self>) {
        self.list_columns_open = !self.list_columns_open;
        cx.notify();
    }

    fn save_list_settings(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        if let Err(err) = store(cx).save_list_view(project, &self.list_settings) {
            eprintln!("could not save the list settings: {err}");
            self.status = Some("Could not save the list settings.".to_string());
        }
    }

    /// Reads the current page on a background connection. The result is applied only if it's still the
    /// latest request when it arrives.
    fn request_list_page(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            self.task_list.rows.clear();
            self.task_list.total = 0;
            self.task_list.has_next = false;
            return cx.notify();
        };
        let statuses = self
            .projects
            .active()
            .map(|project| project.visible_statuses())
            .unwrap_or_default();
        let query = TaskQuery::new(statuses, self.filter.clone(), &self.search_query)
            .with_sort(self.list_settings.sort);
        let cursor = self
            .task_list
            .cursors
            .get(self.task_list.page)
            .cloned()
            .flatten();
        let page_index = self.task_list.page;
        self.task_list.seq += 1;
        let seq = self.task_list.seq;
        let path = cx.global::<AppDatabase>().database_path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = SqliteStore::open(&path).map_err(|err| err.to_string())?;
                    store
                        .list_page(project, &query, cursor.as_ref(), PAGE_SIZE)
                        .map_err(|err| err.to_string())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.apply_list_page(seq, page_index, result, cx)
            });
        })
        .detach();
    }

    fn apply_list_page(
        &mut self,
        seq: u64,
        page_index: usize,
        result: Result<looji_domain::TaskPage, String>,
        cx: &mut Context<Self>,
    ) {
        if seq != self.task_list.seq {
            return;
        }
        match result {
            Ok(page) => {
                // A row was deleted and the total shrank below this page. Show the page before it.
                if page.tasks.is_empty() && page_index > 0 && page_index == self.task_list.page {
                    self.task_list.page -= 1;
                    return self.request_list_page(cx);
                }
                self.task_list.error = None;
                self.task_list.cursors.truncate(self.task_list.page + 1);
                if let Some(next) = page.next.clone() {
                    self.task_list.cursors.push(Some(next));
                }
                self.task_list.rows = page.tasks;
                self.task_list.total = page.total;
                self.task_list.has_next = page.has_next;
            }
            Err(message) => {
                eprintln!("could not load the list: {message}");
                self.task_list.error = Some("Could not load tasks. Try again.".to_string());
            }
        }
        cx.notify();
    }
}

/// The Board and List switch in the header.
pub(super) fn switch(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let mut row = div().flex().flex_row().gap(theme.spacing.xs);
    for (index, (mode, label)) in [
        (ViewMode::Board, "Board"),
        (ViewMode::List, "List"),
        (ViewMode::Roadmap, "Roadmap"),
    ]
    .into_iter()
    .enumerate()
    {
        let selected = view.view_mode == mode;
        row = row.child(
            div()
                .id(("view-switch", index as u64))
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .text_size(theme.typography.caption)
                .cursor_pointer()
                .when(selected, |button| button.bg(theme.palette.surface_selected))
                .when(!selected, |button| {
                    button.hover(|style| style.bg(theme.palette.surface_hover))
                })
                .child(label)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_view_mode(mode, cx)),
                ),
        );
    }
    row.into_any_element()
}

/// The list body: the header, one row per task, the column picker when it's open, and the footer.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let entries = members::entries(&view.members);
    let mut body = div()
        .id("list-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll();
    body = body.child(header_row(view, theme, cx));
    for (index, task) in view.task_list.rows.iter().enumerate() {
        body = body.child(row(index, task, view, &entries, theme, cx));
    }
    if view.task_list.rows.is_empty() && view.task_list.total == 0 {
        body = body.child(empty(view, theme));
    }

    let mut column = div().flex().flex_col().size_full().min_h_0().child(body);
    if view.list_columns_open {
        column = column.child(columns_menu(view, theme, cx));
    }
    column.child(footer(view, theme, cx)).into_any_element()
}

fn column_label(column: ListColumn) -> &'static str {
    match column {
        ListColumn::Title => "Title",
        ListColumn::Status => "Status",
        ListColumn::Priority => "Priority",
        ListColumn::Assignee => "Assignee",
        ListColumn::Tags => "Tags",
        ListColumn::DueDate => "Due",
        ListColumn::Estimate => "Estimate",
    }
}

/// The fixed width of a column, or `None` for the one that takes the rest of the row.
fn column_width(column: ListColumn) -> Option<f32> {
    match column {
        ListColumn::Title => None,
        ListColumn::Status => Some(110.),
        ListColumn::Priority => Some(90.),
        ListColumn::Assignee => Some(150.),
        ListColumn::Tags => Some(180.),
        ListColumn::DueDate => Some(100.),
        ListColumn::Estimate => Some(80.),
    }
}

fn cell(id: (&'static str, u64), column: ListColumn) -> Stateful<Div> {
    let base = div().id(id).min_w_0().truncate();
    match column_width(column) {
        Some(width) => base.w(px(width)).flex_shrink_0(),
        None => base.flex_1(),
    }
}

/// The column headings. A click sorts by the column; the column the list is sorted by shows an arrow in
/// the direction of the sort.
fn header_row(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let sort = view.list_settings.sort;
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .h(px(32.))
        .px(theme.spacing.lg)
        .border_b_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary);
    for (index, column) in view.list_settings.columns.iter().copied().enumerate() {
        let arrow = match (sort.column == column.sort(), sort.direction) {
            (false, _) => "",
            (true, SortDirection::Ascending) => " ↑",
            (true, SortDirection::Descending) => " ↓",
        };
        row = row.child(
            cell(("list-heading", index as u64), column)
                .cursor_pointer()
                .hover(|style| style.text_color(theme.palette.text_primary))
                .child(format!("{}{arrow}", column_label(column)))
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.sort_list(column, cx)),
                ),
        );
    }
    row
}

fn row(
    index: usize,
    task: &Task,
    view: &WorkspaceView,
    entries: &[members::Entry],
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = task.id();
    let mut row = div()
        .id(("list-row", index as u64))
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .h(ROW_HEIGHT)
        .px(theme.spacing.lg)
        .border_b_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.body)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover));
    for (column_index, column) in view.list_settings.columns.iter().copied().enumerate() {
        let content = cell_content(column, task, entries, theme);
        row = row
            .child(cell(("list-cell", (index * 16 + column_index) as u64), column).child(content));
    }
    row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx)))
}

/// What one column shows for a task.
fn cell_content(
    column: ListColumn,
    task: &Task,
    entries: &[members::Entry],
    theme: &Theme,
) -> AnyElement {
    match column {
        ListColumn::Title => task.title().to_string().into_any_element(),
        ListColumn::Status => status_label(task.status()).into_any_element(),
        ListColumn::Priority => priority_label(task.priority()).into_any_element(),
        ListColumn::Assignee => {
            let assignee = task
                .assignee_id()
                .and_then(|assignee| entries.iter().find(|entry| entry.id == assignee));
            match assignee {
                Some(entry) => div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.xs)
                    .child(member_mark(&entry.initials, entry.colour, px(20.), theme))
                    .child(entry.label.clone())
                    .into_any_element(),
                None => div()
                    .text_color(theme.palette.text_secondary)
                    .child("Unassigned")
                    .into_any_element(),
            }
        }
        ListColumn::Tags => task.tags().join(", ").into_any_element(),
        ListColumn::DueDate => task
            .due_on()
            .map(|due| due.to_string())
            .unwrap_or_else(|| "—".to_string())
            .into_any_element(),
        ListColumn::Estimate => task
            .estimate()
            .map(|estimate| format!("{}h", format_hours(estimate.minutes())))
            .unwrap_or_else(|| "—".to_string())
            .into_any_element(),
    }
}

/// The column picker: a checkbox per column. The last shown column is disabled, so the list can't end up
/// with no columns.
fn columns_menu(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let shown = view.list_settings.columns.len();
    let mut menu = div()
        .id("list-columns-menu")
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs)
        .mx(theme.spacing.lg)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .bg(theme.palette.surface_raised)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    for (index, column) in ListColumn::ALL.into_iter().enumerate() {
        let checked = view.list_settings.shows(column);
        let locked = checked && shown == 1;
        let tick = if checked { "✓" } else { " " };
        let mut option = div()
            .id(("list-column-option", index as u64))
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .px(theme.spacing.sm)
            .py(theme.spacing.xs)
            .rounded(theme.radius.sm)
            .child(div().w(px(14.)).child(tick))
            .child(column_label(column));
        option = if locked {
            option.text_color(theme.palette.text_disabled)
        } else {
            option
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_list_column(column, cx)
                }))
        };
        menu = menu.child(option);
    }
    menu.into_any_element()
}

/// The message for a list with no rows: no tasks at all, or none that the filters and search let through.
fn empty(view: &WorkspaceView, theme: &Theme) -> impl IntoElement {
    let text = if view.filter.is_empty() && view.search_query.trim().is_empty() {
        "No tasks"
    } else {
        "No tasks match these filters."
    };
    div()
        .px(theme.spacing.lg)
        .py(theme.spacing.md)
        .text_color(theme.palette.text_secondary)
        .child(text)
}

/// The columns button, the count, and Previous and Next. Next is off on the last page, so it never leads
/// to an empty page.
fn footer(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let list = &view.task_list;
    let summary = if list.rows.is_empty() {
        String::new()
    } else {
        let start = list.page * PAGE_SIZE + 1;
        let end = start + list.rows.len() - 1;
        format!("Showing {start}–{end} of {}", list.total)
    };

    let columns_button = div()
        .id("list-columns-open")
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("Columns")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_columns_menu(cx)));

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .h(px(40.))
        .px(theme.spacing.lg)
        .border_t_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.caption)
        .child(columns_button)
        .child(
            div()
                .flex_1()
                .text_color(theme.palette.text_secondary)
                .child(summary),
        );
    if let Some(error) = list.error.clone() {
        row = row.child(div().text_color(theme.palette.danger).child(error));
    }
    row.child(
        div()
            .flex()
            .flex_row()
            .gap(theme.spacing.xs)
            .child(
                pager("list-previous", "Previous", list.page > 0, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.list_previous(cx))),
            )
            .child(
                pager("list-next", "Next", list.has_next, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.list_next(cx))),
            ),
    )
}

/// A paging button. A disabled one is drawn muted and does nothing when pressed.
fn pager(id: &'static str, label: &str, enabled: bool, theme: &Theme) -> Stateful<Div> {
    let button = div()
        .id(id)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .child(label.to_string());
    if enabled {
        button
            .cursor_pointer()
            .hover(|style| style.bg(theme.palette.surface_hover))
    } else {
        button.text_color(theme.palette.text_disabled)
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{ListState, column_width};
    use looji_domain::ListColumn;

    #[test]
    fn a_fresh_list_starts_at_the_first_page_with_no_cursor() {
        let state = ListState::default();
        assert_eq!(state.page, 0);
        assert_eq!(state.cursors, [None]);
    }

    #[test]
    fn only_the_title_takes_the_rest_of_the_row() {
        let flexible: Vec<ListColumn> = ListColumn::ALL
            .into_iter()
            .filter(|column| column_width(*column).is_none())
            .collect();
        assert_eq!(flexible, [ListColumn::Title]);
    }
}
