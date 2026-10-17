//! WSL repository folders against a real distribution.
//!
//! Tests that need a running distribution are ignored by default. To run them, start a distribution, then point
//! `LOOJI_WSL_REPO` at a git repository inside it, as a `\\wsl.localhost\…` path:
//!
//!     cargo test -p looji-persistence --test wsl_folder -- --ignored
//!
//! The stopped-distribution test has its own precondition: run it while the distribution is stopped.

use std::path::Path;

use looji_domain::{PathCheck, parse_wsl_path};
use looji_persistence::{
    repo_folder::{check_repo_folder, read_repo_file},
    wsl::{probe_through_distro, read_through_distro},
};

fn repo_from_env() -> String {
    std::env::var("LOOJI_WSL_REPO")
        .expect("set LOOJI_WSL_REPO to a git repository in a running WSL distribution")
}

#[test]
fn a_distribution_that_is_not_installed_is_named_in_the_error() {
    let result = check_repo_folder(Path::new(r"\\wsl.localhost\looji-no-such-distro\repo"));
    assert!(
        matches!(
            result,
            Err(PathCheck::DistroNotFound { .. }) | Err(PathCheck::WslUnavailable(_))
        ),
        "{result:?}"
    );
}

#[test]
#[ignore = "needs a running WSL distribution; see the module docs"]
fn a_repository_inside_a_running_distribution_is_accepted_and_readable() {
    let repo = repo_from_env();
    assert_eq!(check_repo_folder(Path::new(&repo)), Ok(()));

    let head = read_repo_file(&Path::new(&repo).join(".git").join("HEAD")).unwrap();
    assert!(
        String::from_utf8_lossy(&head).starts_with("ref:"),
        "unexpected HEAD: {head:?}"
    );
}

#[test]
#[ignore = "needs a running WSL distribution; see the module docs"]
fn a_missing_subfolder_inside_a_distribution_is_missing() {
    let repo = repo_from_env();
    let subfolder = format!("{repo}\\no-such-subfolder");
    assert_eq!(
        check_repo_folder(Path::new(&subfolder)),
        Err(PathCheck::Missing)
    );
}

#[test]
#[ignore = "needs a running WSL distribution; see the module docs"]
fn probing_through_the_distribution_answers_as_linux_sees_the_path() {
    let wsl = parse_wsl_path(&repo_from_env()).expect("LOOJI_WSL_REPO must be a WSL path");
    assert_eq!(probe_through_distro(&wsl.distro, &wsl.linux_path), Ok(()));
    assert_eq!(
        probe_through_distro(&wsl.distro, &format!("{}/no-such", wsl.linux_path)),
        Err(PathCheck::Missing)
    );
    assert_eq!(
        probe_through_distro(&wsl.distro, "/usr"),
        Err(PathCheck::NotAGitRepository)
    );
}

#[test]
#[ignore = "needs a running WSL distribution; see the module docs"]
fn a_file_read_through_the_distribution_matches_the_file() {
    let wsl = parse_wsl_path(&repo_from_env()).expect("LOOJI_WSL_REPO must be a WSL path");
    let head = read_through_distro(&wsl.distro, &format!("{}/.git/HEAD", wsl.linux_path)).unwrap();
    assert!(
        String::from_utf8_lossy(&head).starts_with("ref:"),
        "unexpected HEAD: {head:?}"
    );
}

/// Run this while the distribution is stopped. The check must report it and leave it stopped.
#[test]
#[ignore = "needs a stopped WSL distribution; see the module docs"]
fn a_stopped_distribution_is_reported_and_stays_stopped() {
    let repo = repo_from_env();
    let wsl = parse_wsl_path(&repo).expect("LOOJI_WSL_REPO must be a WSL path");
    assert_eq!(
        check_repo_folder(Path::new(&repo)),
        Err(PathCheck::DistroStopped {
            distro: wsl.distro.clone()
        })
    );
    let running = std::process::Command::new("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&running.stdout).replace('\0', "");
    assert!(
        !text.contains(&wsl.distro),
        "the check started the distribution: {text:?}"
    );
}
