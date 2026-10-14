//! The list view: one row per task, a page at a time. Each page is read from the database with keyset
//! paging, so a page never loads the whole project, and a page deep in the list costs what the first one
//! does.
//!
//! Reads run on a background connection. Each request takes a sequence number, and a response is applied
//! only if no newer request was made, so a slow page can't overwrite a newer one. The filters and the
//! search are the same ones the board uses, through one query, so the total here is the board's count.

use gpui::{prelude::*, *};
use looji_domain::{PAGE_SIZE, PageCursor, Task, TaskPage, TaskQuery, TaskRepository};
use looji_persistence::SqliteStore;

use super::WorkspaceView;
use super::board::status_label;
use super::fields::member_mark;
use super::members;
use super::task_editors::priority_label;
use super::task_form::format_hours;
use crate::database::AppDatabase;
use crate::theme::Theme;

const ROW_HEIGHT: Pixels = px(36.);

/// Which body the board area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ViewMode {
    #[default]
    Board,
    List,
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
        let query = TaskQuery::new(statuses, self.filter.clone(), &self.search_query);
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
        result: Result<TaskPage, String>,
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
                if let Some(last) = page.tasks.last().filter(|_| page.has_next) {
                    self.task_list.cursors.push(Some(PageCursor::after(last)));
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
    for (index, (mode, label)) in [(ViewMode::Board, "Board"), (ViewMode::List, "List")]
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

/// The list body: the header, one row per task, and the paging footer.
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
    body = body.child(header_row(theme));
    for (index, task) in view.task_list.rows.iter().enumerate() {
        body = body.child(row(index, task, &entries, theme, cx));
    }
    if view.task_list.rows.is_empty() && view.task_list.total == 0 {
        body = body.child(empty(view, theme));
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(body)
        .child(footer(view, theme, cx))
        .into_any_element()
}

/// The column headings. The widths here are the widths of the cells in each row.
fn columns() -> [(&'static str, Option<f32>); 7] {
    [
        ("Title", None),
        ("Status", Some(110.)),
        ("Priority", Some(90.)),
        ("Assignee", Some(150.)),
        ("Tags", Some(180.)),
        ("Due", Some(100.)),
        ("Estimate", Some(80.)),
    ]
}

fn cell(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)).flex_shrink_0().min_w_0().truncate(),
        None => div().flex_1().min_w_0().truncate(),
    }
}

fn header_row(theme: &Theme) -> impl IntoElement {
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
    for (label, width) in columns() {
        row = row.child(cell(width).child(label));
    }
    row
}

fn row(
    index: usize,
    task: &Task,
    entries: &[members::Entry],
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = task.id();
    let assignee = task
        .assignee_id()
        .and_then(|assignee| entries.iter().find(|entry| entry.id == assignee));
    let assignee_cell = match assignee {
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
    };
    let estimate = task
        .estimate()
        .map(|estimate| format!("{}h", format_hours(estimate.minutes())))
        .unwrap_or_else(|| "—".to_string());
    let due = task
        .due_on()
        .map(|due| due.to_string())
        .unwrap_or_else(|| "—".to_string());

    div()
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
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(cell(None).child(task.title().to_string()))
        .child(cell(Some(110.)).child(status_label(task.status())))
        .child(cell(Some(90.)).child(priority_label(task.priority())))
        .child(cell(Some(150.)).child(assignee_cell))
        .child(cell(Some(180.)).child(task.tags().join(", ")))
        .child(cell(Some(100.)).child(due))
        .child(cell(Some(80.)).child(estimate))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx)))
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

/// "Showing 101–150 of 137", with Previous and Next. Next is off on the last page, so it never leads to
/// an empty page.
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
    let previous_enabled = list.page > 0;
    let next_enabled = list.has_next;

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(40.))
        .px(theme.spacing.lg)
        .border_t_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.caption)
        .child(
            div()
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
                pager("list-previous", "Previous", previous_enabled, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.list_previous(cx))),
            )
            .child(
                pager("list-next", "Next", next_enabled, theme)
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
    use super::ListState;

    #[test]
    fn a_fresh_list_starts_at_the_first_page_with_no_cursor() {
        let state = ListState::default();
        assert_eq!(state.page, 0);
        assert_eq!(state.cursors, [None]);
    }
}
