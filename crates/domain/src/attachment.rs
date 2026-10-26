//! Media attached to a task: images, videos, and other files. The bytes live in the app's data folder, and the
//! task keeps a row for each one, so a task can show its media in order.

use crate::{RepoResult, TaskId, Timestamp};

/// What an attachment is, which decides how it's shown. Only images are drawn inline. Videos and other files show
/// as a row that opens them in the system's own app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    File,
}

impl MediaKind {
    /// The kind a file name suggests, from its extension. Anything unknown is an ordinary file.
    pub fn from_file_name(file_name: &str) -> Self {
        let extension = file_name
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => MediaKind::Image,
            "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => MediaKind::Video,
            _ => MediaKind::File,
        }
    }

    /// The text the store keeps for the kind.
    pub fn stored(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
            MediaKind::File => "file",
        }
    }

    /// The kind a stored value stands for, or `None` for anything else.
    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "image" => Some(MediaKind::Image),
            "video" => Some(MediaKind::Video),
            "file" => Some(MediaKind::File),
            _ => None,
        }
    }
}

/// One file attached to a task. `stored_name` is the file's name in the attachments folder, unique across the
/// app, and `file_name` is the name the user gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub task: TaskId,
    pub stored_name: String,
    pub file_name: String,
    pub kind: MediaKind,
    pub created_at: Timestamp,
}

/// Storage for attachment rows. The files themselves are handled by the persistence layer.
pub trait AttachmentRepository {
    /// The task's attachments, oldest first.
    fn list_for_task(&self, task: TaskId) -> RepoResult<Vec<Attachment>>;
    fn add(&self, attachment: &Attachment) -> RepoResult<()>;
    fn remove(&self, task: TaskId, stored_name: &str) -> RepoResult<()>;
}

#[cfg(test)]
mod tests {
    use super::MediaKind;

    #[test]
    fn images_videos_and_other_files_are_told_apart_by_extension() {
        assert_eq!(MediaKind::from_file_name("screen.PNG"), MediaKind::Image);
        assert_eq!(MediaKind::from_file_name("clip.mov"), MediaKind::Video);
        assert_eq!(MediaKind::from_file_name("spec.pdf"), MediaKind::File);
        assert_eq!(MediaKind::from_file_name("no-extension"), MediaKind::File);
        assert_eq!(MediaKind::from_file_name("archive.tar.gz"), MediaKind::File);
    }

    #[test]
    fn a_kind_round_trips_through_its_stored_form() {
        for kind in [MediaKind::Image, MediaKind::Video, MediaKind::File] {
            assert_eq!(MediaKind::from_stored(kind.stored()), Some(kind));
        }
        assert_eq!(MediaKind::from_stored("audio"), None);
    }
}
