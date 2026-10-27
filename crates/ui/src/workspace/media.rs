//! Media on a task: the images, videos, and files attached to it. They show in the task's expanded card. Adding copies
//! the picked files into the app's attachments folder in the background, then records each one. A file is opened
//! in the system's own app, and a file can be removed from the task.

use std::path::{Path, PathBuf};

use gpui::{prelude::*, *};
use looji_domain::{Attachment, AttachmentRepository, MediaKind, TaskId, Timestamp};
use looji_persistence::attachments::{
    AttachmentError, StoredFile, import_attachment, remove_attachment,
};

use super::task_form::Mode;
use super::{WorkspaceView, store};
use crate::{components, database::AppDatabase, theme::Theme};

/// The tiles for a task's media, in the order they were added. Images show a picture. Videos and other files show
/// their name. A click on a tile opens the file, and the cross on it removes the file from the task.
pub(super) fn strip(
    task: TaskId,
    items: &[Attachment],
    dir: &Path,
    compact: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    if items.is_empty() {
        return None;
    }
    let tiles: Vec<AnyElement> = items
        .iter()
        .enumerate()
        .map(|(index, item)| tile(task, index, item, dir, compact, theme, cx))
        .collect();
    Some(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(theme.spacing.sm)
            .children(tiles)
            .into_any_element(),
    )
}

fn tile(
    task: TaskId,
    index: usize,
    item: &Attachment,
    dir: &Path,
    compact: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let path = dir.join(&item.stored_name);
    let body = match item.kind {
        MediaKind::Image => div()
            .w(px(if compact { 76. } else { 132. }))
            .h(px(if compact { 54. } else { 92. }))
            .rounded(theme.radius.md)
            .overflow_hidden()
            .border_1()
            .border_color(theme.palette.border)
            .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
            .into_any_element(),
        MediaKind::Video | MediaKind::File => {
            let icon_name = if item.kind == MediaKind::Video {
                "photo"
            } else {
                "file-text"
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .h(px(if compact { 28. } else { 32. }))
                .max_w(px(if compact { 190. } else { 240. }))
                .pl(theme.spacing.sm)
                // Room on the right for the remove cross, so it never covers the name.
                .pr(px(30.))
                .rounded(theme.radius.md)
                .bg(theme.palette.surface)
                .border_1()
                .border_color(theme.palette.border)
                .child(components::icon(
                    icon_name,
                    14.,
                    theme.palette.text_secondary,
                ))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(theme.typography.caption)
                        .child(item.file_name.clone()),
                )
                .into_any_element()
        }
    };
    let stored = item.stored_name.clone();
    let open = path;
    div()
        .id(("media", index as u64))
        .relative()
        .cursor_pointer()
        .child(body)
        .child(div().absolute().top(px(4.)).right(px(4.)).child(
            components::icon_button(("media-remove", index as u64), "x", theme).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    // The tile's own click would open the file, so this press stops there.
                    cx.stop_propagation();
                    this.remove_media(task, stored.clone(), cx)
                }),
            ),
        ))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_media(open.clone(), cx)))
        .into_any_element()
}

/// The label above a description, with a paperclip at the end that attaches images, videos or files. The media then
/// shows right under the description, as part of it.
pub(super) fn description_label(
    task: TaskId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Description"),
        )
        .child(
            components::icon_button(
                ("media-attach", task.as_uuid().as_u128() as u64),
                "paperclip",
                theme,
            )
            .tooltip(super::card::tooltip("Attach images, videos or files"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.add_media(task, cx))),
        )
}

/// The tiles of a task's media, to sit under its description. `None` when it has none.
pub(super) fn tiles(
    task: TaskId,
    items: &[Attachment],
    compact: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dir = cx.global::<AppDatabase>().attachments_dir.clone();
    strip(task, items, &dir, compact, theme, cx)
}

impl WorkspaceView {
    /// Loads a task's media for its expanded card.
    pub(super) fn load_media(&mut self, task: TaskId, cx: &mut Context<Self>) {
        self.expanded_media = match store(cx).list_for_task(task) {
            Ok(items) => items,
            Err(err) => {
                self.status = Some(format!("Could not load the media: {err}"));
                Vec::new()
            }
        };
    }

    /// Asks for files, then copies each one into the attachments folder and records it on the task.
    pub(super) fn add_media(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add media".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let dir = cx.update(|cx| cx.global::<AppDatabase>().attachments_dir.clone());
            for source in paths {
                let dir = dir.clone();
                let imported = cx
                    .background_executor()
                    .spawn(async move { import_attachment(&source, &dir) })
                    .await;
                let _ = this.update(cx, |view, cx| view.record_media(task, imported, cx));
            }
        })
        .detach();
    }

    /// Records a copied file on the task. A file the store refuses is removed again, so nothing is left behind.
    fn record_media(
        &mut self,
        task: TaskId,
        imported: Result<StoredFile, AttachmentError>,
        cx: &mut Context<Self>,
    ) {
        let file = match imported {
            Ok(file) => file,
            Err(err) => {
                self.status = Some(format!("Could not add that file: {err}"));
                return cx.notify();
            }
        };
        let attachment = Attachment {
            task,
            kind: MediaKind::from_file_name(&file.file_name),
            stored_name: file.stored_name.clone(),
            file_name: file.file_name,
            created_at: Timestamp::now(),
        };
        let dir = cx.global::<AppDatabase>().attachments_dir.clone();
        match store(cx).add(&attachment) {
            Ok(()) => {
                if let Some(panel) = self.panel.as_mut().filter(|panel| panel.task == task) {
                    panel.media.push(attachment.clone());
                }
                if let Some(dialog) = self
                    .task_dialog
                    .as_mut()
                    .filter(|dialog| matches!(dialog.form.mode, Mode::Edit(id) if id == task))
                {
                    dialog.media.push(attachment.clone());
                }
                if self.expanded_card == Some(task) {
                    self.expanded_media.push(attachment);
                }
                self.status = None;
                self.index_attachments(cx);
            }
            Err(err) => {
                let _ = remove_attachment(&dir, &attachment.stored_name);
                self.status = Some(format!("Could not add that file: {err}"));
            }
        }
        cx.notify();
    }

    /// Removes a file from a task and deletes its copy.
    pub(super) fn remove_media(
        &mut self,
        task: TaskId,
        stored_name: String,
        cx: &mut Context<Self>,
    ) {
        match store(cx).remove(task, &stored_name) {
            Ok(()) => {
                let dir = cx.global::<AppDatabase>().attachments_dir.clone();
                if let Err(err) = remove_attachment(&dir, &stored_name) {
                    eprintln!("could not delete the file of a removed attachment: {err}");
                }
                self.expanded_media
                    .retain(|item| item.stored_name != stored_name);
                if let Some(panel) = self.panel.as_mut().filter(|panel| panel.task == task) {
                    panel.media.retain(|item| item.stored_name != stored_name);
                }
                if let Some(dialog) = self
                    .task_dialog
                    .as_mut()
                    .filter(|dialog| matches!(dialog.form.mode, Mode::Edit(id) if id == task))
                {
                    dialog.media.retain(|item| item.stored_name != stored_name);
                }
                self.status = None;
                self.index_attachments(cx);
            }
            Err(err) => self.status = Some(format!("Could not remove the file: {err}")),
        }
        cx.notify();
    }

    /// Opens a file in the system's own app for its kind.
    pub(super) fn open_media(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.open_with_system(&path);
    }
}
