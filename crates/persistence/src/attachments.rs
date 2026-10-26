//! Attachment files: copy a picked file into the attachments folder under a unique name, and remove it again.
//!
//! Nothing here touches the database. The rows are written by `AttachmentRepository`, and a file is only copied
//! once the caller is ready to record it.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;
use uuid::Uuid;

/// Largest file accepted, in bytes. Videos are the big ones, so this is generous.
pub const MAX_ATTACHMENT_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttachmentError {
    #[error("the file is larger than 500 MB")]
    TooLarge,
    #[error("the file is empty")]
    Empty,
    #[error("the file could not be copied: {0}")]
    Io(String),
}

/// A file that has been copied into the attachments folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// The name the file has in the attachments folder.
    pub stored_name: String,
    /// The name the file had when it was picked.
    pub file_name: String,
}

/// The attachments folder inside the app data directory.
pub fn attachments_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("attachments")
}

/// Copies `source` into `dir` under a new unique name. Files that are empty or too large are refused before
/// anything is written, and a copy that fails is removed again.
pub fn import_attachment(source: &Path, dir: &Path) -> Result<StoredFile, AttachmentError> {
    let metadata = fs::metadata(source).map_err(|err| AttachmentError::Io(err.to_string()))?;
    if metadata.len() == 0 {
        return Err(AttachmentError::Empty);
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    fs::create_dir_all(dir).map_err(|err| AttachmentError::Io(err.to_string()))?;

    let stored_name = match extension(&file_name) {
        Some(extension) => format!("{}.{extension}", Uuid::new_v4()),
        None => Uuid::new_v4().to_string(),
    };
    let target = dir.join(&stored_name);
    if let Err(err) = fs::copy(source, &target) {
        let _ = fs::remove_file(&target);
        return Err(AttachmentError::Io(err.to_string()));
    }
    Ok(StoredFile {
        stored_name,
        file_name,
    })
}

/// Deletes an attachment's file. A file that is already gone counts as deleted.
pub fn remove_attachment(dir: &Path, stored_name: &str) -> io::Result<()> {
    match fs::remove_file(dir.join(stored_name)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The extension to keep on the stored name, lowercased. Only short letters and digits are kept, so a name can't
/// carry a path or a control character into the folder.
fn extension(file_name: &str) -> Option<String> {
    let (_, extension) = file_name.rsplit_once('.')?;
    let clean = extension.to_ascii_lowercase();
    let usable =
        !clean.is_empty() && clean.len() <= 8 && clean.chars().all(|c| c.is_ascii_alphanumeric());
    usable.then_some(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("looji-attachment-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_picked_file_is_copied_under_a_unique_name_that_keeps_its_extension() {
        let dir = scratch("copy");
        let source = dir.join("Screenshot.PNG");
        fs::write(&source, b"pixels").unwrap();
        let attachments = dir.join("attachments");

        let first = import_attachment(&source, &attachments).unwrap();
        let second = import_attachment(&source, &attachments).unwrap();

        assert_eq!(first.file_name, "Screenshot.PNG");
        assert!(first.stored_name.ends_with(".png"));
        assert_ne!(first.stored_name, second.stored_name);
        assert_eq!(
            fs::read(attachments.join(&first.stored_name)).unwrap(),
            b"pixels"
        );
        remove_attachment(&attachments, &first.stored_name).unwrap();
        assert!(!attachments.join(&first.stored_name).exists());
        // Removing a file twice is fine.
        remove_attachment(&attachments, &first.stored_name).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_and_missing_files_are_refused_before_anything_is_written() {
        let dir = scratch("refuse");
        let empty = dir.join("empty.txt");
        fs::write(&empty, b"").unwrap();
        let attachments = dir.join("attachments");

        assert_eq!(
            import_attachment(&empty, &attachments),
            Err(AttachmentError::Empty)
        );
        assert!(matches!(
            import_attachment(&dir.join("missing.mov"), &attachments),
            Err(AttachmentError::Io(_))
        ));
        assert!(!attachments.exists() || fs::read_dir(&attachments).unwrap().next().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_short_alphanumeric_extensions_are_kept_in_the_stored_name() {
        assert_eq!(extension("clip.MP4").as_deref(), Some("mp4"));
        assert_eq!(extension("notes"), None);
        assert_eq!(extension("odd.with space"), None);
        assert_eq!(extension("long.abcdefghijk"), None);
        assert_eq!(extension("trailing."), None);
    }
}
