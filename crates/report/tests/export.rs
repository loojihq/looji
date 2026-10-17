//! Saving a report: the file is written, a read-only target is refused with a message that says why, and nothing is
//! left behind in the folder either way.

use std::{fs, path::Path};

use looji_domain::group_by_status;
use looji_report::{ExportError, ReportInput, ReportKind, write_report};
use tempfile::TempDir;

fn input() -> ReportInput {
    ReportInput {
        project: "Website".to_string(),
        workspace: "Work".to_string(),
        kind: ReportKind::Published,
        description: String::new(),
        generated_on: "today".to_string(),
        sections: group_by_status(&[]),
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_report_is_written_to_the_chosen_file_and_no_temporary_file_is_left() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("website.pdf");

    let written = write_report(&input(), &target).unwrap();

    assert_eq!(written, target);
    let bytes = fs::read(&target).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(leftovers(dir.path()), ["website.pdf"]);
}

#[test]
fn a_read_only_target_is_refused_with_a_message_about_the_file() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("locked.pdf");
    fs::write(&target, b"existing").unwrap();
    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&target, permissions).unwrap();

    let err = write_report(&input(), &target).unwrap_err();

    assert!(matches!(err, ExportError::NotWritable { .. }), "{err:?}");
    assert!(err.to_string().contains("locked.pdf"), "{err}");
    assert_eq!(
        fs::read(&target).unwrap(),
        b"existing",
        "the existing file is untouched"
    );
    assert_eq!(
        leftovers(dir.path()),
        ["locked.pdf"],
        "no temporary file is left behind"
    );
}

#[test]
fn a_folder_that_does_not_exist_is_a_clear_failure() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("missing").join("report.pdf");

    let err = write_report(&input(), &target).unwrap_err();

    assert!(err.to_string().contains("report.pdf"), "{err}");
}
