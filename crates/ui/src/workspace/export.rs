//! Exports the selected project's board as a PDF report. The save dialog comes first. Cancelling it does nothing. The
//! file is written on a background thread, and the sidebar then says where it went or why it couldn't be written.

use std::path::PathBuf;

use gpui::{prelude::*, *};
use looji_domain::{ReportItem, Timestamp, group_by_status};
use looji_report::{ReportInput, ReportKind, write_report};

use super::{WorkspaceView, panel::format_time};
use crate::theme::Theme;

impl WorkspaceView {
    /// Asks where to save the report, then writes it. Nothing is saved when no project is selected or the dialog is
    /// cancelled.
    pub(super) fn export_report(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let suggested = format!("{}.pdf", file_name_part(project.name()));
        let input = self.report_input();
        let picked = cx.prompt_for_new_path(&documents_folder(), Some(&suggested));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let outcome = cx
                .background_executor()
                .spawn(async move { write_report(&input, &path).map_err(|err| err.to_string()) })
                .await;
            let _ = this.update(cx, |view, cx| view.finish_export(outcome, cx));
        })
        .detach();
    }

    /// The selected project's tasks as the report sees them. Its board is already loaded, so this reads memory only.
    fn report_input(&self) -> ReportInput {
        let items: Vec<ReportItem> = self
            .board
            .iter()
            .map(|task| ReportItem {
                title: task.title().to_string(),
                description: task.description().to_string(),
                status: task.status(),
                priority: task
                    .priority()
                    .map(|priority| priority.as_str().to_string())
                    .unwrap_or_else(|| "none".to_string()),
                assignee: task.assignee_id().and_then(|id| {
                    self.members
                        .iter()
                        .find(|member| member.id() == id)
                        .map(|member| member.name().to_string())
                }),
                due: task.due_on().map(|due| due.to_string()),
            })
            .collect();
        let project = self.projects.active();
        ReportInput {
            project: project
                .map(|project| project.name().to_string())
                .unwrap_or_default(),
            workspace: self.list.active().name().to_string(),
            kind: ReportKind::Published,
            description: project
                .and_then(|project| project.description())
                .unwrap_or_default()
                .to_string(),
            generated_on: format!("Generated {}", format_time(Timestamp::now())),
            sections: group_by_status(&items),
        }
    }

    fn finish_export(&mut self, outcome: Result<PathBuf, String>, cx: &mut Context<Self>) {
        self.status = Some(match outcome {
            Ok(path) => format!("Report saved to {}", path.display()),
            Err(message) => message,
        });
        cx.notify();
    }
}

/// A file name made from a project name, without the characters Windows doesn't allow in names.
pub(super) fn file_name_part(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "report".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Where the save dialog starts: the user's Documents folder, or the current folder when there isn't one.
pub(super) fn documents_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Documents"))
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The export button in the header, drawn while a project is selected.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    crate::components::button_icon(
        "export-report",
        "file-text",
        "Export PDF",
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_report(cx)))
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::workspace::export::file_name_part;

    #[test]
    fn a_project_name_becomes_a_safe_file_name() {
        assert_eq!(file_name_part("Web: relaunch/v2"), "Web- relaunch-v2");
        assert_eq!(file_name_part("   "), "report");
    }
}
