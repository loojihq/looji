//! Checks GitHub for a newer release of looji. The check asks the latest release, compares its tag with the running
//! version by semver, and says whether there is something newer. A failed check says nothing to the user and is
//! tried again later.
//!
//! Nothing here touches the UI. The app runs `fetch_latest` on its runtime and shows the result, if any.

pub mod apply;
pub mod download;

use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use thiserror::Error;

/// The repository whose releases are checked.
pub const REPOSITORY: &str = "danielkosgei/looji";

/// How long a successful check is good for before the next one.
pub const CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
/// How long to wait after a failed check before trying again.
pub const RETRY_INTERVAL_MS: i64 = 60 * 60 * 1000;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    /// A release newer than the running version. `version` is without the leading `v`.
    Available {
        version: String,
        url: String,
        /// The file to download for this platform, with where to check it. `None` when the release has no file for
        /// this platform, in which case the user is only told about the release.
        artifact: Option<Artifact>,
    },
}

/// A release file for this platform, and the checksum list that says what it should contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub url: String,
    /// The size GitHub reports, when it reports one.
    pub size: Option<u64>,
    /// The `SHA256SUMS.txt` published with the release. Without it the file can't be verified, so it isn't used.
    pub checksums_url: Option<String>,
}

/// The name of the release file that replaces the running program on this platform, or `None` when updates can't be
/// applied here.
pub fn platform_asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("looji-windows-x64.exe")
    } else {
        None
    }
}

/// The name of the checksum list published with every release.
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";

/// Why a check didn't produce an answer. None of these are shown to the user; the check is retried later.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckError {
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub answered with status {0}")]
    Status(u16),
    #[error("the release could not be read: {0}")]
    Malformed(String),
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
}

fn pick_artifact(assets: &[Asset]) -> Option<Artifact> {
    let wanted = platform_asset_name()?;
    let file = assets.iter().find(|asset| asset.name == wanted)?;
    Some(Artifact {
        name: file.name.clone(),
        url: file.browser_download_url.clone(),
        size: file.size,
        checksums_url: assets
            .iter()
            .find(|asset| asset.name == CHECKSUMS_ASSET)
            .map(|asset| asset.browser_download_url.clone()),
    })
}

/// Reads the body of a latest-release response and compares it with the running version. Drafts and prereleases
/// are never offered, since the latest-release endpoint doesn't return them and a newer tag that is one is not an
/// update for most users.
pub fn interpret(body: &str, current: &str) -> Result<UpdateStatus, CheckError> {
    let release: Release =
        serde_json::from_str(body).map_err(|err| CheckError::Malformed(err.to_string()))?;
    if release.draft || release.prerelease {
        return Ok(UpdateStatus::UpToDate);
    }
    let latest = parse_version(&release.tag_name)?;
    let running = Version::parse(current).map_err(|err| CheckError::Malformed(err.to_string()))?;
    if latest > running {
        Ok(UpdateStatus::Available {
            version: latest.to_string(),
            url: release.html_url,
            artifact: pick_artifact(&release.assets),
        })
    } else {
        Ok(UpdateStatus::UpToDate)
    }
}

/// A tag such as `v1.2.3` or `1.2.3`, read as a version.
pub fn parse_version(tag: &str) -> Result<Version, CheckError> {
    Version::parse(tag.trim().trim_start_matches('v'))
        .map_err(|err| CheckError::Malformed(format!("{tag:?}: {err}")))
}

/// Asks GitHub for the latest release and compares it with `current`. A repository with no releases answers 404,
/// which is read as up to date.
pub async fn fetch_latest(
    client: &reqwest::Client,
    url: &str,
    current: &str,
) -> Result<UpdateStatus, CheckError> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, format!("looji/{current}"))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|err| CheckError::Network(err.to_string()))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateStatus::UpToDate);
    }
    if !status.is_success() {
        return Err(CheckError::Status(status.as_u16()));
    }
    let body = response
        .text()
        .await
        .map_err(|err| CheckError::Network(err.to_string()))?;
    interpret(&body, current)
}

/// When the last check happened, for deciding whether the next one is due.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Schedule {
    pub last_success_ms: Option<i64>,
    pub last_failure_ms: Option<i64>,
}

/// Whether a check should run now. A successful check is good for a day. After a failure, the next attempt waits
/// an hour, and a failure doesn't reset the day from the last success.
pub fn is_due(schedule: Schedule, now_ms: i64) -> bool {
    let day_passed = schedule
        .last_success_ms
        .is_none_or(|last| now_ms - last >= CHECK_INTERVAL_MS);
    if !day_passed {
        return false;
    }
    let waiting_to_retry = schedule.last_failure_ms.is_some_and(|failed| {
        schedule.last_success_ms.is_none_or(|ok| failed > ok) && now_ms - failed < RETRY_INTERVAL_MS
    });
    !waiting_to_retry
}

/// Checks `url` with a client of its own, so a caller only needs the runtime. Use this from the app.
pub async fn check(url: &str, current: &str) -> Result<UpdateStatus, CheckError> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|err| CheckError::Network(err.to_string()))?;
    fetch_latest(&client, url, current).await
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{CHECK_INTERVAL_MS, RETRY_INTERVAL_MS, Schedule, UpdateStatus, interpret, is_due};

    const BODY: &str = r#"{"tag_name": "v%VERSION%", "html_url": "https://github.com/danielkosgei/looji/releases/tag/v%VERSION%", "draft": false, "prerelease": false}"#;

    fn body(version: &str) -> String {
        BODY.replace("%VERSION%", version)
    }

    #[test]
    fn a_newer_release_is_offered_with_its_page() {
        let status = interpret(&body("0.2.0"), "0.1.0").unwrap();
        assert_eq!(
            status,
            UpdateStatus::Available {
                version: "0.2.0".to_string(),
                url: "https://github.com/danielkosgei/looji/releases/tag/v0.2.0".to_string(),
                artifact: None,
            }
        );
    }

    #[test]
    fn an_equal_release_is_up_to_date() {
        assert_eq!(
            interpret(&body("0.1.0"), "0.1.0").unwrap(),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn an_older_release_is_up_to_date_not_a_downgrade() {
        assert_eq!(
            interpret(&body("0.0.9"), "0.1.0").unwrap(),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn versions_compare_by_semver_not_text() {
        // As text, "0.10.0" sorts before "0.9.0". By semver it is newer.
        assert!(matches!(
            interpret(&body("0.10.0"), "0.9.0").unwrap(),
            UpdateStatus::Available { .. }
        ));
        assert_eq!(
            interpret(&body("0.9.0"), "0.10.0").unwrap(),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn a_prerelease_or_draft_is_not_offered() {
        let prerelease = body("0.2.0").replace("\"prerelease\": false", "\"prerelease\": true");
        let draft = body("0.2.0").replace("\"draft\": false", "\"draft\": true");
        assert_eq!(
            interpret(&prerelease, "0.1.0").unwrap(),
            UpdateStatus::UpToDate
        );
        assert_eq!(interpret(&draft, "0.1.0").unwrap(), UpdateStatus::UpToDate);
    }

    #[test]
    fn an_unreadable_release_is_an_error_not_up_to_date() {
        assert!(interpret("<html>maintenance</html>", "0.1.0").is_err());
        assert!(interpret(&body("not-a-version"), "0.1.0").is_err());
    }

    #[test]
    fn a_check_is_due_when_none_has_succeeded() {
        assert!(is_due(Schedule::default(), 1_000));
    }

    #[test]
    fn a_successful_check_is_not_repeated_within_a_day() {
        let schedule = Schedule {
            last_success_ms: Some(0),
            last_failure_ms: None,
        };
        assert!(!is_due(schedule, CHECK_INTERVAL_MS - 1));
        assert!(is_due(schedule, CHECK_INTERVAL_MS));
    }

    #[test]
    fn a_failed_check_is_retried_after_an_hour_and_not_sooner() {
        let schedule = Schedule {
            last_success_ms: None,
            last_failure_ms: Some(0),
        };
        assert!(!is_due(schedule, RETRY_INTERVAL_MS - 1));
        assert!(is_due(schedule, RETRY_INTERVAL_MS));
    }

    #[test]
    fn a_failure_after_a_success_still_waits_for_the_day() {
        let schedule = Schedule {
            last_success_ms: Some(0),
            last_failure_ms: Some(CHECK_INTERVAL_MS - 10),
        };
        assert!(
            !is_due(schedule, CHECK_INTERVAL_MS - 5),
            "the day from the success hasn't passed"
        );
        assert!(is_due(schedule, CHECK_INTERVAL_MS + RETRY_INTERVAL_MS));
    }
}
