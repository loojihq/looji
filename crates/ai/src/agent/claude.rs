//! Claude Code as an agent. It runs `claude -p` inside the repository, with only reading tools: its own `Read`, `Grep`
//! and `Glob`, and looji's knowledge tools, which it reaches through an MCP server that is looji itself started with
//! `--knowledge-mcp`. It can't edit files or run commands, and it doesn't save the session. The answer, and what it
//! did on the way, are read from its `stream-json` output.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
};

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

use super::describe_tool_use;
use crate::{
    AiError, ChatEvent, ChatRequest,
    provider::{ChatProvider, EventStream, Producer, open_stream},
    types::{ChatRole, Source},
};

/// The most turns Claude Code takes for a quick answer, such as a chat reply. Longer work, such as writing a
/// blueprint, asks for more through `max_turns`.
pub const QUICK_TURNS: u32 = 12;
/// The most turns for work that reads a lot before it answers.
pub const DEEP_TURNS: u32 = 40;

/// What the agent may use: reading, and looji's knowledge tools.
const ALLOWED_TOOLS: &str = "Read,Grep,Glob,mcp__looji__search,mcp__looji__read_file,mcp__looji__find_symbol,mcp__looji__find_references,mcp__looji__list_issues,mcp__looji__get_issue,mcp__looji__recent_changes,mcp__looji__repo_map";

/// What it may not touch even by reading: files whose names say they hold secrets. Claude Code applies these to its
/// file search too, as well as it can. Writing and running commands aren't offered at all.
const DENIED_TOOLS: &str = "Read(**/.env),Read(**/.env.*),Read(**/*.pem),Read(**/*.key),Read(**/*.pfx),Read(**/id_rsa*),Read(**/id_ed25519*),Read(**/*secret*),Read(**/credentials*),Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch";

/// Claude Code, set up to explore one repository.
pub struct ClaudeCodeAgent {
    /// The repository. Claude Code runs there, so its file tools see the code.
    pub root: PathBuf,
    /// The program that serves the knowledge tools: looji itself.
    pub server: PathBuf,
    /// Its arguments up to the sources log: `--knowledge-mcp <database> <project> <repository>`.
    pub server_args: Vec<String>,
    /// The most turns it may take before it must answer.
    pub max_turns: u32,
}

/// What one line of Claude Code's output means for the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Observation {
    /// A tool is being used.
    Step(String),
    /// A file is being read, so the answer can cite it.
    File { path: String, start: u32, end: u32 },
    /// A tool answered, so looji's tools may have logged sources.
    ToolAnswered,
    /// The answer.
    Answer(String),
    /// The run failed, with Claude Code's own account.
    Failure(String),
}

/// Reading the output a line at a time. A tool call arrives as its own event for each call, so the ones already seen
/// are remembered.
#[derive(Debug, Default)]
pub(crate) struct StreamReader {
    seen: HashSet<String>,
    /// Text from messages that had no tool call, which becomes the answer if the result doesn't carry one.
    pending: String,
}

impl StreamReader {
    pub(crate) fn read_line(&mut self, line: &str) -> Vec<Observation> {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match event["type"].as_str() {
            Some("assistant") => self.assistant(&event["message"]["content"]),
            Some("user") => {
                let answered = event["message"]["content"]
                    .as_array()
                    .is_some_and(|blocks| {
                        blocks.iter().any(|block| block["type"] == "tool_result")
                    });
                if answered {
                    vec![Observation::ToolAnswered]
                } else {
                    Vec::new()
                }
            }
            Some("result") => {
                let text = event["result"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let failed = event["is_error"].as_bool().unwrap_or(false)
                    || event["subtype"].as_str() != Some("success");
                if failed {
                    let reason = if text.is_empty() {
                        format!(
                            "Claude Code stopped ({}).",
                            event["subtype"].as_str().unwrap_or("no reason given")
                        )
                    } else {
                        text
                    };
                    vec![Observation::Failure(reason)]
                } else if text.is_empty() {
                    vec![Observation::Answer(
                        std::mem::take(&mut self.pending).trim().to_string(),
                    )]
                } else {
                    vec![Observation::Answer(text)]
                }
            }
            _ => Vec::new(),
        }
    }

    fn assistant(&mut self, content: &Value) -> Vec<Observation> {
        let mut found = Vec::new();
        let Some(blocks) = content.as_array() else {
            return found;
        };
        for block in blocks {
            match block["type"].as_str() {
                Some("tool_use") => {
                    // Words said before a tool call are the model thinking aloud, not the answer.
                    self.pending.clear();
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    if !self.seen.insert(id) {
                        continue;
                    }
                    let name = block["name"].as_str().unwrap_or_default();
                    let input = &block["input"];
                    found.push(Observation::Step(describe_tool_use(name, input)));
                    if name == "Read"
                        && let Some(path) = input["file_path"].as_str()
                    {
                        let start = input["offset"].as_u64().unwrap_or(1) as u32;
                        let end = input["limit"]
                            .as_u64()
                            .map_or(0, |limit| start + limit as u32 - 1);
                        found.push(Observation::File {
                            path: path.to_string(),
                            start,
                            end,
                        });
                    }
                }
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        self.pending.push_str(text);
                    }
                }
                _ => {}
            }
        }
        found
    }
}

/// A source from one line of the log the MCP server writes.
pub fn parse_source_line(line: &str) -> Option<Source> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(Source {
        kind: value["kind"].as_str()?.to_string(),
        reference: value["reference"].as_str()?.to_string(),
        label: value["label"].as_str().unwrap_or_default().to_string(),
        start_line: value["start_line"].as_u64().unwrap_or(0) as u32,
        end_line: value["end_line"].as_u64().unwrap_or(0) as u32,
    })
}

/// The path as the repository knows it: relative to `root`, with `/` between parts. A path outside the repository
/// is kept as it is.
pub(crate) fn relative_to(root: &Path, path: &str) -> String {
    let normalised = path.replace('\\', "/");
    let root = root.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    normalised
        .strip_prefix(root)
        .map(|rest| rest.trim_start_matches('/').to_string())
        .unwrap_or(normalised)
}

/// The conversation as one prompt. A single question goes as it is, after the instructions. A longer conversation is
/// labelled, so the model can tell who said what.
fn prompt_for(request: &ChatRequest) -> String {
    let mut prompt = String::new();
    if let Some(system) = &request.system {
        prompt.push_str(system);
        prompt.push_str("\n\n");
    }
    match request.messages.as_slice() {
        [only] => prompt.push_str(&only.content),
        messages => {
            prompt.push_str("The conversation so far, ending with the message to answer now:\n\n");
            for message in messages {
                let who = match message.role {
                    ChatRole::User => "User",
                    ChatRole::Assistant => "Assistant",
                };
                prompt.push_str(&format!("{who}: {}\n\n", message.content));
            }
        }
    }
    prompt
}

/// The `claude` program. On Windows it's usually installed as a script, which has to be started by its own name.
fn launcher() -> Command {
    if cfg!(windows) {
        if let Ok(output) = std::process::Command::new("where")
            .arg("claude")
            .stderr(Stdio::null())
            .output()
        {
            let listing = String::from_utf8_lossy(&output.stdout).into_owned();
            let lines: Vec<&str> = listing.lines().map(str::trim).collect();
            let pick = lines
                .iter()
                .find(|line| line.to_lowercase().ends_with(".exe"))
                .or_else(|| {
                    lines
                        .iter()
                        .find(|line| line.to_lowercase().ends_with(".cmd"))
                });
            if let Some(path) = pick {
                return Command::new(path);
            }
        }
        let mut command = Command::new("cmd");
        command.args(["/C", "claude"]);
        return command;
    }
    Command::new("claude")
}

/// A folder of our own in the system's temporary folder, for the tool configuration and the sources log.
struct Workspace(PathBuf);

impl Workspace {
    fn create() -> std::io::Result<Self> {
        let unique = format!(
            "looji-agent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl ChatProvider for ClaudeCodeAgent {
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>> {
        Box::pin(async move {
            let work = Workspace::create().map_err(|err| {
                AiError::CommandFailed(format!("Claude Code could not be set up: {err}"))
            })?;
            let sources_log = work.0.join("sources.jsonl");
            let mut server_args = self.server_args.clone();
            server_args.push(sources_log.to_string_lossy().into_owned());
            let config = json!({
                "mcpServers": { "looji": { "command": self.server.to_string_lossy(), "args": server_args } }
            });
            let config_path = work.0.join("mcp.json");
            std::fs::write(&config_path, config.to_string()).map_err(|err| {
                AiError::CommandFailed(format!("Claude Code could not be set up: {err}"))
            })?;

            let mut command = launcher();
            command
                .args([
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--no-session-persistence",
                ])
                .args(["--model", &request.model])
                .args(["--max-turns", &self.max_turns.to_string()])
                .args(["--strict-mcp-config", "--mcp-config"])
                .arg(&config_path)
                .args(["--tools", "Read,Grep,Glob"])
                .args(["--allowedTools", ALLOWED_TOOLS])
                .args(["--disallowedTools", DENIED_TOOLS])
                .current_dir(&self.root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(|err| {
                AiError::CommandFailed(format!("Claude Code did not start: {err}"))
            })?;
            if let Some(mut stdin) = child.stdin.take() {
                let written = stdin.write_all(prompt_for(&request).as_bytes()).await;
                drop(stdin);
                if let Err(err) = written {
                    let _ = child.start_kill();
                    return Err(AiError::CommandFailed(format!(
                        "Claude Code did not take the request: {err}"
                    )));
                }
            }
            let (producer, stream) = open_stream(&cancel);
            tokio::spawn(read_stream(
                child,
                producer,
                self.root.clone(),
                work,
                sources_log,
            ));
            Ok(Box::pin(stream) as EventStream)
        })
    }
}

/// Reads Claude Code's output until it ends, sending steps and sources as they happen and the answer at the end. Ends
/// early when the stream is stopped, which kills the command.
async fn read_stream(
    mut child: Child,
    out: Producer,
    root: PathBuf,
    work: Workspace,
    sources_log: PathBuf,
) {
    let _work = work;
    let Some(stdout) = child.stdout.take() else {
        out.send(Err(AiError::CommandFailed(
            "Claude Code gave no output stream".to_string(),
        )))
        .await;
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut reader = StreamReader::default();
    let mut logged = 0usize;
    loop {
        let line = tokio::select! {
            biased;
            _ = out.stopped() => {
                let _ = child.start_kill();
                return;
            }
            line = lines.next_line() => line,
        };
        let line = match line {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                out.send(Err(AiError::CommandFailed(format!(
                    "Claude Code stopped while answering: {err}"
                ))))
                .await;
                return;
            }
        };
        for observation in reader.read_line(&line) {
            let event = match observation {
                Observation::Step(step) => Ok(ChatEvent::Step(step)),
                Observation::File { path, start, end } => {
                    let reference = relative_to(&root, &path);
                    let label = reference
                        .rsplit('/')
                        .next()
                        .unwrap_or(&reference)
                        .to_string();
                    Ok(ChatEvent::Source(Source {
                        kind: "code".to_string(),
                        reference,
                        label,
                        start_line: start,
                        end_line: end,
                    }))
                }
                Observation::ToolAnswered => {
                    let sources = new_sources(&sources_log, &mut logged);
                    for source in sources {
                        if !out.send(Ok(ChatEvent::Source(source))).await {
                            let _ = child.start_kill();
                            return;
                        }
                    }
                    continue;
                }
                Observation::Answer(text) => {
                    if !out.send(Ok(ChatEvent::Token(text))).await {
                        let _ = child.start_kill();
                        return;
                    }
                    out.send(Ok(ChatEvent::Finished { usage: None })).await;
                    let _ = child.wait().await;
                    return;
                }
                Observation::Failure(reason) => {
                    out.send(Err(AiError::CommandFailed(reason))).await;
                    let _ = child.wait().await;
                    return;
                }
            };
            if !out.send(event).await {
                let _ = child.start_kill();
                return;
            }
        }
    }
    let _ = child.wait().await;
    out.send(Err(AiError::CommandFailed(
        "Claude Code finished without an answer. Check that it is signed in.".to_string(),
    )))
    .await;
}

/// The sources logged since the last look.
fn new_sources(log: &Path, already: &mut usize) -> Vec<Source> {
    let Ok(text) = std::fs::read_to_string(log) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let fresh: Vec<Source> = lines
        .iter()
        .skip(*already)
        .filter_map(|line| parse_source_line(line))
        .collect();
    *already = lines.len();
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = include_str!("../../tests/fixtures/claude_agent_stream.jsonl");

    fn read_all(text: &str) -> Vec<Observation> {
        let mut reader = StreamReader::default();
        text.lines()
            .flat_map(|line| reader.read_line(line))
            .collect()
    }

    #[test]
    fn a_real_run_gives_its_steps_then_its_answer() {
        let observations = read_all(STREAM);
        assert_eq!(
            observations,
            [
                Observation::Step("Looking up retrieve_hunks".to_string()),
                Observation::Step("Finding where retrieve_hunks is used".to_string()),
                Observation::ToolAnswered,
                Observation::ToolAnswered,
                Observation::Answer(
                    "`retrieve_hunks` is defined at `crates/persistence/src/retrieve.rs:19-85`. The one non-test caller is `crates/ui/src/workspace/chat.rs:654`; the rest are tests in `crates/ai/tests/task_review.rs`, `crates/persistence/tests/repo_index.rs` and `crates/persistence/tests/scale.rs`.".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_tool_call_seen_twice_is_one_step() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__looji__repo_map","input":{}}]}}"#;
        let mut reader = StreamReader::default();
        assert_eq!(reader.read_line(line).len(), 1);
        assert!(reader.read_line(line).is_empty());
    }

    #[test]
    fn reading_a_file_is_a_step_and_a_source() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"C:\\proj\\src\\a.rs","offset":10,"limit":20}}]}}"#;
        let observations = StreamReader::default().read_line(line);
        assert_eq!(
            observations[0],
            Observation::Step("Reading src/a.rs".to_string())
        );
        assert_eq!(
            observations[1],
            Observation::File {
                path: "C:\\proj\\src\\a.rs".to_string(),
                start: 10,
                end: 29
            }
        );
    }

    #[test]
    fn words_said_before_a_tool_call_are_not_the_answer() {
        let mut reader = StreamReader::default();
        reader.read_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me look that up."}]}}"#);
        reader.read_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__looji__repo_map","input":{}}]}}"#);
        reader.read_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"It is in src."}]}}"#);
        // A result with no text of its own falls back on the last message's words only.
        let answer = reader
            .read_line(r#"{"type":"result","subtype":"success","is_error":false,"result":""}"#);
        assert_eq!(answer, [Observation::Answer("It is in src.".to_string())]);
    }

    #[test]
    fn a_failed_run_reports_claude_codes_reason() {
        let failure = StreamReader::default().read_line(
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#,
        );
        assert_eq!(
            failure,
            [Observation::Failure(
                "Claude Code stopped (error_max_turns).".to_string()
            )]
        );
        let signed_out = StreamReader::default().read_line(
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Please log in."}"#,
        );
        assert_eq!(
            signed_out,
            [Observation::Failure("Please log in.".to_string())]
        );
    }

    #[test]
    fn lines_that_are_not_events_are_ignored() {
        let mut reader = StreamReader::default();
        assert!(
            reader
                .read_line("[claude-code:unrecognized_model] {}")
                .is_empty()
        );
        assert!(reader.read_line("").is_empty());
        assert!(
            reader
                .read_line(r#"{"type":"rate_limit_event"}"#)
                .is_empty()
        );
    }

    #[test]
    fn a_logged_source_line_is_read_back() {
        let source = parse_source_line(
            r#"{"kind":"code","reference":"src/a.rs","label":"fn a","start_line":3,"end_line":9}"#,
        )
        .unwrap();
        assert_eq!(
            source,
            Source {
                kind: "code".into(),
                reference: "src/a.rs".into(),
                label: "fn a".into(),
                start_line: 3,
                end_line: 9
            }
        );
        assert_eq!(parse_source_line("not json"), None);
        assert_eq!(parse_source_line(r#"{"label":"x"}"#), None);
    }

    #[test]
    fn new_sources_are_read_once() {
        let dir = std::env::temp_dir().join(format!("looji-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("sources.jsonl");
        let mut seen = 0;
        assert!(new_sources(&log, &mut seen).is_empty());
        std::fs::write(
            &log,
            "{\"kind\":\"doc\",\"reference\":\"a.md\",\"label\":\"A\"}\n",
        )
        .unwrap();
        assert_eq!(new_sources(&log, &mut seen).len(), 1);
        assert!(new_sources(&log, &mut seen).is_empty());
        std::fs::write(&log, "{\"kind\":\"doc\",\"reference\":\"a.md\",\"label\":\"A\"}\n{\"kind\":\"issue\",\"reference\":\"i1\",\"label\":\"I\"}\n").unwrap();
        let more = new_sources(&log, &mut seen);
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].kind, "issue");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paths_are_made_relative_to_the_repository() {
        let root = Path::new("C:\\Users\\me\\proj");
        assert_eq!(
            relative_to(root, "C:\\Users\\me\\proj\\src\\a.rs"),
            "src/a.rs"
        );
        assert_eq!(relative_to(root, "C:/Users/me/proj/src/a.rs"), "src/a.rs");
        assert_eq!(
            relative_to(root, "D:\\elsewhere\\b.rs"),
            "D:/elsewhere/b.rs"
        );
    }

    #[test]
    fn one_question_goes_as_it_is_and_a_conversation_is_labelled() {
        use crate::ChatMessage;
        let single = ChatRequest {
            model: "m".into(),
            system: Some("Rules.".into()),
            messages: vec![ChatMessage::user("Where is login?")],
            max_tokens: 100,
        };
        assert_eq!(prompt_for(&single), "Rules.\n\nWhere is login?");
        let long = ChatRequest {
            model: "m".into(),
            system: None,
            messages: vec![
                ChatMessage::user("Where is login?"),
                ChatMessage::assistant("In auth."),
                ChatMessage::user("And logout?"),
            ],
            max_tokens: 100,
        };
        let prompt = prompt_for(&long);
        assert!(
            prompt.contains("User: Where is login?\n\nAssistant: In auth.\n\nUser: And logout?")
        );
    }
}
