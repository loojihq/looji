//! Noticing changes in a repository, so the knowledge base follows the code without being told to.
//!
//! The operating system reports file changes, and they are collected into batches: a save from an editor is often
//! several events, and a branch switch is thousands. Where the system can't report changes, such as a folder inside
//! WSL or on a network drive, the tree is scanned every few seconds and compared with the last scan instead.
//! Either way the output is the same: batches of changed paths, and a flag for when git moved.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

use notify::{RecursiveMode, Watcher};

use crate::{Error, Result, safety};

/// One batch of changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Files that changed, relative to the repository with `/` between parts. A deleted file is listed too.
    pub paths: Vec<String>,
    /// HEAD or a branch moved: a commit, a checkout, a pull.
    pub git_changed: bool,
}

/// A running watcher. Dropping it stops it.
pub struct RepoWatcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _native: Option<notify::RecommendedWatcher>,
    /// True when changes come from scanning, not from the operating system.
    pub polling: bool,
}

impl Drop for RepoWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Whether the system can't be expected to report changes under `root`: a UNC path, which is how WSL and network
/// shares are reached from Windows.
pub fn needs_polling(root: &Path) -> bool {
    let text = root.to_string_lossy();
    text.starts_with("\\\\") || text.starts_with("//")
}

impl RepoWatcher {
    /// Starts watching `root`. Batches arrive on the receiver once the tree has been quiet for `debounce`. With
    /// `poll_every`, or when the system can't report changes, the tree is scanned that often instead.
    pub fn start(
        root: &Path,
        debounce: Duration,
        poll_every: Duration,
        force_polling: bool,
    ) -> Result<(Self, Receiver<Change>)> {
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let root = root.to_path_buf();

        if !force_polling && !needs_polling(&root) {
            let (events, incoming) = mpsc::channel();
            let watcher = notify::recommended_watcher(move |event| {
                let _ = events.send(event);
            })
            .and_then(|mut watcher| {
                watcher.watch(&root, RecursiveMode::Recursive)?;
                Ok(watcher)
            });
            if let Ok(watcher) = watcher {
                let thread = {
                    let (stop, root) = (Arc::clone(&stop), root.clone());
                    thread::spawn(move || collect(&root, incoming, sender, debounce, &stop))
                };
                return Ok((
                    Self {
                        stop,
                        thread: Some(thread),
                        _native: Some(watcher),
                        polling: false,
                    },
                    receiver,
                ));
            }
            // The system refused, so scanning does the job instead.
        }

        let (first_scan, first_head) = (snapshot(&root), head_marker(&root));
        let thread = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("looji-poll".into())
                .spawn(move || poll(&root, sender, poll_every, &stop, first_scan, first_head))
                .map_err(|err| Error::Watch(err.to_string()))?
        };
        Ok((
            Self {
                stop,
                thread: Some(thread),
                _native: None,
                polling: true,
            },
            receiver,
        ))
    }
}

/// Whether a changed path is one of the files git moves when a commit or a checkout happens.
fn is_git_marker(relative: &str) -> bool {
    relative == ".git/HEAD"
        || relative == ".git/packed-refs"
        || relative == ".git/ORIG_HEAD"
        || relative.starts_with(".git/refs/")
}

/// Adds what one event says to the batch being collected.
fn absorb(root: &Path, event: notify::Event, paths: &mut BTreeSet<String>, git_changed: &mut bool) {
    for path in event.paths {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if is_git_marker(&relative) {
            *git_changed = true;
        } else if safety::classify(&relative) != safety::FileClass::Skip
            || safety::is_secret_path(&relative)
        {
            paths.insert(relative);
        }
    }
}

/// Collects the operating system's events into batches. A batch is sent once nothing more has arrived for
/// `debounce`, so a burst of saves is one batch.
fn collect(
    root: &Path,
    incoming: Receiver<notify::Result<notify::Event>>,
    sender: Sender<Change>,
    debounce: Duration,
    stop: &AtomicBool,
) {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    let mut git_changed = false;
    while !stop.load(Ordering::Relaxed) {
        match incoming.recv_timeout(Duration::from_millis(100)) {
            Ok(first) => {
                if let Ok(event) = first {
                    absorb(root, event, &mut paths, &mut git_changed);
                }
                // Keep collecting until it goes quiet.
                let mut last_event = Instant::now();
                while last_event.elapsed() < debounce && !stop.load(Ordering::Relaxed) {
                    match incoming.recv_timeout(Duration::from_millis(20)) {
                        Ok(Ok(event)) => {
                            absorb(root, event, &mut paths, &mut git_changed);
                            last_event = Instant::now();
                        }
                        Ok(Err(_)) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                if !paths.is_empty() || git_changed {
                    let change = Change {
                        paths: std::mem::take(&mut paths).into_iter().collect(),
                        git_changed: std::mem::take(&mut git_changed),
                    };
                    if sender.send(change).is_err() {
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// What a scan records about a file, to tell whether it changed.
type Snapshot = HashMap<String, (SystemTime, u64)>;

fn snapshot(root: &Path) -> Snapshot {
    let mut files: Snapshot = HashMap::new();
    for path in safety::indexable_files(root) {
        if let Ok(metadata) = std::fs::metadata(root.join(&path)) {
            files.insert(
                path,
                (
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    metadata.len(),
                ),
            );
        }
    }
    files
}

/// The head commit read straight from the files, so scanning needs no `git` program.
fn head_marker(root: &Path) -> String {
    let head = std::fs::read_to_string(root.join(".git/HEAD")).unwrap_or_default();
    let reference = head.trim().strip_prefix("ref: ").map(str::to_string);
    let commit = reference
        .as_ref()
        .and_then(|name| std::fs::read_to_string(root.join(".git").join(name)).ok())
        .unwrap_or_default();
    format!("{}|{}", head.trim(), commit.trim())
}

/// Scans the tree every `every` and sends what changed since the last scan.
fn poll(
    root: &Path,
    sender: Sender<Change>,
    every: Duration,
    stop: &AtomicBool,
    mut before: Snapshot,
    mut head: String,
) {
    loop {
        let mut waited = Duration::ZERO;
        while waited < every {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let step = Duration::from_millis(50).min(every - waited);
            thread::sleep(step);
            waited += step;
        }
        let after = snapshot(root);
        let mut changed: BTreeSet<String> = BTreeSet::new();
        for (path, state) in &after {
            if before.get(path) != Some(state) {
                changed.insert(path.clone());
            }
        }
        for path in before.keys() {
            if !after.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        let marker = head_marker(root);
        let git_changed = marker != head;
        before = after;
        head = marker;
        if (!changed.is_empty() || git_changed)
            && sender
                .send(Change {
                    paths: changed.into_iter().collect(),
                    git_changed,
                })
                .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn wait(receiver: &Receiver<Change>, seconds: u64) -> Option<Change> {
        receiver.recv_timeout(Duration::from_secs(seconds)).ok()
    }

    #[test]
    fn scanning_reports_a_new_changed_and_deleted_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("old.rs"), "fn a() {}").unwrap();
        let (_watcher, changes) = RepoWatcher::start(
            dir.path(),
            Duration::from_millis(50),
            Duration::from_millis(100),
            true,
        )
        .unwrap();

        fs::write(dir.path().join("new.rs"), "fn b() {}").unwrap();
        let first = wait(&changes, 5).expect("a batch for the new file");
        assert_eq!(first.paths, ["new.rs"]);

        fs::write(dir.path().join("new.rs"), "fn b() { changed(); }").unwrap();
        let second = wait(&changes, 5).expect("a batch for the changed file");
        assert_eq!(second.paths, ["new.rs"]);

        fs::remove_file(dir.path().join("old.rs")).unwrap();
        let third = wait(&changes, 5).expect("a batch for the deleted file");
        assert_eq!(third.paths, ["old.rs"]);
    }

    #[test]
    fn scanning_ignores_files_the_knowledge_base_would_not_read() {
        let dir = TempDir::new().unwrap();
        let (_watcher, changes) = RepoWatcher::start(
            dir.path(),
            Duration::from_millis(50),
            Duration::from_millis(100),
            true,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/dep.js"), "x").unwrap();
        fs::write(dir.path().join("logo.png"), "x").unwrap();
        assert!(wait(&changes, 1).is_none());
        fs::write(dir.path().join("real.rs"), "fn real() {}").unwrap();
        assert_eq!(wait(&changes, 5).unwrap().paths, ["real.rs"]);
    }

    #[test]
    fn scanning_notices_when_git_moves() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git/refs/heads")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(dir.path().join(".git/refs/heads/main"), "aaa\n").unwrap();
        let (_watcher, changes) = RepoWatcher::start(
            dir.path(),
            Duration::from_millis(50),
            Duration::from_millis(100),
            true,
        )
        .unwrap();

        fs::write(dir.path().join(".git/refs/heads/main"), "bbb\n").unwrap();

        let change = wait(&changes, 5).expect("a batch for the commit");
        assert!(change.git_changed);
        assert!(change.paths.is_empty());
    }

    #[test]
    fn the_operating_system_reports_a_save_as_one_batch() {
        let dir = TempDir::new().unwrap();
        let (watcher, changes) = RepoWatcher::start(
            dir.path(),
            Duration::from_millis(200),
            Duration::from_secs(60),
            false,
        )
        .unwrap();
        if watcher.polling {
            return; // This machine cannot watch natively, so the scanning tests cover it.
        }
        // Give the watcher a moment to attach before the first write.
        thread::sleep(Duration::from_millis(300));
        for index in 0..3 {
            fs::write(dir.path().join("saved.rs"), format!("fn v{index}() {{}}")).unwrap();
        }
        let change = wait(&changes, 10).expect("a batch for the saves");
        assert_eq!(change.paths, ["saved.rs"]);
    }

    #[test]
    fn unc_paths_are_scanned_because_the_system_does_not_report_them() {
        assert!(needs_polling(Path::new(
            "\\\\wsl.localhost\\Ubuntu\\home\\me\\project"
        )));
        assert!(!needs_polling(Path::new("C:\\Users\\me\\project")));
        assert!(!needs_polling(Path::new("/home/me/project")));
    }

    #[test]
    fn dropping_the_watcher_stops_it() {
        let dir = TempDir::new().unwrap();
        let (watcher, changes) = RepoWatcher::start(
            dir.path(),
            Duration::from_millis(50),
            Duration::from_millis(50),
            true,
        )
        .unwrap();
        drop(watcher);
        fs::write(dir.path().join("late.rs"), "fn late() {}").unwrap();
        assert!(changes.recv_timeout(Duration::from_millis(400)).is_err());
    }
}
