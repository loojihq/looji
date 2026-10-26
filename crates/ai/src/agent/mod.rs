//! Agents: a model that can look things up before it answers. It is given tools that read the project, calls them as
//! often as it needs, and answers when it has found enough. Two providers run it. The Anthropic API runs the loop
//! here, calling a [`ToolHost`] for each tool. Claude Code runs its own loop, and reaches the same tools through an
//! MCP server. Either way the caller gets the same events: steps as they happen, the sources that were read, and the
//! answer.

pub mod anthropic;
pub mod claude;

use serde_json::Value;

use crate::types::Source;

/// The most tool calls rounds one answer may use before it must answer with what it has.
pub const MAX_ROUNDS: usize = 8;

/// A tool the model can call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// A JSON Schema for the arguments.
    pub schema: Value,
}

/// What a tool call gave back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolReply {
    pub text: String,
    pub sources: Vec<Source>,
    /// The call failed, and `text` says why, so the model can try something else.
    pub is_error: bool,
}

/// The tools an agent can use. Calls are made off the async runtime, so a tool may read files and databases.
pub trait ToolHost: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;
    fn call(&self, name: &str, input: &Value) -> ToolReply;
}

/// A few words for what a tool call is doing, shown while the answer is being found. Works for looji's own tools
/// (bare, or with the `mcp__looji__` prefix Claude Code gives them) and for Claude Code's file tools.
pub fn describe_tool_use(name: &str, input: &Value) -> String {
    let name = name.strip_prefix("mcp__looji__").unwrap_or(name);
    let text = |key: &str| {
        input[key]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let shorten = |value: &str| -> String {
        let mut shown: String = value.chars().take(60).collect();
        if value.chars().count() > 60 {
            shown.push('\u{2026}');
        }
        shown
    };
    let file = |key: &str| {
        text(key).map(|path| {
            let parts: Vec<&str> = path
                .split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .collect();
            parts[parts.len().saturating_sub(2)..].join("/")
        })
    };
    match name {
        "search" => match text("query") {
            Some(query) => format!("Searching for \"{}\"", shorten(query)),
            None => "Searching the project".to_string(),
        },
        "read_file" => match text("path") {
            Some(path) => format!("Reading {}", shorten(path)),
            None => "Reading a file".to_string(),
        },
        "Read" => match file("file_path") {
            Some(path) => format!("Reading {path}"),
            None => "Reading a file".to_string(),
        },
        "find_symbol" => match text("name") {
            Some(name) => format!("Looking up {}", shorten(name)),
            None => "Looking up a definition".to_string(),
        },
        "find_references" => match text("name") {
            Some(name) => format!("Finding where {} is used", shorten(name)),
            None => "Finding uses".to_string(),
        },
        "list_issues" => "Looking through the issues".to_string(),
        "get_issue" => "Reading an issue".to_string(),
        "recent_changes" => "Checking recent changes".to_string(),
        "repo_map" => "Mapping the repository".to_string(),
        "Grep" => match text("pattern") {
            Some(pattern) => format!("Searching the files for \"{}\"", shorten(pattern)),
            None => "Searching the files".to_string(),
        },
        "Glob" => match text("pattern") {
            Some(pattern) => format!("Finding files like {}", shorten(pattern)),
            None => "Finding files".to_string(),
        },
        other => format!("Using {other}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::describe_tool_use;

    #[test]
    fn steps_read_as_short_sentences_for_looji_tools_with_or_without_the_prefix() {
        assert_eq!(
            describe_tool_use("search", &json!({"query": "verify password"})),
            "Searching for \"verify password\""
        );
        assert_eq!(
            describe_tool_use("mcp__looji__find_references", &json!({"name": "login"})),
            "Finding where login is used"
        );
        assert_eq!(
            describe_tool_use("mcp__looji__repo_map", &json!({})),
            "Mapping the repository"
        );
        assert_eq!(
            describe_tool_use("get_issue", &json!({"id": "1"})),
            "Reading an issue"
        );
    }

    #[test]
    fn claude_codes_file_tools_are_described_by_file_name_and_pattern() {
        assert_eq!(
            describe_tool_use(
                "Read",
                &json!({"file_path": "C:\\Users\\me\\proj\\src\\auth\\login.rs"})
            ),
            "Reading auth/login.rs"
        );
        assert_eq!(
            describe_tool_use("Grep", &json!({"pattern": "fn main"})),
            "Searching the files for \"fn main\""
        );
        assert_eq!(
            describe_tool_use("Glob", &json!({"pattern": "**/*.rs"})),
            "Finding files like **/*.rs"
        );
    }

    #[test]
    fn missing_arguments_and_unknown_tools_still_give_a_step() {
        assert_eq!(
            describe_tool_use("search", &json!({})),
            "Searching the project"
        );
        assert_eq!(describe_tool_use("Teleport", &json!({})), "Using Teleport");
    }

    #[test]
    fn a_long_query_is_shortened() {
        let query = "word ".repeat(40);
        let step = describe_tool_use("search", &json!({ "query": query }));
        assert!(step.chars().count() < 80, "{step}");
        assert!(step.contains('\u{2026}'));
    }
}
