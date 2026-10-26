//! Git history as a source of knowledge: commits, which files they touched, and which issues they are about.
//!
//! This runs the `git` program, so a machine without it simply has no history in the knowledge base. Nothing here
//! writes to the repository.

use std::{path::Path, process::Command};

use crate::{
    Error, Result,
    chunk::{CommitDoc, IssueDoc},
    tokens::query_terms,
};

/// Separates the commits in `git log` output, and the fields within one. Neither appears in a commit message.
const RECORD: char = '\u{1e}';
const FIELD: char = '\u{1f}';

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashes up for a background command.
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|err| Error::Git(format!("could not run git: {err}")))?;
    if !output.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The commit the repository is at. `None` for a folder that isn't a repository, or one with no commits.
pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|sha| !sha.is_empty())
}

/// The branch the repository is on. `None` when detached or when it isn't a repository.
pub fn branch(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|name| !name.is_empty() && name != "HEAD")
}

/// The uncommitted changes, as `git diff` shows them, for a reviewer to read. Files that look like secrets are left out,
/// and so is anything past `max_chars`, which the text says. Empty when nothing changed or git isn't there.
pub fn working_changes(root: &Path, max_chars: usize) -> String {
    let Ok(names) = git(root, &["diff", "--name-only", "HEAD"]) else {
        return String::new();
    };
    let files: Vec<&str> = names
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !crate::safety::is_secret_path(name))
        .collect();
    if files.is_empty() {
        return String::new();
    }
    let mut args = vec!["diff", "--no-color", "-U2", "HEAD", "--"];
    args.extend(files);
    let Ok(mut text) = git(root, &args) else {
        return String::new();
    };
    if text.len() > max_chars {
        let mut end = max_chars;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[the rest of the changes are not shown]");
    }
    text
}

/// Up to `limit` of the newest commits, newest first. With `since`, only the commits after that one. An unknown
/// `since`, such as one that a rebase removed, gives the newest `limit` instead.
pub fn recent_commits(root: &Path, limit: usize, since: Option<&str>) -> Result<Vec<CommitDoc>> {
    let format = format!("--format={RECORD}%H{FIELD}%an{FIELD}%at{FIELD}%B{FIELD}");
    let count = format!("-n{limit}");
    let range = since.map(|sha| format!("{sha}..HEAD"));
    let mut args = vec!["log", &count, "--name-only", &format];
    if let Some(range) = &range {
        args.push(range);
    }
    match git(root, &args) {
        Ok(text) => Ok(parse_log(&text)),
        Err(_) if since.is_some() => recent_commits(root, limit, None),
        Err(err) => Err(err),
    }
}

/// Reads `git log` output in the format `recent_commits` asks for.
pub fn parse_log(text: &str) -> Vec<CommitDoc> {
    text.split(RECORD)
        .filter_map(|record| {
            let mut fields = record.split(FIELD);
            let sha = fields.next()?.trim().to_string();
            let author = fields.next()?.trim().to_string();
            let date = fields.next()?.trim().parse().unwrap_or(0);
            let message = fields.next()?.trim().to_string();
            let files = fields
                .next()
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            (!sha.is_empty()).then_some(CommitDoc {
                sha,
                author,
                date,
                message,
                files,
            })
        })
        .collect()
}

/// The ids of the issues a commit is about. A commit is about an issue when its message mentions the issue's id (or
/// its first eight characters, as `#1a2b3c4d`), or when most of the words of the issue's title are in the message.
/// Two words at least are needed, so a one-word title is only matched by a mention.
pub fn issues_for_commit(commit: &CommitDoc, issues: &[IssueDoc]) -> Vec<String> {
    let message = commit.message.to_lowercase();
    let message_terms = query_terms(&commit.message);
    let mut linked = Vec::new();
    for issue in issues {
        let short: String = issue.id.chars().take(8).collect();
        let mentioned = (issue.id.len() >= 8 && message.contains(&issue.id.to_lowercase()))
            || message.contains(&format!("#{}", short.to_lowercase()));
        let terms = query_terms(&issue.title);
        let shared = terms
            .iter()
            .filter(|term| message_terms.contains(term))
            .count();
        let overlaps = terms.len() >= 2 && shared >= 2 && shared * 10 >= terms.len() * 6;
        if mentioned || overlaps {
            linked.push(issue.id.clone());
        }
    }
    linked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(id: &str, title: &str) -> IssueDoc {
        IssueDoc {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            status: "Todo".into(),
            priority: None,
            assignee: None,
            tags: Vec::new(),
        }
    }

    fn commit(message: &str) -> CommitDoc {
        CommitDoc {
            sha: "c1".into(),
            author: "Maya".into(),
            date: 0,
            message: message.into(),
            files: Vec::new(),
        }
    }

    #[test]
    fn git_log_output_is_read_into_commits_with_their_files() {
        let log = format!(
            "{RECORD}aaa{FIELD}Maya{FIELD}1700000000{FIELD}Add retry\n\nIt gave up too early.\n{FIELD}\n\nsrc/upload.rs\nsrc/retry.rs\n\
             {RECORD}bbb{FIELD}Sam{FIELD}1700000500{FIELD}Fix typo{FIELD}\n\nREADME.md\n"
        );
        let commits = parse_log(&log);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "aaa");
        assert_eq!(commits[0].author, "Maya");
        assert_eq!(commits[0].date, 1_700_000_000);
        assert_eq!(commits[0].message, "Add retry\n\nIt gave up too early.");
        assert_eq!(commits[0].files, ["src/upload.rs", "src/retry.rs"]);
        assert_eq!(commits[1].files, ["README.md"]);
    }

    #[test]
    fn an_empty_log_has_no_commits() {
        assert!(parse_log("").is_empty());
    }

    #[test]
    fn a_commit_is_about_an_issue_it_mentions() {
        let issues = [issue(
            "1a2b3c4d-0000-0000-0000-000000000000",
            "Pricing page",
        )];
        assert_eq!(
            issues_for_commit(&commit("Work on #1a2b3c4d"), &issues).len(),
            1
        );
        assert!(issues_for_commit(&commit("Unrelated change"), &issues).is_empty());
    }

    #[test]
    fn a_commit_is_about_an_issue_whose_title_words_it_mostly_uses() {
        let issues = [
            issue("a", "Fix login timeout"),
            issue("b", "Add export to PDF"),
        ];
        let linked = issues_for_commit(&commit("Fix the login timeout when idle"), &issues);
        assert_eq!(linked, ["a"]);
    }

    #[test]
    fn one_shared_word_is_not_enough() {
        let issues = [issue("a", "Fix login timeout")];
        assert!(issues_for_commit(&commit("Fix typo in README"), &issues).is_empty());
        assert!(issues_for_commit(&commit("Login page styling"), &issues).is_empty());
    }

    #[test]
    fn a_repository_with_history_reports_its_head_branch_and_commits() {
        let dir = tempfile::TempDir::new().unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output();
            matches!(status, Ok(output) if output.status.success())
        };
        if !run(&["init", "-q", "-b", "main"]) {
            return; // No git on this machine, so there is nothing to check.
        }
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "Tester"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "First commit"]);
        std::fs::write(dir.path().join("b.txt"), "two").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "Second commit\n\nWith a body."]);

        assert!(head(dir.path()).is_some());
        assert_eq!(branch(dir.path()).as_deref(), Some("main"));
        let commits = recent_commits(dir.path(), 10, None).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].message, "Second commit\n\nWith a body.");
        assert_eq!(commits[0].files, ["b.txt"]);
        let newest = recent_commits(dir.path(), 10, Some(&commits[1].sha)).unwrap();
        assert_eq!(newest.len(), 1);
        // A commit that is not in the history falls back to the newest ones.
        assert_eq!(
            recent_commits(dir.path(), 10, Some("deadbeef"))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn working_changes_show_edits_but_never_the_files_that_look_secret() {
        let dir = tempfile::TempDir::new().unwrap();
        let run = |args: &[&str]| {
            matches!(
                Command::new("git").arg("-C").arg(dir.path()).args(args).output(),
                Ok(output) if output.status.success()
            )
        };
        if !run(&["init", "-q", "-b", "main"]) {
            return;
        }
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "Tester"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=old\n").unwrap();
        run(&["add", "-f", "."]);
        run(&["commit", "-q", "-m", "First"]);
        assert_eq!(working_changes(dir.path(), 10_000), "");

        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=new-secret-value\n").unwrap();
        let changes = working_changes(dir.path(), 10_000);
        assert!(changes.contains("+two"), "{changes}");
        assert!(!changes.contains("new-secret-value"), "{changes}");

        let short = working_changes(dir.path(), 20);
        assert!(short.contains("not shown"));
    }

    #[test]
    fn a_folder_that_is_not_a_repository_has_no_head() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(head(dir.path()), None);
    }
}
