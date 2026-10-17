//! Putting an update in place of the running program. The programs here are plain files, so what's checked is the
//! file handling: the new one takes the old one's name, the old one is kept, and every failure puts the old one back.

use std::fs;

use looji_updates::apply::{
    ApplyError, previous_path, remove_previous, replace_and_start, replace_executable, roll_back,
};
use tempfile::TempDir;

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_new_program_takes_the_old_ones_name_and_the_old_one_is_kept() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    let staged = dir.path().join("staged.exe");
    fs::write(&target, b"version one").unwrap();
    fs::write(&staged, b"version two, which is longer").unwrap();

    let replacement = replace_executable(&target, &staged).unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"version two, which is longer");
    assert_eq!(fs::read(&replacement.previous).unwrap(), b"version one");
    assert_eq!(replacement.previous, previous_path(&target));
}

#[test]
fn an_update_file_that_is_missing_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    fs::write(&target, b"version one").unwrap();

    let result = replace_executable(&target, &dir.path().join("missing.exe"));

    assert!(
        matches!(result, Err(ApplyError::StagedUnreadable(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(&target).unwrap(), b"version one");
    assert_eq!(names(dir.path()), ["looji.exe"], "no .old file was made");
}

#[test]
fn a_failed_copy_puts_the_previous_program_back() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    fs::write(&target, b"version one").unwrap();
    // A folder can be measured but not copied as a file, so the copy fails after the old program has been moved.
    let staged = dir.path().join("staged");
    fs::create_dir(&staged).unwrap();

    let result = replace_executable(&target, &staged);

    assert!(
        matches!(result, Err(ApplyError::CopyFailed(_))),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&target).unwrap(),
        b"version one",
        "the previous version is back under its own name"
    );
    assert!(!previous_path(&target).exists(), "nothing is left aside");
}

#[test]
fn a_new_program_that_cannot_start_is_rolled_back() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    let staged = dir.path().join("staged.exe");
    fs::write(&target, b"version one").unwrap();
    // Not a program, so the copy works and starting it doesn't.
    fs::write(&staged, b"this is not an executable").unwrap();

    let result = replace_and_start(&target, &staged, &[]);

    assert!(
        matches!(result, Err(ApplyError::LaunchFailed(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(&target).unwrap(), b"version one");
    assert!(!previous_path(&target).exists());
}

#[test]
fn a_new_program_that_starts_is_left_in_place_with_the_old_one_beside_it() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    fs::write(&target, b"version one").unwrap();
    // A real program, so starting it works. This test program itself will do.
    let staged = std::env::current_exe().unwrap();

    let replacement = replace_and_start(&target, &staged, &["--list".to_string()]).unwrap();

    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        fs::metadata(&staged).unwrap().len()
    );
    assert_eq!(fs::read(&replacement.previous).unwrap(), b"version one");
}

#[test]
fn rolling_back_restores_the_old_program() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    let staged = dir.path().join("staged.exe");
    fs::write(&target, b"version one").unwrap();
    fs::write(&staged, b"version two").unwrap();
    let replacement = replace_executable(&target, &staged).unwrap();

    roll_back(&replacement).unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"version one");
    assert!(!replacement.previous.exists());
}

#[test]
fn a_leftover_old_file_from_an_earlier_update_does_not_block_the_next() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    let staged = dir.path().join("staged.exe");
    fs::write(&target, b"version two").unwrap();
    fs::write(previous_path(&target), b"stale version one").unwrap();
    fs::write(&staged, b"version three").unwrap();

    let replacement = replace_executable(&target, &staged).unwrap();

    assert_eq!(fs::read(&replacement.previous).unwrap(), b"version two");
}

#[test]
fn the_old_file_is_removed_once_the_new_version_has_started_and_a_missing_one_is_fine() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("looji.exe");
    fs::write(&target, b"current").unwrap();
    fs::write(previous_path(&target), b"old").unwrap();

    remove_previous(&target).unwrap();
    remove_previous(&target).unwrap();

    assert_eq!(names(dir.path()), ["looji.exe"]);
}
