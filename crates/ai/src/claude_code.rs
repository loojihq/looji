//! Claude Code, run as a local command. The request goes to `claude` in print mode on its standard input, and the
//! answer is read from its standard output as it's written, so tokens stream as they arrive. Nothing is stored
//! here: the command uses the sign-in the user already has with Claude Code.

use std::process::Stdio;

use futures_util::future::BoxFuture;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

use crate::{
    AiError, ChatEvent, ChatRequest,
    provider::{ChatProvider, EventStream, Producer, open_stream},
};

/// Bytes read from the command at a time.
const READ_SIZE: usize = 4096;

/// Claude Code as a provider. It runs `claude -p` once per request.
pub struct ClaudeCodeProvider;

impl ClaudeCodeProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the `claude` command can be found on this machine.
pub fn installed() -> bool {
    let finder = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(finder)
        .arg("claude")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The command for `claude`. On Windows it's installed as a script, which only a shell can start.
fn command(model: &str) -> Command {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "claude"]);
        command
    } else {
        Command::new("claude")
    };
    command
        .args(["-p", "--output-format", "text", "--model", model])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

/// The request as one prompt. The system text comes first, then each message in order.
fn prompt_for(request: &ChatRequest) -> String {
    let mut parts = Vec::new();
    if let Some(system) = &request.system {
        parts.push(system.clone());
    }
    parts.extend(
        request
            .messages
            .iter()
            .map(|message| message.content.clone()),
    );
    parts.join("\n\n")
}

impl ChatProvider for ClaudeCodeProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>> {
        Box::pin(async move {
            let prompt = prompt_for(&request);
            let mut child = command(&request.model).spawn().map_err(|err| {
                AiError::CommandFailed(format!("Claude Code did not start: {err}"))
            })?;
            // The prompt goes in on standard input, so its text never reaches the command line.
            if let Some(mut stdin) = child.stdin.take() {
                let written = stdin.write_all(prompt.as_bytes()).await;
                drop(stdin);
                if let Err(err) = written {
                    let _ = child.start_kill();
                    return Err(AiError::CommandFailed(format!(
                        "Claude Code did not take the request: {err}"
                    )));
                }
            }
            let (producer, stream) = open_stream(&cancel);
            tokio::spawn(read_output(child, producer));
            Ok(Box::pin(stream) as EventStream)
        })
    }
}

/// Sends the command's output as tokens until it ends, or until the stream is stopped, which kills the command.
async fn read_output(mut child: Child, out: Producer) {
    let Some(mut stdout) = child.stdout.take() else {
        out.send(Err(AiError::CommandFailed(
            "Claude Code gave no output stream".to_string(),
        )))
        .await;
        return;
    };
    let mut buffer = [0u8; READ_SIZE];
    // A character can be split between two reads, so the bytes that aren't a whole character yet wait here.
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let read = tokio::select! {
            biased;
            _ = out.stopped() => {
                let _ = child.start_kill();
                return;
            }
            read = stdout.read(&mut buffer) => read,
        };
        match read {
            Ok(0) => break,
            Ok(count) => {
                pending.extend_from_slice(&buffer[..count]);
                let complete = match std::str::from_utf8(&pending) {
                    Ok(text) => text.len(),
                    Err(err) => err.valid_up_to(),
                };
                if complete > 0 {
                    let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
                    pending.drain(..complete);
                    if !out.send(Ok(ChatEvent::Token(text))).await {
                        let _ = child.start_kill();
                        return;
                    }
                }
            }
            Err(err) => {
                out.send(Err(AiError::CommandFailed(format!(
                    "Claude Code stopped while answering: {err}"
                ))))
                .await;
                return;
            }
        }
    }
    if !pending.is_empty() {
        let text = String::from_utf8_lossy(&pending).into_owned();
        out.send(Ok(ChatEvent::Token(text))).await;
    }
    match child.wait().await {
        Ok(status) if status.success() => {
            out.send(Ok(ChatEvent::Finished { usage: None })).await;
        }
        Ok(status) => {
            out.send(Err(AiError::CommandFailed(format!(
                "Claude Code finished without an answer (exit {}). Check that it is signed in.",
                status.code().unwrap_or(-1)
            ))))
            .await;
        }
        Err(err) => {
            out.send(Err(AiError::CommandFailed(format!(
                "Claude Code did not report its result: {err}"
            ))))
            .await;
        }
    }
}
