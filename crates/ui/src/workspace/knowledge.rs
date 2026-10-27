//! The knowledge base in the running app. One background indexer works for the open project's repository: it reads
//! the code, docs, commits and issues into the knowledge database, then watches the repository and keeps the
//! database current as files change. The UI never waits on it. It reads its status, and asks it for searches.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use gpui::{prelude::*, *};
use looji_domain::{
    CodeSourceRepository, DEFAULT_TOKEN_BUDGET, Hunk, MAX_HUNKS, ProjectId, Task, TaskId,
    pack_hunks,
};
use looji_knowledge::{
    Health, IssueDoc, KnowledgeStore, SearchQuery, SourceKind, Syncer, git, watch::RepoWatcher,
};

use super::{WorkspaceView, board::status_label, members, store};
use crate::{database::AppDatabase, theme::Theme};

/// How long the watcher waits for the files to go quiet before it reports a batch.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// How often a repository that can't report changes is scanned.
const POLL_EVERY: Duration = Duration::from_secs(3);
/// How often the app checks whether the indexer matches the open project and its issues.
const TICK: Duration = Duration::from_secs(1);
/// Commits read on a first index.
const FIRST_COMMITS: usize = 300;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Starting,
    Indexing,
    Watching,
    Failed(String),
}

/// What the indexer reports. The settings view draws it.
#[derive(Debug, Clone, Default)]
pub(super) struct Status {
    pub phase: Phase,
    pub health: Option<Health>,
    /// Changes are found by scanning, because the system can't report them for this folder.
    pub polling: bool,
}

enum Command {
    Issues(Vec<IssueDoc>),
    Refresh,
    Stop,
}

/// The indexer for one project, and the handle the UI keeps.
pub(super) struct Service {
    project: ProjectId,
    root: String,
    commands: Sender<Command>,
    pub status: Arc<Mutex<Status>>,
    issues_fingerprint: u64,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        // The worker notices within a moment. It is not waited for, so closing a project never stalls the window.
        drop(self.worker.take());
    }
}

impl Service {
    fn start(
        project: ProjectId,
        root: String,
        database: PathBuf,
        issues: Vec<IssueDoc>,
        fingerprint: u64,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let worker = {
            let (status, root, key) = (
                Arc::clone(&status),
                root.clone(),
                project.as_uuid().to_string(),
            );
            thread::Builder::new()
                .name("looji-knowledge".into())
                .spawn(move || {
                    run(
                        &key,
                        Path::new(&root),
                        &database,
                        issues,
                        &receiver,
                        &status,
                    )
                })
                .ok()
        };
        Self {
            project,
            root,
            commands,
            status,
            issues_fingerprint: fingerprint,
            worker,
        }
    }

    pub(super) fn snapshot(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

fn update(status: &Mutex<Status>, change: impl FnOnce(&mut Status)) {
    if let Ok(mut status) = status.lock() {
        change(&mut status);
    }
}

/// The indexer's whole life: read everything once, then follow the repository until told to stop.
fn run(
    project: &str,
    root: &Path,
    database: &Path,
    mut issues: Vec<IssueDoc>,
    commands: &Receiver<Command>,
    status: &Mutex<Status>,
) {
    let fail = |message: String| update(status, |status| status.phase = Phase::Failed(message));
    let mut store = match KnowledgeStore::open(database) {
        Ok(store) => store,
        Err(err) => return fail(err.to_string()),
    };
    let mut syncer = match Syncer::new() {
        Ok(syncer) => syncer,
        Err(err) => return fail(err.to_string()),
    };
    update(status, |status| status.phase = Phase::Indexing);

    // The watcher starts first, so a file saved during the first index is reported afterwards, not missed.
    let (watcher, changes) = match RepoWatcher::start(root, DEBOUNCE, POLL_EVERY, false) {
        Ok(started) => started,
        Err(err) => return fail(err.to_string()),
    };
    update(status, |status| status.polling = watcher.polling);

    let refresh_health = |store: &KnowledgeStore| {
        if let Ok(health) = store.health(project) {
            update(status, |status| status.health = Some(health));
        }
    };
    let index_everything =
        |store: &mut KnowledgeStore, syncer: &mut Syncer, issues: &[IssueDoc]| {
            let outcome = syncer
                .sync_repo(store, project, root)
                .and_then(|_| syncer.sync_issues(store, project, issues));
            if let Err(err) = outcome {
                eprintln!("looji: knowledge index failed: {err}");
            }
            take_new_commits(store, syncer, project, root, issues);
            refresh_health(store);
        };
    index_everything(&mut store, &mut syncer, &issues);
    update(status, |status| status.phase = Phase::Watching);

    loop {
        match commands.recv_timeout(Duration::from_millis(150)) {
            Ok(Command::Issues(list)) => {
                issues = list;
                if let Err(err) = syncer.sync_issues(&mut store, project, &issues) {
                    eprintln!("looji: knowledge issues failed: {err}");
                }
                refresh_health(&store);
            }
            Ok(Command::Refresh) => {
                update(status, |status| status.phase = Phase::Indexing);
                index_everything(&mut store, &mut syncer, &issues);
                update(status, |status| status.phase = Phase::Watching);
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let mut touched = false;
        while let Ok(change) = changes.try_recv() {
            touched = true;
            if let Err(err) = syncer.apply_paths(&mut store, project, root, &change.paths) {
                eprintln!("looji: knowledge update failed: {err}");
            }
            if change.git_changed {
                take_new_commits(&mut store, &syncer, project, root, &issues);
            }
        }
        if touched {
            refresh_health(&store);
        }
    }
    drop(watcher);
}

/// Reads the commits made since the last look, adds them, and links each to the issues it is about.
fn take_new_commits(
    store: &mut KnowledgeStore,
    syncer: &Syncer,
    project: &str,
    root: &Path,
    issues: &[IssueDoc],
) {
    let Some(head) = git::head(root) else {
        return;
    };
    let seen = store.meta(project, "head").ok().flatten();
    if seen.as_deref() == Some(head.as_str()) {
        return;
    }
    let Ok(commits) = git::recent_commits(root, FIRST_COMMITS, seen.as_deref()) else {
        return;
    };
    if syncer.sync_commits(store, project, &commits).is_err() {
        return;
    }
    for commit in &commits {
        for issue in git::issues_for_commit(commit, issues) {
            let _ = store.link_commit(project, &commit.sha, &issue);
        }
    }
    let _ = store.set_meta(project, "head", &head);
}

/// The issues of a board as the knowledge base sees them.
fn issue_docs(tasks: &[Task], members: &[looji_domain::TeamMember]) -> Vec<IssueDoc> {
    let entries = members::entries(members);
    tasks
        .iter()
        .map(|task| IssueDoc {
            id: task.id().as_uuid().to_string(),
            title: task.title().to_string(),
            description: task.description().to_string(),
            status: status_label(task.status()).to_string(),
            priority: task
                .priority()
                .map(|priority| priority.as_str().to_string()),
            assignee: task
                .assignee_id()
                .and_then(|id| entries.iter().find(|entry| entry.id == id))
                .map(|entry| entry.label.clone()),
            tags: task.tags().to_vec(),
        })
        .collect()
}

/// A number that changes when any task on the board does, so the indexer is only told when it matters.
fn fingerprint(tasks: &[Task]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for task in tasks {
        task.id().as_uuid().hash(&mut hasher);
        task.updated_at().unix_millis().hash(&mut hasher);
    }
    tasks.len().hash(&mut hasher);
    hasher.finish()
}

impl WorkspaceView {
    /// Starts the loop that keeps the indexer matched to the open project. It looks once a second, which costs a
    /// single small query.
    pub(super) fn start_knowledge(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            loop {
                if this
                    .update(cx, |view, cx| view.reconcile_knowledge(cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(TICK).await;
            }
        });
        self.knowledge_task = Some(task);
    }

    /// Makes the indexer the one for the open project, and gives it the issues when they have changed.
    fn reconcile_knowledge(&mut self, cx: &mut Context<Self>) {
        let wanted = self.projects.active_id().and_then(|project| {
            let root = store(cx).load_path(project).ok().flatten()?;
            Path::new(&root).is_dir().then_some((project, root))
        });
        let running = self
            .knowledge
            .as_ref()
            .map(|service| (service.project, service.root.clone()));
        if wanted != running {
            self.knowledge = None;
            if let Some((project, root)) = wanted {
                let database = cx.global::<AppDatabase>().knowledge_path.clone();
                let issues = issue_docs(&self.board, &self.members);
                let fingerprint = fingerprint(&self.board);
                self.knowledge = Some(Service::start(project, root, database, issues, fingerprint));
            }
            cx.notify();
            return;
        }
        let current = fingerprint(&self.board);
        let docs = match self.knowledge.as_ref() {
            Some(service) if service.issues_fingerprint != current => {
                Some(issue_docs(&self.board, &self.members))
            }
            _ => None,
        };
        if let (Some(service), Some(docs)) = (self.knowledge.as_mut(), docs) {
            let _ = service.commands.send(Command::Issues(docs));
            service.issues_fingerprint = current;
        }
        if self.project_settings_open {
            cx.notify();
        }
    }

    /// Reads the project again from the start, for when the index looks wrong.
    pub(super) fn reindex_knowledge(&mut self, cx: &mut Context<Self>) {
        if let Some(service) = &self.knowledge {
            let _ = service.commands.send(Command::Refresh);
        }
        cx.notify();
    }
}

/// The hunks to show the model for a question, from the knowledge base. `None` when the project hasn't been indexed
/// yet, so the caller can fall back to reading the repository directly.
/// The project's other issues that look like this one, read from the knowledge base. Empty until the project has been
/// indexed, and when anything goes wrong, since this is a hint and not something the user asked for.
pub(super) fn related_for(
    knowledge_path: &Path,
    project: ProjectId,
    task: TaskId,
    title: &str,
    description: &str,
) -> Vec<looji_knowledge::related::RelatedIssue> {
    let Ok(store) = looji_knowledge::KnowledgeStore::open(knowledge_path) else {
        return Vec::new();
    };
    store
        .related_issues(
            &project.as_uuid().to_string(),
            title,
            description,
            Some(&task.as_uuid().to_string()),
            3,
        )
        .unwrap_or_default()
}

pub(super) fn search_hunks(
    database: &Path,
    project: ProjectId,
    question: &str,
) -> Option<Vec<Hunk>> {
    let store = KnowledgeStore::open(database).ok()?;
    let key = project.as_uuid().to_string();
    if store.health(&key).ok()?.code_chunks == 0 {
        return None;
    }
    let hits = store
        .search(
            &key,
            &SearchQuery {
                text: question.to_string(),
                kinds: vec![SourceKind::Code, SourceKind::Doc],
                limit: 24,
                ..Default::default()
            },
        )
        .ok()?;
    let candidates: Vec<Hunk> = hits
        .into_iter()
        .filter_map(|hit| {
            Some(Hunk {
                path: hit.path?,
                start_line: hit.start_line,
                end_line: hit.end_line,
                // Scores are small floats. Scaled, so the domain's integer packing keeps their order.
                score: ((hit.score * 100.0).round() as u32).max(1),
                text: hit.text,
            })
        })
        .collect();
    Some(pack_hunks(candidates, DEFAULT_TOKEN_BUDGET, MAX_HUNKS))
}

/// The index health, as a quiet block for the repository settings: what is indexed, how fresh it is, what was held
/// back, and a way to read the project again.
pub(super) fn health_block(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let service = view.knowledge.as_ref()?;
    let status = service.snapshot();
    let line = |text: String| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(text)
    };
    let headline = match &status.phase {
        Phase::Starting | Phase::Indexing => "Reading the project\u{2026}".to_string(),
        Phase::Failed(message) => format!("The index stopped: {message}"),
        Phase::Watching if status.polling => {
            "Up to date. Checking for changes every few seconds.".to_string()
        }
        Phase::Watching => "Up to date. Following changes as they are saved.".to_string(),
    };
    let mut block = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(theme.typography.body)
                .child("What the AI knows"),
        )
        .child(line(headline));
    if let Some(health) = &status.health {
        block = block.child(line(format!(
            "{} code, {} doc, {} issue and {} commit passages from {} sources.",
            health.code_chunks,
            health.doc_chunks,
            health.issue_chunks,
            health.commit_chunks,
            health.sources
        )));
        if let Some(at) = health.last_indexed_at {
            block = block.child(line(format!("Last change read {}.", ago(at))));
        }
        for (path, note) in health.notes.iter().take(5) {
            block = block.child(line(format!("{path}: {note}")));
        }
        if health.notes.len() > 5 {
            block = block.child(line(format!("and {} more.", health.notes.len() - 5)));
        }
    }
    block = block.child(
        div().flex().flex_row().child(
            crate::components::button(
                "knowledge-reindex",
                "Read again",
                crate::components::Variant::Ghost,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reindex_knowledge(cx))),
        ),
    );
    Some(block.into_any_element())
}

/// How long ago a time was, in words.
fn ago(at_millis: i64) -> String {
    let now = looji_domain::Timestamp::now().unix_millis();
    let seconds = ((now - at_millis) / 1000).max(0);
    match seconds {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{seconds} seconds ago"),
        60..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=86_399 => format!("{} hours ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::ago;

    #[test]
    fn times_are_told_in_the_largest_fitting_unit() {
        let now = looji_domain::Timestamp::now().unix_millis();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - 30_000), "30 seconds ago");
        assert_eq!(ago(now - 5 * 60_000), "5 minutes ago");
        assert_eq!(ago(now - 3 * 3_600_000), "3 hours ago");
        assert_eq!(ago(now - 2 * 86_400_000), "2 days ago");
    }
}
