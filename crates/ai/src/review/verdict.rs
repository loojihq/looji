//! A review of a task that was moved into review: does the work do what the task said? The reviewer is an agent, so it
//! reads the code and the changes itself, then answers in one JSON object with a verdict, a result for each of the
//! task's "Done when" points with the evidence for it, and anything else worth fixing. The reply is read strictly.
//! A review that can't be read is reported as unreadable, never guessed at, and a pass is only given when every point
//! is met.

use serde_json::{Value, json};

use super::json_object;
use crate::{ChatMessage, ChatRequest};

/// How much of the working tree's changes the reviewer is shown, at most.
pub const MAX_CHANGES_CHARS: usize = 12_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Met,
    Partly,
    NotMet,
    /// The reviewer couldn't tell from the code.
    Unclear,
}

impl Rating {
    fn stored(self) -> &'static str {
        match self {
            Rating::Met => "met",
            Rating::Partly => "partly",
            Rating::NotMet => "not_met",
            Rating::Unclear => "unclear",
        }
    }

    fn from_stored(text: &str) -> Option<Self> {
        match text
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "met" => Some(Rating::Met),
            "partly" | "partial" => Some(Rating::Partly),
            "not_met" => Some(Rating::NotMet),
            "unclear" => Some(Rating::Unclear),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every point is met and nothing serious was found.
    Done,
    /// Something is missing or wrong.
    NeedsWork,
    /// The code didn't settle it.
    Unclear,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Done => "Looks done",
            Outcome::NeedsWork => "Needs work",
            Outcome::Unclear => "Not sure",
        }
    }

    fn stored(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::NeedsWork => "needs_work",
            Outcome::Unclear => "unclear",
        }
    }

    fn from_stored(text: &str) -> Option<Self> {
        match text
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "done" | "pass" => Some(Outcome::Done),
            "needs_work" | "fail" | "concerns" => Some(Outcome::NeedsWork),
            "unclear" => Some(Outcome::Unclear),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    pub text: String,
    pub rating: Rating,
    /// Where it shows, or what is missing.
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::High => "High",
            Severity::Medium => "Medium",
            Severity::Low => "Low",
        }
    }

    fn stored(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }

    fn from_stored(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "high" => Severity::High,
            "low" => Severity::Low,
            _ => Severity::Medium,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub text: String,
    /// A file, with lines when the reviewer gave them.
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReview {
    pub outcome: Outcome,
    pub summary: String,
    pub criteria: Vec<Criterion>,
    pub findings: Vec<Finding>,
}

/// The points under a "Done when" heading in a task's description, in order. The heading may be plain text or a
/// markdown heading, with or without a colon. Points are list items, with or without a checkbox, and they end at a
/// blank line or another heading.
pub fn done_when(description: &str) -> Vec<String> {
    let mut points = Vec::new();
    let mut inside = false;
    for line in description.lines() {
        let trimmed = line.trim();
        if !inside {
            let heading = trimmed.trim_start_matches('#').trim().trim_end_matches(':');
            if heading.eq_ignore_ascii_case("done when") {
                inside = true;
            }
            continue;
        }
        if trimmed.is_empty() {
            if !points.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            break;
        }
        match list_item(trimmed) {
            Some(point) => points.push(point),
            None => break,
        }
    }
    points
}

fn list_item(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            (digits > 0)
                .then(|| {
                    line[digits..]
                        .strip_prefix(". ")
                        .or_else(|| line[digits..].strip_prefix(") "))
                })
                .flatten()
        })?;
    let rest = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|mark| rest.strip_prefix(mark))
        .unwrap_or(rest);
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

/// What the reviewer is told.
const INSTRUCTIONS: &str = "You review one task that a developer has moved into review. Decide whether the work does \
what the task asked. Use your tools to look: read the code the task is about, and check the changes you are shown \
against it. Judge each \"Done when\" point separately, and only call it met when you have seen the code that does it. \
If there are no points, judge the task's title and description in the same way. Then note anything else worth fixing \
in the changes: bugs, missing handling, missing tests.\n\nFinish with only a JSON object of this shape, with no other \
text: {\"outcome\": \"done\" | \"needs_work\" | \"unclear\", \"summary\": \"one or two sentences\", \"criteria\": \
[{\"text\": \"the point\", \"rating\": \"met\" | \"partly\" | \"not_met\" | \"unclear\", \"evidence\": \"path:line \
where it shows, or what is missing\"}], \"findings\": [{\"severity\": \"high\" | \"medium\" | \"low\", \"text\": \
\"what is wrong\", \"location\": \"path:line\"}]}. Say \"done\" only when every point is met and nothing high was found.";

/// The request that asks an agent for a review. `changes` is the working tree's changes, already limited.
pub fn build_review_request(
    model: &str,
    title: &str,
    description: &str,
    criteria: &[String],
    changes: &str,
) -> ChatRequest {
    let mut message = format!("Task: {title}\n");
    if !description.trim().is_empty() {
        message.push_str(&format!("\nDescription:\n{}\n", description.trim()));
    }
    if criteria.is_empty() {
        message.push_str("\nThe task lists no \"Done when\" points.\n");
    } else {
        message.push_str("\nDone when:\n");
        for (index, point) in criteria.iter().enumerate() {
            message.push_str(&format!("{}. {point}\n", index + 1));
        }
    }
    if changes.trim().is_empty() {
        message.push_str(
            "\nThere are no uncommitted changes. Look at recent commits and the code itself.\n",
        );
    } else {
        message.push_str("\nUncommitted changes in the repository:\n\n");
        message.push_str(changes.trim());
        message.push('\n');
    }
    ChatRequest {
        model: model.to_string(),
        system: Some(INSTRUCTIONS.to_string()),
        messages: vec![ChatMessage::user(message)],
        max_tokens: 3_000,
    }
}

/// Reads a reply. `criteria` are the points the review was asked about. The result always has one entry for each,
/// in order, so a point the reviewer skipped shows as unclear instead of being lost.
pub fn parse_review(reply: &str, criteria: &[String]) -> Result<TaskReview, String> {
    let value = json_object(reply).ok_or("The review was not readable.")?;
    let said = value
        .get("outcome")
        .and_then(Value::as_str)
        .and_then(Outcome::from_stored)
        .ok_or("The review gave no outcome.")?;
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    let given: Vec<(String, Rating, String)> = value
        .get("criteria")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let rating = Rating::from_stored(item.get("rating")?.as_str()?)?;
            Some((
                item.get("text")?.as_str()?.trim().to_string(),
                rating,
                item.get("evidence")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            ))
        })
        .collect();
    let criteria: Vec<Criterion> = if criteria.is_empty() {
        given
            .into_iter()
            .map(|(text, rating, evidence)| Criterion {
                text,
                rating,
                evidence,
            })
            .collect()
    } else {
        criteria
            .iter()
            .enumerate()
            .map(|(index, text)| {
                // A point is matched by its words first, then by its place in the list.
                let found = given
                    .iter()
                    .find(|(said, ..)| said.eq_ignore_ascii_case(text))
                    .or_else(|| given.get(index));
                match found {
                    Some((_, rating, evidence)) => Criterion {
                        text: text.clone(),
                        rating: *rating,
                        evidence: evidence.clone(),
                    },
                    None => Criterion {
                        text: text.clone(),
                        rating: Rating::Unclear,
                        evidence: "The review did not say.".to_string(),
                    },
                }
            })
            .collect()
    };
    let findings: Vec<Finding> = value
        .get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let text = item.get("text")?.as_str()?.trim();
            (!text.is_empty()).then(|| Finding {
                severity: Severity::from_stored(
                    item.get("severity").and_then(Value::as_str).unwrap_or(""),
                ),
                text: text.to_string(),
                location: item
                    .get("location")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|location| !location.is_empty())
                    .map(str::to_string),
            })
        })
        .collect();

    // The outcome can only be as good as the points allow.
    let serious = findings
        .iter()
        .any(|finding| finding.severity == Severity::High);
    let outcome = if criteria
        .iter()
        .any(|criterion| criterion.rating == Rating::NotMet)
        || serious
    {
        Outcome::NeedsWork
    } else if said == Outcome::Done
        && criteria
            .iter()
            .all(|criterion| criterion.rating == Rating::Met)
    {
        Outcome::Done
    } else if said == Outcome::NeedsWork
        || criteria
            .iter()
            .any(|criterion| criterion.rating == Rating::Partly)
    {
        Outcome::NeedsWork
    } else {
        Outcome::Unclear
    };
    Ok(TaskReview {
        outcome,
        summary,
        criteria,
        findings,
    })
}

impl TaskReview {
    /// The review as saved.
    pub fn to_json(&self) -> String {
        json!({
            "outcome": self.outcome.stored(),
            "summary": self.summary,
            "criteria": self.criteria.iter().map(|criterion| json!({
                "text": criterion.text,
                "rating": criterion.rating.stored(),
                "evidence": criterion.evidence,
            })).collect::<Vec<_>>(),
            "findings": self.findings.iter().map(|finding| json!({
                "severity": finding.severity.stored(),
                "text": finding.text,
                "location": finding.location,
            })).collect::<Vec<_>>(),
        })
        .to_string()
    }

    /// A saved review, or `None` when what was saved can't be read.
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let criteria = value
            .get("criteria")?
            .as_array()?
            .iter()
            .filter_map(|item| {
                Some(Criterion {
                    text: item.get("text")?.as_str()?.to_string(),
                    rating: Rating::from_stored(item.get("rating")?.as_str()?)?,
                    evidence: item.get("evidence")?.as_str()?.to_string(),
                })
            })
            .collect();
        let findings = value
            .get("findings")?
            .as_array()?
            .iter()
            .filter_map(|item| {
                Some(Finding {
                    severity: Severity::from_stored(item.get("severity")?.as_str()?),
                    text: item.get("text")?.as_str()?.to_string(),
                    location: item
                        .get("location")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect();
        Some(Self {
            outcome: Outcome::from_stored(value.get("outcome")?.as_str()?)?,
            summary: value.get("summary")?.as_str()?.to_string(),
            criteria,
            findings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn done_when_points_are_read_from_a_list_under_the_heading() {
        let description = "Make login safer.\n\nDone when:\n- [ ] Empty passwords are refused\n- [x] A test covers it\n* Errors are shown\n1. Numbered works too\n\nNotes below.";
        assert_eq!(
            done_when(description),
            points(&[
                "Empty passwords are refused",
                "A test covers it",
                "Errors are shown",
                "Numbered works too"
            ])
        );
    }

    #[test]
    fn the_heading_may_be_markdown_and_any_case() {
        assert_eq!(
            done_when("## DONE WHEN\n- one\n- two"),
            points(&["one", "two"])
        );
        assert_eq!(done_when("Done when\n- one"), points(&["one"]));
    }

    #[test]
    fn no_heading_or_no_list_gives_no_points() {
        assert!(done_when("Just a description.\n- not under a heading").is_empty());
        assert!(done_when("Done when:\nno list here").is_empty());
        assert!(done_when("").is_empty());
    }

    #[test]
    fn points_stop_at_the_next_heading() {
        assert_eq!(done_when("Done when:\n- a\n## Other\n- b"), points(&["a"]));
    }

    fn reply(outcome: &str, rating: &str) -> String {
        format!(
            "```json\n{{\"outcome\": \"{outcome}\", \"summary\": \"Mostly there.\", \"criteria\": [{{\"text\": \"Empty passwords are refused\", \"rating\": \"{rating}\", \"evidence\": \"src/auth.rs:12\"}}], \"findings\": [{{\"severity\": \"low\", \"text\": \"No test.\", \"location\": \"src/auth.rs\"}}]}}\n```"
        )
    }

    #[test]
    fn a_review_where_every_point_is_met_is_done() {
        let review = parse_review(
            &reply("done", "met"),
            &points(&["Empty passwords are refused"]),
        )
        .unwrap();
        assert_eq!(review.outcome, Outcome::Done);
        assert_eq!(review.criteria[0].evidence, "src/auth.rs:12");
        assert_eq!(review.findings[0].severity, Severity::Low);
        assert_eq!(review.findings[0].location.as_deref(), Some("src/auth.rs"));
    }

    #[test]
    fn done_is_not_given_when_a_point_is_not_met_whatever_the_reviewer_said() {
        let review = parse_review(
            &reply("done", "not_met"),
            &points(&["Empty passwords are refused"]),
        )
        .unwrap();
        assert_eq!(review.outcome, Outcome::NeedsWork);
    }

    #[test]
    fn a_high_finding_stops_a_pass() {
        let text = "{\"outcome\": \"done\", \"summary\": \"\", \"criteria\": [], \"findings\": [{\"severity\": \"high\", \"text\": \"Deletes data.\"}]}";
        assert_eq!(parse_review(text, &[]).unwrap().outcome, Outcome::NeedsWork);
    }

    #[test]
    fn a_point_the_reviewer_skipped_shows_as_unclear() {
        let review = parse_review(
            &reply("done", "met"),
            &points(&["Empty passwords are refused", "Errors are shown"]),
        )
        .unwrap();
        assert_eq!(review.criteria.len(), 2);
        assert_eq!(review.criteria[1].rating, Rating::Unclear);
        assert_eq!(review.outcome, Outcome::Unclear);
    }

    #[test]
    fn a_reply_that_is_not_a_review_is_an_error() {
        assert!(parse_review("Looks fine to me.", &[]).is_err());
        assert!(parse_review("{\"summary\": \"x\"}", &[]).is_err());
        assert!(parse_review("{\"outcome\": \"maybe\"}", &[]).is_err());
    }

    #[test]
    fn a_saved_review_reads_back_the_same() {
        let review = parse_review(
            &reply("needs_work", "partly"),
            &points(&["Empty passwords are refused"]),
        )
        .unwrap();
        assert_eq!(TaskReview::from_json(&review.to_json()), Some(review));
        assert_eq!(TaskReview::from_json("not json"), None);
    }

    #[test]
    fn the_request_lists_the_points_and_the_changes() {
        let request = build_review_request(
            "m",
            "Fix login",
            "Details.",
            &points(&["a", "b"]),
            "diff --git a/x b/x",
        );
        let text = &request.messages[0].content;
        assert!(text.contains("1. a") && text.contains("2. b"));
        assert!(text.contains("diff --git"));
        let none = build_review_request("m", "Fix login", "", &[], "");
        assert!(none.messages[0].content.contains("no \"Done when\" points"));
        assert!(none.messages[0].content.contains("no uncommitted changes"));
    }
}
