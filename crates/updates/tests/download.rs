//! Downloading and verifying a release file against a local server. The server can honour or ignore a request for the
//! rest of a file, send a body that doesn't match its checksum, and drop the connection part way, so each way a
//! download goes wrong has a test. The client is the real client; only the server is a stand-in.

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use looji_updates::{
    Artifact,
    download::{DownloadError, Progress, download_verified, file_sha256},
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const NAME: &str = "looji-windows-x64.exe";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What the stand-in server does with the file. `body` is what it sends.
struct Behaviour {
    body: Vec<u8>,
    /// Whether a `Range` request is answered with the rest of the file.
    honour_range: bool,
    /// Sends only this many bytes of the next response, then closes the connection.
    cut_after: Option<usize>,
    /// The `Range` header of each request for the file, in order.
    ranges: Vec<Option<String>>,
    checksums: String,
}

type Shared = Arc<Mutex<Behaviour>>;

async fn serve(behaviour: Shared) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let behaviour = Arc::clone(&behaviour);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let range = request.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range:")
                        .map(|v| v.trim().to_string())
                });

                let (status, headers, body, cut) = {
                    let mut state = behaviour.lock().unwrap();
                    if path == "/sums" {
                        (
                            "200 OK",
                            String::new(),
                            state.checksums.clone().into_bytes(),
                            None,
                        )
                    } else {
                        state.ranges.push(range.clone());
                        let start = range
                            .as_deref()
                            .and_then(|value| value.strip_prefix("bytes="))
                            .and_then(|value| value.strip_suffix('-'))
                            .and_then(|value| value.parse::<usize>().ok());
                        let total = state.body.len();
                        let cut = state.cut_after.take();
                        match start {
                            Some(start) if state.honour_range && start >= total => {
                                ("416 Range Not Satisfiable", String::new(), Vec::new(), None)
                            }
                            Some(start) if state.honour_range => (
                                "206 Partial Content",
                                format!("Content-Range: bytes {start}-{}/{total}\r\n", total - 1),
                                state.body[start..].to_vec(),
                                cut,
                            ),
                            _ => ("200 OK", String::new(), state.body.clone(), cut),
                        }
                    }
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                let sent = cut.map_or(body.len(), |cut| cut.min(body.len()));
                let _ = socket.write_all(&body[..sent]).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    address
}

fn artifact(address: &str, size: Option<u64>) -> Artifact {
    Artifact {
        name: NAME.to_string(),
        url: format!("http://{address}/file"),
        size,
        checksums_url: Some(format!("http://{address}/sums")),
    }
}

fn setup(body: Vec<u8>, checksum_of: &[u8]) -> Shared {
    Arc::new(Mutex::new(Behaviour {
        body,
        honour_range: true,
        cut_after: None,
        ranges: Vec::new(),
        checksums: format!("{}  {NAME}\n", sha256_hex(checksum_of)),
    }))
}

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn a_file_that_matches_its_checksum_is_accepted_with_progress() {
    let body = sample(200_000);
    let behaviour = setup(body.clone(), &body);
    let address = serve(behaviour).await;
    let dir = TempDir::new().unwrap();
    let mut seen: Vec<Progress> = Vec::new();

    let path = download_verified(
        &client(),
        &artifact(&address, Some(body.len() as u64)),
        dir.path(),
        |p| seen.push(p),
    )
    .await
    .unwrap();

    assert_eq!(fs::read(&path).unwrap(), body);
    assert_eq!(names(dir.path()), [NAME], "no part file is left");
    assert!(
        seen.windows(2)
            .all(|pair| pair[0].downloaded <= pair[1].downloaded),
        "progress only goes forward"
    );
    let last = seen.last().unwrap();
    assert_eq!(
        (last.downloaded, last.total),
        (body.len() as u64, Some(body.len() as u64))
    );
}

#[tokio::test]
async fn a_tampered_file_is_rejected_and_deleted() {
    let genuine = sample(50_000);
    let mut tampered = genuine.clone();
    tampered[1234] ^= 0xFF;
    // The checksum list is for the genuine file, but the server sends the tampered one.
    let behaviour = setup(tampered, &genuine);
    let address = serve(behaviour).await;
    let dir = TempDir::new().unwrap();

    let result = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {}).await;

    assert!(
        matches!(result, Err(DownloadError::ChecksumMismatch { .. })),
        "{result:?}"
    );
    assert!(
        names(dir.path()).is_empty(),
        "nothing is left to run: {:?}",
        names(dir.path())
    );
}

#[tokio::test]
async fn a_verified_file_that_is_later_tampered_with_is_not_reused() {
    let body = sample(30_000);
    let behaviour = setup(body.clone(), &body);
    let address = serve(Arc::clone(&behaviour)).await;
    let dir = TempDir::new().unwrap();
    let first = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {})
        .await
        .unwrap();
    fs::write(&first, b"replaced by something else").unwrap();

    let second = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {})
        .await
        .unwrap();

    assert_eq!(
        fs::read(&second).unwrap(),
        body,
        "the file was downloaded again"
    );
    assert_eq!(behaviour.lock().unwrap().ranges.len(), 2);
}

#[tokio::test]
async fn an_interrupted_download_keeps_its_part_and_resumes_from_it() {
    let body = sample(120_000);
    let behaviour = setup(body.clone(), &body);
    behaviour.lock().unwrap().cut_after = Some(40_000);
    let address = serve(Arc::clone(&behaviour)).await;
    let dir = TempDir::new().unwrap();
    let artifact = artifact(&address, Some(body.len() as u64));

    let first = download_verified(&client(), &artifact, dir.path(), |_| {}).await;

    assert!(
        matches!(first, Err(DownloadError::Interrupted(_))),
        "{first:?}"
    );
    let part = dir.path().join(format!("{NAME}.part"));
    assert_eq!(
        fs::metadata(&part).unwrap().len(),
        40_000,
        "what arrived is kept"
    );

    let mut seen: Vec<Progress> = Vec::new();
    let path = download_verified(&client(), &artifact, dir.path(), |p| seen.push(p))
        .await
        .unwrap();

    assert_eq!(fs::read(&path).unwrap(), body);
    let ranges = behaviour.lock().unwrap().ranges.clone();
    assert_eq!(
        ranges,
        [None, Some("bytes=40000-".to_string())],
        "the second request asked for the rest"
    );
    assert_eq!(
        seen.first().unwrap().downloaded,
        40_000,
        "progress counts what was already there"
    );
    assert_eq!(names(dir.path()), [NAME]);
}

#[tokio::test]
async fn a_server_that_ignores_the_range_restarts_the_download_cleanly() {
    let body = sample(90_000);
    let behaviour = setup(body.clone(), &body);
    {
        let mut state = behaviour.lock().unwrap();
        state.cut_after = Some(25_000);
        state.honour_range = false;
    }
    let address = serve(Arc::clone(&behaviour)).await;
    let dir = TempDir::new().unwrap();
    let artifact = artifact(&address, Some(body.len() as u64));
    assert!(
        download_verified(&client(), &artifact, dir.path(), |_| {})
            .await
            .is_err()
    );

    let path = download_verified(&client(), &artifact, dir.path(), |_| {})
        .await
        .unwrap();

    assert_eq!(
        fs::read(&path).unwrap(),
        body,
        "the file isn't the old part with the whole file glued on"
    );
}

#[tokio::test]
async fn a_part_that_is_longer_than_the_file_is_discarded() {
    let body = sample(10_000);
    let behaviour = setup(body.clone(), &body);
    let address = serve(behaviour).await;
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(format!("{NAME}.part")), sample(50_000)).unwrap();

    let path = download_verified(
        &client(),
        &artifact(&address, Some(body.len() as u64)),
        dir.path(),
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(fs::read(&path).unwrap(), body);
}

#[tokio::test]
async fn a_corrupt_part_that_resumes_into_a_wrong_file_is_deleted_by_the_checksum() {
    let body = sample(60_000);
    let behaviour = setup(body.clone(), &body);
    let address = serve(behaviour).await;
    let dir = TempDir::new().unwrap();
    // A part of the right length whose bytes are not the start of the file.
    fs::write(
        dir.path().join(format!("{NAME}.part")),
        vec![0xAAu8; 20_000],
    )
    .unwrap();

    let result = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {}).await;

    assert!(
        matches!(result, Err(DownloadError::ChecksumMismatch { .. })),
        "{result:?}"
    );
    assert!(
        names(dir.path()).is_empty(),
        "the bad part is gone, so the next attempt starts fresh"
    );

    let again = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {})
        .await
        .unwrap();
    assert_eq!(file_sha256(&again).unwrap(), sha256_hex(&body));
}

#[tokio::test]
async fn a_release_without_a_checksum_for_the_file_is_refused_before_downloading() {
    let body = sample(1_000);
    let behaviour = setup(body.clone(), &body);
    behaviour.lock().unwrap().checksums = format!("{}  some-other-file.exe\n", sha256_hex(&body));
    let address = serve(Arc::clone(&behaviour)).await;
    let dir = TempDir::new().unwrap();

    let result = download_verified(&client(), &artifact(&address, None), dir.path(), |_| {}).await;

    assert!(
        matches!(result, Err(DownloadError::NoChecksum(_))),
        "{result:?}"
    );
    assert!(
        behaviour.lock().unwrap().ranges.is_empty(),
        "the file was never requested"
    );
}

#[tokio::test]
async fn a_release_with_no_checksum_list_is_refused() {
    let dir = TempDir::new().unwrap();
    let artifact = Artifact {
        name: NAME.to_string(),
        url: "http://127.0.0.1:1/file".to_string(),
        size: None,
        checksums_url: None,
    };

    let result = download_verified(&client(), &artifact, dir.path(), |_| {}).await;

    assert!(
        matches!(result, Err(DownloadError::NoChecksums)),
        "{result:?}"
    );
}
