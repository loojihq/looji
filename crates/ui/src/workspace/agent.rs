//! Connects the AI agents to the project's knowledge. The Anthropic API runs the tools here, through `Host`. Claude Code
//! starts looji again as an MCP server and uses the same tools from there.

use std::{path::Path, sync::Arc};

use looji_ai::{
    AiError, ChatProvider, Provider,
    agent::{
        ToolHost, ToolReply, ToolSpec,
        anthropic::AnthropicAgent,
        claude::{ClaudeCodeAgent, DEEP_TURNS, QUICK_TURNS},
    },
    claude_code, key_store,
};
use looji_domain::ProjectId;
use looji_knowledge::tools::{KnowledgeTools, definitions};
use serde_json::Value;

/// The knowledge tools, as an agent calls them.
struct Host(KnowledgeTools);

impl ToolHost for Host {
    fn specs(&self) -> Vec<ToolSpec> {
        definitions()
            .into_iter()
            .map(|tool| ToolSpec {
                name: tool.name.to_string(),
                description: tool.description.to_string(),
                schema: tool.schema,
            })
            .collect()
    }

    fn call(&self, name: &str, input: &Value) -> ToolReply {
        let output = self.0.call(name, input);
        ToolReply {
            text: output.text,
            sources: output
                .sources
                .into_iter()
                .map(|source| looji_ai::Source {
                    kind: source.kind.stored().to_string(),
                    reference: source.reference,
                    label: source.label,
                    start_line: source.start_line,
                    end_line: source.end_line,
                })
                .collect(),
            is_error: output.is_error,
        }
    }
}

/// The agent for `provider`, set up to explore the repository at `root` through the project's knowledge base.
pub(super) fn agent_for(
    provider: Provider,
    knowledge_path: &Path,
    project: ProjectId,
    root: &Path,
    deep: bool,
) -> Result<Arc<dyn ChatProvider>, AiError> {
    let project = project.as_uuid().to_string();
    match provider {
        Provider::ClaudeCode => {
            if !claude_code::installed() {
                return Err(AiError::CommandFailed(
                    "Claude Code isn't installed on this computer. Install it and sign in, then try again."
                        .to_string(),
                ));
            }
            let server = std::env::current_exe().map_err(|err| {
                AiError::CommandFailed(format!(
                    "Looji could not find itself to start the tools: {err}"
                ))
            })?;
            Ok(Arc::new(ClaudeCodeAgent {
                root: root.to_path_buf(),
                server,
                server_args: vec![
                    "--knowledge-mcp".to_string(),
                    knowledge_path.to_string_lossy().into_owned(),
                    project,
                    root.to_string_lossy().into_owned(),
                ],
                max_turns: if deep { DEEP_TURNS } else { QUICK_TURNS },
            }))
        }
        Provider::Anthropic => {
            let key = key_store::load()?.ok_or(AiError::NoApiKey)?;
            let tools = KnowledgeTools::open(knowledge_path, &project, root)
                .map(|tools| {
                    tools.with_embedder(Arc::new(looji_embed::LocalEmbedder::new(
                        knowledge_path.with_file_name("models"),
                    )))
                })
                .map_err(|err| {
                    AiError::CommandFailed(format!(
                        "The project's knowledge could not be opened: {err}"
                    ))
                })?;
            Ok(Arc::new(AnthropicAgent::new(key, Arc::new(Host(tools)))))
        }
    }
}
