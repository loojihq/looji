//! The release check against a local server that answers with canned responses. The client code is the real client;
//! only the server is a stand-in, so nothing here reaches GitHub.

use std::time::Duration;

use looji_updates::{CheckError, UpdateStatus, fetch_latest};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn release(version: &str) -> String {
    format!(
        r#"{{"tag_name": "v{version}", "html_url": "https://github.com/danielkosgei/looji/releases/tag/v{version}", "draft": false, "prerelease": false}}"#
    )
}

/// Serves one response per connection, in order, and returns the address to ask.
async fn serve(responses: Vec<(u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let _ = socket.read(&mut request).await;
            let reply = format!(
                "HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        }
    });
    address
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_newer_release_on_the_server_is_offered() {
    let address = serve(vec![(200, release("0.2.0"))]).await;

    let status = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0")
        .await
        .unwrap();

    assert!(
        matches!(status, UpdateStatus::Available { ref version, .. } if version == "0.2.0"),
        "{status:?}"
    );
}

#[tokio::test]
async fn an_equal_release_on_the_server_is_up_to_date() {
    let address = serve(vec![(200, release("0.1.0"))]).await;

    let status = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0")
        .await
        .unwrap();

    assert_eq!(status, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn an_older_release_on_the_server_is_up_to_date() {
    let address = serve(vec![(200, release("0.0.5"))]).await;

    let status = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0")
        .await
        .unwrap();

    assert_eq!(status, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn a_repository_with_no_releases_is_up_to_date() {
    let address = serve(vec![(404, r#"{"message": "Not Found"}"#.to_string())]).await;

    let status = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0")
        .await
        .unwrap();

    assert_eq!(status, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn a_server_error_is_an_error_for_the_retry_to_handle() {
    let address = serve(vec![(503, "unavailable".to_string())]).await;

    let result = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0").await;

    assert_eq!(result, Err(CheckError::Status(503)));
}

#[tokio::test]
async fn an_unreadable_body_is_an_error_not_up_to_date() {
    let address = serve(vec![(200, "<html>login</html>".to_string())]).await;

    let result = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0").await;

    assert!(
        matches!(result, Err(CheckError::Malformed(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_network_failure_is_an_error_and_never_a_notice() {
    // Nothing listens on this port, so the connection is refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let result = fetch_latest(&client(), &format!("http://{address}/latest"), "0.1.0").await;

    assert!(matches!(result, Err(CheckError::Network(_))), "{result:?}");
}
