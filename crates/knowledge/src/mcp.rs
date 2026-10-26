//! The tools as an MCP server, so Claude Code can use them. MCP over standard input and output is one JSON-RPC message
//! per line. The server answers `initialize`, `tools/list`, `tools/call`, and `ping`, and ignores notifications.
//!
//! Claude Code shows the model only the text of a tool's answer, so the sources a call drew on go to a log file, one
//! JSON object per line. The app reads that file to list "Sources used" under the answer.

use std::{
    fs::OpenOptions,
    io::{BufRead, Write},
    path::Path,
};

use serde_json::{Value, json};

use crate::tools::{KnowledgeTools, ToolSource, definitions};

/// The protocol revision this server speaks when the client doesn't ask for one.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// One source as a line of the log.
pub fn source_line(source: &ToolSource) -> String {
    json!({
        "kind": source.kind.stored(),
        "reference": source.reference,
        "label": source.label,
        "start_line": source.start_line,
        "end_line": source.end_line,
    })
    .to_string()
}

fn append_sources(log: &Path, sources: &[ToolSource]) {
    if sources.is_empty() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log) {
        for source in sources {
            let _ = writeln!(file, "{}", source_line(source));
        }
    }
}

/// The reply to one message, or `None` when it needs none. A message that isn't JSON gets a parse error, since the
/// client is waiting on something.
pub fn handle_message(
    tools: &KnowledgeTools,
    line: &str,
    sources_log: Option<&Path>,
) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(_) => return Some(failure(Value::Null, -32700, "The message was not JSON.")),
    };
    // A message with no id is a notification, and gets no answer.
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or_default();
    let reply = match method {
        "initialize" => json!({
            "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "looji", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": definitions()
                .into_iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.schema,
                }))
                .collect::<Vec<_>>(),
        }),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = message["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let output = tools.call(name, &arguments);
            if let Some(log) = sources_log {
                append_sources(log, &output.sources);
            }
            json!({
                "content": [{ "type": "text", "text": output.text }],
                "isError": output.is_error,
            })
        }
        _ => return Some(failure(id, -32601, "That method is not supported.")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": reply }))
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Serves messages from `input` until it ends, writing each reply to `output` as one line.
pub fn serve(
    tools: &KnowledgeTools,
    input: impl BufRead,
    mut output: impl Write,
    sources_log: Option<&Path>,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_message(tools, &line, sources_log) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunk::{Chunk, SourceKind},
        store::{KnowledgeStore, SourceMeta},
    };
    use tempfile::TempDir;

    fn tools() -> KnowledgeTools {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "src/auth.rs",
                &SourceMeta::new("1", 1, None),
                &[Chunk {
                    kind: SourceKind::Code,
                    source: "src/auth.rs".into(),
                    path: Some("src/auth.rs".into()),
                    label: Some("fn verify_password".into()),
                    start_line: 1,
                    end_line: 3,
                    text: "fn verify_password() {}".into(),
                }],
            )
            .unwrap();
        KnowledgeTools::with_store(store, "p", Path::new("."))
    }

    fn ask(tools: &KnowledgeTools, message: Value, log: Option<&Path>) -> Option<Value> {
        handle_message(tools, &message.to_string(), log)
    }

    #[test]
    fn initialize_answers_with_the_tools_capability_and_echoes_the_protocol() {
        let reply = ask(&tools(), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}), None).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert_eq!(reply["result"]["serverInfo"]["name"], "looji");
    }

    #[test]
    fn a_notification_gets_no_answer() {
        assert_eq!(
            ask(
                &tools(),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                None
            ),
            None
        );
    }

    #[test]
    fn the_tool_list_names_every_tool_with_its_schema() {
        let reply = ask(
            &tools(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            None,
        )
        .unwrap();
        let list = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(list.len(), 8);
        assert!(
            list.iter()
                .all(|tool| tool["inputSchema"]["type"] == "object"
                    && tool["description"].is_string())
        );
    }

    #[test]
    fn a_tool_call_returns_text_and_logs_its_sources() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("sources.jsonl");
        let reply = ask(
            &tools(),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "search", "arguments": {"query": "verify password"}}}),
            Some(&log),
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], false);
        assert!(
            reply["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("src/auth.rs:1-3")
        );
        let logged = std::fs::read_to_string(&log).unwrap();
        let line: Value = serde_json::from_str(logged.lines().next().unwrap()).unwrap();
        assert_eq!(line["kind"], "code");
        assert_eq!(line["reference"], "src/auth.rs");
        assert_eq!(line["start_line"], 1);
    }

    #[test]
    fn a_failed_tool_call_is_an_error_result_not_a_protocol_error() {
        let reply = ask(&tools(), json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "nope", "arguments": {}}}), None).unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply.get("error").is_none());
    }

    #[test]
    fn unknown_methods_and_bad_json_are_protocol_errors() {
        let unknown = ask(
            &tools(),
            json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}),
            None,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        let bad = handle_message(&tools(), "{not json", None).unwrap();
        assert_eq!(bad["error"]["code"], -32700);
    }

    #[test]
    fn the_server_reads_lines_and_writes_one_reply_line_each() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
        let mut output = Vec::new();
        serve(&tools(), input.as_bytes(), &mut output, None).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(
            text.lines()
                .all(|line| serde_json::from_str::<Value>(line).is_ok())
        );
    }
}
