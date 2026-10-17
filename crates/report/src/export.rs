//! Writes a rendered report to a file. The bytes go to a temporary file in the same folder first, and that is renamed
//! over the target, so an interrupted save never leaves a half-written PDF. A target that can't be written, such as a
//! read-only file or a folder the user can't write to, is reported as such, so the message can say what to do.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{ReportError, ReportInput, render};

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("looji can't write to {}. Choose a folder you can write to, or a file that isn't read-only.", path.display())]
    NotWritable { path: PathBuf },
    #[error("the report could not be saved to {}: {reason}", path.display())]
    Failed { path: PathBuf, reason: String },
    #[error("{0}")]
    Render(#[from] ReportError),
}

/// Renders the report and writes it to `path`, returning the path it was written to.
pub fn write_report(input: &ReportInput, path: &Path) -> Result<PathBuf, ExportError> {
    // Linux would let the rename below replace a read-only file, so refuse one here on every platform.
    if fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly()) {
        return Err(ExportError::NotWritable {
            path: path.to_path_buf(),
        });
    }
    let bytes = render(input)?;
    let temp = temp_path(path);
    if let Err(err) = fs::write(&temp, &bytes) {
        let _ = fs::remove_file(&temp);
        return Err(classify(path, err));
    }
    if let Err(err) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(classify(path, err));
    }
    Ok(path.to_path_buf())
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report.pdf".to_string());
    path.with_file_name(format!(".{name}.looji-tmp"))
}

fn classify(path: &Path, err: io::Error) -> ExportError {
    match err.kind() {
        io::ErrorKind::PermissionDenied => ExportError::NotWritable {
            path: path.to_path_buf(),
        },
        _ => ExportError::Failed {
            path: path.to_path_buf(),
            reason: err.to_string(),
        },
    }
}
