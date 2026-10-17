//! Downloads a release file and checks it before it's accepted. The file's expected SHA-256 comes from the checksum list
//! published with the release. A file that doesn't match is deleted, and a release without a checksum for the file is
//! refused, so an unverified file is never handed on.
//!
//! The download goes to a `.part` file. If it's interrupted, the part is kept and the next attempt asks the server for
//! the rest. A server that ignores the request and sends the whole file again is handled by starting over.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::Artifact;

/// How far a download has got. `total` is `None` when the server didn't say how big the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("this release has no checksum list, so the file can't be checked")]
    NoChecksums,
    #[error("the checksum list has no entry for {0}")]
    NoChecksum(String),
    #[error("the download was interrupted ({0}). It will carry on from where it stopped.")]
    Interrupted(String),
    #[error("the downloaded file doesn't match its checksum, so it was deleted")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("the server answered with status {0}")]
    Status(u16),
    #[error("could not use {}: {reason}", path.display())]
    Storage { path: PathBuf, reason: String },
}

/// The entries of a `SHA256SUMS.txt`: lines of a 64-digit hash, then the file name. A `*` before the name marks a
/// binary file and is ignored. Lines that don't look like that are skipped.
pub fn parse_checksums(text: &str) -> HashMap<String, String> {
    let mut sums = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((hash, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim().trim_start_matches('*');
        if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty() {
            sums.insert(name.to_string(), hash.to_ascii_lowercase());
        }
    }
    sums
}

/// The SHA-256 of a file, as lowercase hex, read in blocks so a large file isn't held in memory.
pub fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut block)?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn storage(path: &Path, err: io::Error) -> DownloadError {
    DownloadError::Storage {
        path: path.to_path_buf(),
        reason: err.to_string(),
    }
}

/// Downloads `artifact` into `dir` and returns the path of the verified file. `on_progress` is told how many bytes
/// have arrived, counting those from an earlier attempt that is being resumed.
pub async fn download_verified(
    client: &reqwest::Client,
    artifact: &Artifact,
    dir: &Path,
    mut on_progress: impl FnMut(Progress),
) -> Result<PathBuf, DownloadError> {
    let checksums_url = artifact
        .checksums_url
        .as_deref()
        .ok_or(DownloadError::NoChecksums)?;
    let sums = parse_checksums(&fetch_text(client, checksums_url).await?);
    let expected = sums
        .get(&artifact.name)
        .cloned()
        .ok_or_else(|| DownloadError::NoChecksum(artifact.name.clone()))?;

    fs::create_dir_all(dir).map_err(|err| storage(dir, err))?;
    let final_path = dir.join(&artifact.name);
    let part = dir.join(format!("{}.part", artifact.name));

    // A file from an earlier run can be used if it still matches. Otherwise it's in the way.
    if final_path.exists() {
        match file_sha256(&final_path) {
            Ok(actual) if actual == expected => {
                let size = fs::metadata(&final_path)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                on_progress(Progress {
                    downloaded: size,
                    total: Some(size),
                });
                return Ok(final_path);
            }
            _ => {
                let _ = fs::remove_file(&final_path);
            }
        }
    }

    let mut restarted = false;
    loop {
        let mut have = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
        // A part as big as the whole file can't be a prefix of it, so it can't be resumed.
        if artifact.size.is_some_and(|size| have >= size) {
            let _ = fs::remove_file(&part);
            have = 0;
        }

        let mut request = client.get(&artifact.url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut response = request
            .send()
            .await
            .map_err(|err| DownloadError::Interrupted(err.to_string()))?;
        let status = response.status();

        // 416: the part is longer than the file, or the file changed. Start over once.
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && have > 0 && !restarted {
            let _ = fs::remove_file(&part);
            restarted = true;
            continue;
        }
        if !status.is_success() {
            return Err(DownloadError::Status(status.as_u16()));
        }

        let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT;
        let (mut file, mut downloaded) = if resuming {
            let file = OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(|err| storage(&part, err))?;
            (file, have)
        } else {
            // The server sent the whole file, whatever was asked, so what's here is replaced.
            (File::create(&part).map_err(|err| storage(&part, err))?, 0)
        };
        let total = response
            .content_length()
            .map(|length| length + downloaded)
            .or(artifact.size);
        on_progress(Progress { downloaded, total });

        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk).map_err(|err| storage(&part, err))?;
                    downloaded += chunk.len() as u64;
                    on_progress(Progress { downloaded, total });
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = file.flush();
                    return Err(DownloadError::Interrupted(err.to_string()));
                }
            }
        }
        file.flush().map_err(|err| storage(&part, err))?;
        drop(file);

        // The connection can close early without an error. What arrived is kept for the next attempt.
        if let Some(total) = total
            && downloaded < total
        {
            return Err(DownloadError::Interrupted(
                "the server closed the connection before the file was complete".to_string(),
            ));
        }

        let actual = file_sha256(&part).map_err(|err| storage(&part, err))?;
        if actual != expected {
            let _ = fs::remove_file(&part);
            return Err(DownloadError::ChecksumMismatch { expected, actual });
        }
        let _ = fs::remove_file(&final_path);
        fs::rename(&part, &final_path).map_err(|err| storage(&final_path, err))?;
        return Ok(final_path);
    }
}

/// Downloads and verifies `artifact` with a client of its own. A connection that stalls is given up on, but a long
/// download is not, since the file can be large and the connection slow.
pub async fn download(
    artifact: &Artifact,
    dir: &Path,
    on_progress: impl FnMut(Progress),
) -> Result<PathBuf, DownloadError> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| DownloadError::Interrupted(err.to_string()))?;
    download_verified(&client, artifact, dir, on_progress).await
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, DownloadError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| DownloadError::Interrupted(err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::Status(status.as_u16()));
    }
    response
        .text()
        .await
        .map_err(|err| DownloadError::Interrupted(err.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::download::parse_checksums;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn checksum_lines_are_read_by_file_name() {
        let text =
            format!("{HASH}  looji-windows-x64.exe\n{HASH} *other.zip\nnot a checksum line\n");
        let sums = parse_checksums(&text);
        assert_eq!(
            sums.get("looji-windows-x64.exe").map(String::as_str),
            Some(HASH)
        );
        assert_eq!(sums.get("other.zip").map(String::as_str), Some(HASH));
        assert_eq!(sums.len(), 2);
    }

    #[test]
    fn a_hash_of_the_wrong_length_is_ignored() {
        assert!(parse_checksums("abc123  file.exe").is_empty());
    }

    #[test]
    fn upper_case_hashes_are_compared_as_lower_case() {
        let text = format!("{}  file.exe", HASH.to_ascii_uppercase());
        assert_eq!(
            parse_checksums(&text).get("file.exe").map(String::as_str),
            Some(HASH)
        );
    }
}
