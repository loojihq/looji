//! The blueprint: a set of documents and diagrams that define a project. An agent researches the repository when the
//! project has one, then answers with one JSON object. Diagrams in it are the constrained graphs of `looji-diagram`,
//! and each is checked there. A diagram that fails its checks is left out and counted, and the document stays.

use looji_diagram::parse_graph;
use serde_json::Value;

use crate::{ChatMessage, ChatRequest, review::json_object};

/// The kinds of document a blueprint holds, in the order they are shown.
pub const KINDS: [&str; 7] = [
    "overview",
    "architecture",
    "data_model",
    "interfaces",
    "decisions",
    "risks",
    "milestones",
];

/// The longest a document's body may be, in characters. Longer ones are cut, since a runaway reply helps nobody.
pub const MAX_BODY_CHARS: usize = 12_000;

/// One document as the model wrote it, with its diagrams checked.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDoc {
    pub kind: String,
    pub title: String,
    pub body: String,
    /// The diagrams that passed, as JSON.
    pub diagrams: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBlueprint {
    pub docs: Vec<ParsedDoc>,
    /// How many diagrams were left out for failing their checks.
    pub diagrams_dropped: usize,
}

const INSTRUCTIONS: &str = "You write the blueprint for a software project: the documents that define what it is and how it \
is built. If the project has a repository, research it first with your tools: read the repo map, search the code, \
and read the issues, so the blueprint describes what exists and where the project is going, not a guess. \
Say plainly what you could not find.\n\n\
Write these documents, in this order, each as Markdown of at most 400 words with short headings and lists: \
\"overview\" (problem, goals, non-goals, users), \"architecture\" (components, what each owns, how they talk, the \
data flow), \"data_model\" (entities, fields, relations), \"interfaces\" (APIs, events, file formats), \"decisions\" \
(each as: context, choice, alternatives, consequences), \"risks\" (risks and open questions), \"milestones\" \
(ordered, each with what it delivers). Skip a document only if it truly does not apply.\n\n\
Diagrams are not drawn. Write each as a graph in a document's \"diagrams\" list: {\"type\": \"component\" | \"flow\" | \
\"entity\" | \"deployment\", \"title\": \"...\", \"groups\": [{\"id\": \"...\", \"label\": \"...\"}], \"nodes\": \
[{\"id\": \"...\", \"label\": \"short\", \"kind\": \"service | database | ...\", \"group\": \"a group id\", \"link\": \
{\"type\": \"file\" | \"section\" | \"issue\", \"target\": \"path, section title, or issue title\"}}], \"edges\": \
[{\"from\": \"node id\", \"to\": \"node id\", \"label\": \"short\", \"style\": \"solid\" | \"dashed\"}]}. At most 25 \
nodes each, labels under 40 characters. Give the architecture a component diagram and, where it helps, a flow \
diagram of the main data path. Give the data model an entity diagram. Link a node to a file only if you have seen \
that file.\n\n\
Reply with only a JSON object: {\"documents\": [{\"kind\": \"...\", \"title\": \"...\", \"body\": \"Markdown\", \
\"diagrams\": []}]}. No text outside it.";

/// The request that asks an agent for a blueprint. `idea` is what the user wants built or described.
pub fn build_blueprint_request(
    model: &str,
    project: &str,
    description: &str,
    idea: &str,
    has_repository: bool,
) -> ChatRequest {
    let mut message = format!("Project: {project}\n");
    if !description.trim().is_empty() {
        message.push_str(&format!("\nDescription:\n{}\n", description.trim()));
    }
    if !idea.trim().is_empty() {
        message.push_str(&format!(
            "\nWhat to write the blueprint for:\n{}\n",
            idea.trim()
        ));
    }
    message.push_str(if has_repository {
        "\nThe project has a repository. Research it before you write."
    } else {
        "\nThe project has no repository yet, so design it from the description."
    });
    ChatRequest {
        model: model.to_string(),
        system: Some(INSTRUCTIONS.to_string()),
        messages: vec![ChatMessage::user(message)],
        max_tokens: 12_000,
    }
}

/// The request that asks an agent to change one document. The reply has the same shape as a whole blueprint, with one
/// document in it, so it is read the same way.
pub fn build_revision_request(
    model: &str,
    project: &str,
    document: (&str, &str, &str, &str),
    instruction: &str,
    has_repository: bool,
) -> ChatRequest {
    let (kind, title, body, diagrams) = document;
    let system = format!(
        "{INSTRUCTIONS}

You are not writing a whole blueprint now. You are changing one document of it,          as the user asks. Keep everything the user did not ask you to change, in the same words. Reply with the          JSON object as above, holding only that one document, complete, with its diagrams."
    );
    let mut message = format!(
        "Project: {project}

The document, kind \"{kind}\", titled \"{title}\":

{body}

Its diagrams, as JSON:
{diagrams}

Change to make:
{}
",
        instruction.trim()
    );
    message.push_str(if has_repository {
        "
The project has a repository. Look at it if the change depends on what the code does."
    } else {
        "
The project has no repository."
    });
    ChatRequest {
        model: model.to_string(),
        system: Some(system),
        messages: vec![ChatMessage::user(message)],
        max_tokens: 8_000,
    }
}

fn clean_body(text: &str) -> String {
    let body = text.trim();
    if body.chars().count() <= MAX_BODY_CHARS {
        return body.to_string();
    }
    body.chars().take(MAX_BODY_CHARS).collect()
}

/// Reads a reply. A reply with no documents is an error. A document without a title or body is skipped.
pub fn parse_blueprint(reply: &str) -> Result<ParsedBlueprint, String> {
    let value = json_object(reply).ok_or("The blueprint was not readable.")?;
    let documents = value
        .get("documents")
        .and_then(Value::as_array)
        .ok_or("The blueprint had no documents.")?;
    let mut docs = Vec::new();
    let mut dropped = 0;
    for item in documents {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let body = clean_body(item.get("body").and_then(Value::as_str).unwrap_or(""));
        if title.is_empty() || body.is_empty() {
            continue;
        }
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .map(|kind| kind.trim().to_ascii_lowercase().replace([' ', '-'], "_"))
            .filter(|kind| KINDS.contains(&kind.as_str()))
            .unwrap_or_else(|| "other".to_string());
        let mut diagrams = Vec::new();
        for diagram in item
            .get("diagrams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if parse_graph(diagram).is_ok() {
                diagrams.push(diagram.clone());
            } else {
                dropped += 1;
            }
        }
        docs.push(ParsedDoc {
            kind,
            title: title.to_string(),
            body,
            diagrams,
        });
    }
    if docs.is_empty() {
        return Err("The blueprint had no documents with a title and a body.".to_string());
    }
    Ok(ParsedBlueprint {
        docs,
        diagrams_dropped: dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(extra_diagram: &str) -> String {
        format!(
            r#"```json
{{"documents": [
  {{"kind": "overview", "title": "Overview", "body": "Goals.", "diagrams": []}},
  {{"kind": "Architecture", "title": "Architecture", "body": "Parts.", "diagrams": [
     {{"type": "component", "nodes": [{{"id": "a", "label": "A"}}, {{"id": "b", "label": "B"}}], "edges": [{{"from": "a", "to": "b"}}]}}
     {extra_diagram}
  ]}},
  {{"kind": "mystery", "title": "Notes", "body": "Other things."}},
  {{"kind": "risks", "title": "", "body": "no title"}}
]}}
```"#
        )
    }

    #[test]
    fn documents_are_read_with_their_valid_diagrams_and_blank_ones_skipped() {
        let blueprint = parse_blueprint(&reply("")).unwrap();
        assert_eq!(blueprint.docs.len(), 3);
        assert_eq!(blueprint.docs[1].kind, "architecture");
        assert_eq!(blueprint.docs[1].diagrams.len(), 1);
        assert_eq!(blueprint.docs[2].kind, "other");
        assert_eq!(blueprint.diagrams_dropped, 0);
    }

    #[test]
    fn a_diagram_that_fails_its_checks_is_dropped_and_counted_and_the_document_stays() {
        let bad = r#", {"type": "flow", "nodes": [{"id": "a", "label": "A"}], "edges": [{"from": "a", "to": "ghost"}]}"#;
        let blueprint = parse_blueprint(&reply(bad)).unwrap();
        assert_eq!(blueprint.docs[1].diagrams.len(), 1);
        assert_eq!(blueprint.diagrams_dropped, 1);
    }

    #[test]
    fn a_reply_with_no_usable_documents_is_an_error() {
        assert!(parse_blueprint("Sure, here you go.").is_err());
        assert!(parse_blueprint("{\"documents\": []}").is_err());
        assert!(parse_blueprint("{\"documents\": [{\"title\": \"x\"}]}").is_err());
    }

    #[test]
    fn a_revision_request_carries_the_document_and_the_change_asked_for() {
        let request = build_revision_request(
            "m",
            "Shop",
            ("risks", "Risks", "Old risks.", "[]"),
            "Add a risk about payments",
            false,
        );
        let text = &request.messages[0].content;
        assert!(text.contains("Old risks.") && text.contains("Add a risk about payments"));
        assert!(
            request
                .system
                .as_deref()
                .unwrap()
                .contains("only that one document")
        );
    }

    #[test]
    fn the_request_says_whether_there_is_a_repository_to_research() {
        let with = build_blueprint_request("m", "Shop", "A shop.", "Add payments", true);
        assert!(with.messages[0].content.contains("Research it"));
        let without = build_blueprint_request("m", "Shop", "", "", false);
        assert!(without.messages[0].content.contains("no repository"));
    }
}
