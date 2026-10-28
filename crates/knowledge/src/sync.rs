//! Reading a repository, its issues, and its commits into the knowledge base.
//!
//! Every source is identified by a hash of its content. A source whose hash hasn't changed is not read again, so a
//! full sync of an unchanged repository only walks the tree and hashes files, and a change to one file costs one
//! file. `apply_paths` is the same work for just the paths a watcher reported.

use std::{collections::HashSet, fs, path::Path};

use looji_symbols::Extractor;
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    chunk::{
        Chunk, CommitDoc, IssueDoc, SourceKind, chunk_code, chunk_commit, chunk_issue,
        chunk_markdown, chunk_plain,
    },
    safety::{self, FileClass, MAX_FILE_BYTES, MAX_PDF_BYTES},
    store::{KnowledgeStore, SourceMeta},
};

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Sources looked at.
    pub scanned: usize,
    /// Sources that were new or changed, and so were read again.
    pub indexed: usize,
    /// Sources whose hash had not changed.
    pub unchanged: usize,
    /// Sources that are gone, and were forgotten.
    pub removed: usize,
    /// Files held back: secrets, binaries, and files over the size limit.
    pub held_back: usize,
}

impl SyncReport {
    fn absorb(&mut self, outcome: Outcome) {
        self.scanned += 1;
        match outcome {
            Outcome::Indexed => self.indexed += 1,
            Outcome::Unchanged => self.unchanged += 1,
            Outcome::Removed => self.removed += 1,
            Outcome::HeldBack => self.held_back += 1,
            Outcome::Ignored => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Indexed,
    Unchanged,
    Removed,
    HeldBack,
    Ignored,
}

/// Blueprint documents are indexed as documents under this made-up folder, so a search can find the design and a
/// source list can tell it apart from a file.
pub const BLUEPRINT_PREFIX: &str = "blueprint/";

/// Files attached to tasks are indexed as documents under this made-up folder, named by their stored name.
pub const ATTACHMENT_PREFIX: &str = "attachment/";

/// A file attached to a task or a blueprint section, as the knowledge base is told about it.
#[derive(Debug, Clone)]
pub struct AttachedFile {
    /// The name the file has in the attachments folder.
    pub stored_name: String,
    /// The name the user knows it by.
    pub file_name: String,
    /// Where it is kept, in words, such as "task: Fix refunds" or "blueprint: architecture".
    pub origin: String,
    /// Where the file is.
    pub path: std::path::PathBuf,
}

/// Whether a file name says the file is a picture whose words can be read.
fn is_picture(file_name: &str) -> bool {
    file_name.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "gif"
        )
    })
}

/// Puts `title` in front of each chunk's label, so a passage says which attachment it came from.
fn titled(chunks: &mut [Chunk], title: &str) {
    for chunk in chunks {
        chunk.label = Some(match chunk.label.take() {
            Some(label) => format!("{title} · {label}"),
            None => title.to_string(),
        });
    }
}

pub struct Syncer {
    extractor: Extractor,
}

/// A hash of some bytes, as lowercase hex.
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Syncer {
    pub fn new() -> Result<Self> {
        let extractor = Extractor::new().map_err(|err| Error::Parser(err.to_string()))?;
        Ok(Self { extractor })
    }

    /// Brings the knowledge base's code and docs for `project` in line with the repository at `root`: new and
    /// changed files are read, and files that are gone, or that the ignore rules now leave out, are forgotten.
    pub fn sync_repo(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        root: &Path,
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let files = safety::indexable_files(root);
        let present: HashSet<&str> = files.iter().map(String::as_str).collect();
        for path in &files {
            let outcome = self.index_file(store, project, root, path)?;
            report.absorb(outcome);
        }
        for kind in [SourceKind::Code, SourceKind::Doc] {
            for reference in store.source_refs(project, kind)? {
                // Blueprint documents are not files, so a missing file is not a reason to drop them.
                let not_a_file = reference.starts_with(BLUEPRINT_PREFIX)
                    || reference.starts_with(ATTACHMENT_PREFIX);
                if !present.contains(reference.as_str()) && !not_a_file {
                    store.remove_source(project, kind, &reference)?;
                    report.scanned += 1;
                    report.removed += 1;
                }
            }
        }
        // Files held back as secrets are listed in the health view, so the user can see they were seen and kept out.
        for path in safety::secret_files(root) {
            let outcome = self.hold_back(
                store,
                project,
                &path,
                "held back: the name says it holds a secret",
            )?;
            report.absorb(outcome);
        }
        store.set_meta(
            project,
            "synced_at",
            &crate::store::now_millis().to_string(),
        )?;
        Ok(report)
    }

    /// The same work for a few paths, relative to `root`. A path that is a folder, or that no longer exists, is
    /// forgotten if it was known.
    pub fn apply_paths(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        root: &Path,
        paths: &[String],
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        for path in paths {
            let path = path.replace('\\', "/");
            if root.join(&path).is_dir() {
                continue;
            }
            let outcome = self.index_file(store, project, root, &path)?;
            report.absorb(outcome);
        }
        Ok(report)
    }

    fn hold_back(
        &self,
        store: &mut KnowledgeStore,
        project: &str,
        path: &str,
        note: &str,
    ) -> Result<Outcome> {
        let kind = match safety::classify(path) {
            FileClass::Doc | FileClass::Pdf => SourceKind::Doc,
            _ if path.starts_with(ATTACHMENT_PREFIX) => SourceKind::Doc,
            _ => SourceKind::Code,
        };
        store.replace_source(
            project,
            kind,
            path,
            &SourceMeta::new("held-back", 0, Some(note)),
            &[],
        )?;
        Ok(Outcome::HeldBack)
    }

    /// Reads a PDF's text into the knowledge base, a chunk or more for each page, labelled with the page. A PDF with
    /// no text layer, or one that can't be read, is held back with the reason.
    fn index_pdf(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        path: &str,
        bytes: &[u8],
        title: Option<&str>,
    ) -> Result<Outcome> {
        let hash = hash_bytes(bytes);
        if store
            .source_hash(project, SourceKind::Doc, path)?
            .as_deref()
            == Some(hash.as_str())
        {
            return Ok(Outcome::Unchanged);
        }
        let mut scanned = false;
        let pages = match crate::pdf::read(bytes) {
            crate::pdf::Reading::Text(pages) => pages,
            crate::pdf::Reading::NoTextLayer => match crate::ocr::pdf_pages(bytes) {
                Ok(pages) => {
                    scanned = true;
                    pages
                }
                Err(reason) => {
                    let note = format!("held back: no text layer, and {reason}");
                    return self.hold_back(store, project, path, &note);
                }
            },
            crate::pdf::Reading::Unreadable(reason) => {
                return self.hold_back(store, project, path, &format!("held back: {reason}"));
            }
        };
        let mut chunks = Vec::new();
        let mut redacted_any = false;
        for (number, text) in pages {
            let Some((text, redacted)) = safety::redact(&text) else {
                return self.hold_back(store, project, path, "held back: contains a private key");
            };
            redacted_any |= redacted;
            for mut chunk in chunk_plain(path, SourceKind::Doc, &text) {
                chunk.label = Some(match title {
                    Some(title) => format!("{title} · page {number}"),
                    None => format!("page {number}"),
                });
                chunks.push(chunk);
            }
        }
        let mut notes = Vec::new();
        if scanned {
            notes.push("text read from scanned pages with OCR");
        }
        if redacted_any {
            notes.push("secret text was taken out before it was indexed");
        }
        let note = (!notes.is_empty()).then(|| notes.join("; "));
        store.replace_source(
            project,
            SourceKind::Doc,
            path,
            &SourceMeta::new(&hash, bytes.len() as u64, note.as_deref()),
            &chunks,
        )?;
        Ok(Outcome::Indexed)
    }

    /// Reads one file into the knowledge base, unless it hasn't changed.
    fn index_file(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        root: &Path,
        path: &str,
    ) -> Result<Outcome> {
        let class = safety::classify(path);
        let full = root.join(path);
        let exists = full.is_file();

        if !exists || class == FileClass::Skip {
            let mut removed = false;
            for kind in [SourceKind::Code, SourceKind::Doc] {
                if store.source_hash(project, kind, path)?.is_some() {
                    store.remove_source(project, kind, path)?;
                    removed = true;
                }
            }
            if exists && safety::is_secret_path(path) {
                return self.hold_back(
                    store,
                    project,
                    path,
                    "held back: the name says it holds a secret",
                );
            }
            return Ok(if removed {
                Outcome::Removed
            } else {
                Outcome::Ignored
            });
        }
        let kind = match class {
            FileClass::Doc | FileClass::Pdf => SourceKind::Doc,
            _ => SourceKind::Code,
        };

        let metadata = fs::metadata(&full).map_err(|err| Error::Io(err.to_string()))?;
        if class == FileClass::Pdf {
            if metadata.len() > MAX_PDF_BYTES {
                return self.hold_back(store, project, path, "held back: larger than 20 MB");
            }
            let bytes = fs::read(&full).map_err(|err| Error::Io(err.to_string()))?;
            return self.index_pdf(store, project, path, &bytes, None);
        }
        if metadata.len() > MAX_FILE_BYTES {
            return self.hold_back(store, project, path, "held back: larger than 1 MB");
        }
        let bytes = fs::read(&full).map_err(|err| Error::Io(err.to_string()))?;
        if !safety::looks_like_text(&bytes) {
            return self.hold_back(store, project, path, "held back: not a text file");
        }
        let hash = hash_bytes(&bytes);
        if store.source_hash(project, kind, path)?.as_deref() == Some(hash.as_str()) {
            return Ok(Outcome::Unchanged);
        }
        let text = String::from_utf8_lossy(&bytes);
        let Some((text, redacted)) = safety::redact(&text) else {
            return self.hold_back(store, project, path, "held back: contains a private key");
        };

        let mut note =
            redacted.then(|| "secret text was taken out before it was indexed".to_string());
        let chunks: Vec<Chunk> = match class {
            FileClass::Code(language) => match self.extractor.extract(language, path, &text) {
                Ok(symbols) => chunk_code(path, &text, &symbols),
                Err(reason) => {
                    let why = format!("read as plain text: the file {reason}");
                    note = Some(match note {
                        Some(existing) => format!("{why}; {existing}"),
                        None => why,
                    });
                    chunk_plain(path, SourceKind::Code, &text)
                }
            },
            _ => chunk_markdown(path, &text),
        };
        store.replace_source(
            project,
            kind,
            path,
            &SourceMeta::new(&hash, bytes.len() as u64, note.as_deref()),
            &chunks,
        )?;
        Ok(Outcome::Indexed)
    }

    /// Brings the issue chunks in line with `issues`: new and changed issues are written, and issues that are gone
    /// are forgotten.
    /// Indexes the blueprint's documents, given as (kind, title, body). Documents that are gone are removed.
    pub fn sync_blueprint(
        &self,
        store: &mut KnowledgeStore,
        project: &str,
        docs: &[(String, String, String)],
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let mut present: HashSet<String> = HashSet::new();
        for (kind, title, body) in docs {
            let reference = format!("{BLUEPRINT_PREFIX}{kind}.md");
            if !present.insert(reference.clone()) {
                continue;
            }
            let text = format!(
                "# {title}

{body}"
            );
            let hash = hash_bytes(text.as_bytes());
            if store
                .source_hash(project, SourceKind::Doc, &reference)?
                .as_deref()
                == Some(hash.as_str())
            {
                report.absorb(Outcome::Unchanged);
                continue;
            }
            let chunks = chunk_markdown(&reference, &text);
            store.replace_source(
                project,
                SourceKind::Doc,
                &reference,
                &SourceMeta::new(&hash, text.len() as u64, Some("the project's blueprint")),
                &chunks,
            )?;
            report.absorb(Outcome::Indexed);
        }
        for reference in store.source_refs(project, SourceKind::Doc)? {
            if reference.starts_with(BLUEPRINT_PREFIX) && !present.contains(&reference) {
                store.remove_source(project, SourceKind::Doc, &reference)?;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Indexes the text of files attached to tasks: PDFs by page, and text and markdown files. Pictures, videos and
    /// other files have no text to read and are left out. An attachment that is gone is forgotten.
    pub fn sync_attachments(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        files: &[AttachedFile],
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let mut present: HashSet<String> = HashSet::new();
        for file in files {
            let class = safety::classify(&file.file_name);
            let picture = is_picture(&file.file_name);
            if !picture && !matches!(class, FileClass::Doc | FileClass::Pdf) {
                continue;
            }
            let reference = format!("{ATTACHMENT_PREFIX}{}", file.stored_name);
            let Ok(metadata) = fs::metadata(&file.path) else {
                continue;
            };
            present.insert(reference.clone());
            let title = format!("{} ({})", file.file_name, file.origin);
            let outcome = if picture {
                if metadata.len() > crate::ocr::MAX_IMAGE_BYTES {
                    self.hold_back(store, project, &reference, "held back: larger than 20 MB")?
                } else {
                    let bytes = fs::read(&file.path).map_err(|err| Error::Io(err.to_string()))?;
                    self.index_picture(store, project, &reference, &bytes, &title)?
                }
            } else if class == FileClass::Pdf {
                if metadata.len() > MAX_PDF_BYTES {
                    self.hold_back(store, project, &reference, "held back: larger than 20 MB")?
                } else {
                    let bytes = fs::read(&file.path).map_err(|err| Error::Io(err.to_string()))?;
                    self.index_pdf(store, project, &reference, &bytes, Some(&title))?
                }
            } else if metadata.len() > MAX_FILE_BYTES {
                self.hold_back(store, project, &reference, "held back: larger than 1 MB")?
            } else {
                let bytes = fs::read(&file.path).map_err(|err| Error::Io(err.to_string()))?;
                self.index_text(store, project, &reference, &bytes, &title)?
            };
            report.absorb(outcome);
        }
        for reference in store.source_refs(project, SourceKind::Doc)? {
            if reference.starts_with(ATTACHMENT_PREFIX) && !present.contains(&reference) {
                store.remove_source(project, SourceKind::Doc, &reference)?;
                report.scanned += 1;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Reads the words in a picture and indexes them like a document. A picture with no words, or one that can't be
    /// read, is held back with the reason.
    fn index_picture(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        reference: &str,
        bytes: &[u8],
        title: &str,
    ) -> Result<Outcome> {
        let hash = hash_bytes(bytes);
        if store
            .source_hash(project, SourceKind::Doc, reference)?
            .as_deref()
            == Some(hash.as_str())
        {
            return Ok(Outcome::Unchanged);
        }
        let text = match crate::ocr::text(bytes) {
            Ok(text) => text,
            Err(reason) => {
                return self.hold_back(store, project, reference, &format!("held back: {reason}"));
            }
        };
        let Some((text, redacted)) = safety::redact(&text) else {
            return self.hold_back(
                store,
                project,
                reference,
                "held back: contains a private key",
            );
        };
        let mut chunks = chunk_plain(reference, SourceKind::Doc, &text);
        titled(&mut chunks, title);
        let note = if redacted {
            "text read from a picture; secret text was taken out before it was indexed"
        } else {
            "text read from a picture"
        };
        store.replace_source(
            project,
            SourceKind::Doc,
            reference,
            &SourceMeta::new(&hash, bytes.len() as u64, Some(note)),
            &chunks,
        )?;
        Ok(Outcome::Indexed)
    }

    /// Reads an attached text file as a document, with its title in front of each passage's label.
    fn index_text(
        &mut self,
        store: &mut KnowledgeStore,
        project: &str,
        reference: &str,
        bytes: &[u8],
        title: &str,
    ) -> Result<Outcome> {
        if !safety::looks_like_text(bytes) {
            return self.hold_back(store, project, reference, "held back: not a text file");
        }
        let hash = hash_bytes(bytes);
        if store
            .source_hash(project, SourceKind::Doc, reference)?
            .as_deref()
            == Some(hash.as_str())
        {
            return Ok(Outcome::Unchanged);
        }
        let text = String::from_utf8_lossy(bytes);
        let Some((text, redacted)) = safety::redact(&text) else {
            return self.hold_back(
                store,
                project,
                reference,
                "held back: contains a private key",
            );
        };
        let mut chunks = chunk_markdown(reference, &text);
        titled(&mut chunks, title);
        let note = redacted.then_some("secret text was taken out before it was indexed");
        store.replace_source(
            project,
            SourceKind::Doc,
            reference,
            &SourceMeta::new(&hash, bytes.len() as u64, note),
            &chunks,
        )?;
        Ok(Outcome::Indexed)
    }

    pub fn sync_issues(
        &self,
        store: &mut KnowledgeStore,
        project: &str,
        issues: &[IssueDoc],
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let mut present: HashSet<&str> = HashSet::new();
        for issue in issues {
            present.insert(issue.id.as_str());
            let chunks = chunk_issue(issue);
            let hash = hash_bytes(
                chunks
                    .iter()
                    .map(|chunk| chunk.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\u{1}")
                    .as_bytes(),
            );
            if store
                .source_hash(project, SourceKind::Issue, &issue.id)?
                .as_deref()
                == Some(hash.as_str())
            {
                report.absorb(Outcome::Unchanged);
                continue;
            }
            let size: usize = chunks.iter().map(|chunk| chunk.text.len()).sum();
            store.replace_source(
                project,
                SourceKind::Issue,
                &issue.id,
                &SourceMeta::new(&hash, size as u64, None),
                &chunks,
            )?;
            store.put_issue(project, issue)?;
            report.absorb(Outcome::Indexed);
        }
        for reference in store.source_refs(project, SourceKind::Issue)? {
            if !present.contains(reference.as_str()) {
                store.remove_source(project, SourceKind::Issue, &reference)?;
                store.remove_issue(project, &reference)?;
                report.scanned += 1;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Adds commits the knowledge base doesn't have yet. A commit never changes, so a known one is left alone.
    pub fn sync_commits(
        &self,
        store: &mut KnowledgeStore,
        project: &str,
        commits: &[CommitDoc],
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        for commit in commits {
            if store
                .source_hash(project, SourceKind::Commit, &commit.sha)?
                .is_some()
            {
                report.absorb(Outcome::Unchanged);
                continue;
            }
            let chunk = chunk_commit(commit);
            let size = chunk.text.len() as u64;
            store.replace_source(
                project,
                SourceKind::Commit,
                &commit.sha,
                &SourceMeta::new(&commit.sha, size, None),
                &[chunk],
            )?;
            store.put_commit(project, commit)?;
            report.absorb(Outcome::Indexed);
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SearchQuery;
    use std::fs;
    use tempfile::TempDir;

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.to_string(),
            limit: 10,
            ..Default::default()
        }
    }

    fn setup() -> (TempDir, KnowledgeStore, Syncer) {
        (
            TempDir::new().unwrap(),
            KnowledgeStore::open_in_memory().unwrap(),
            Syncer::new().unwrap(),
        )
    }

    const AUTH: &str = "pub fn verify_password(input: &str) -> bool {\n    input.len() > 8\n}\n\npub fn sign_out() {\n}\n";

    #[test]
    fn a_repository_is_indexed_and_a_question_finds_its_code() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/auth.rs", AUTH);
        write(
            dir.path(),
            "README.md",
            "# Demo\nA tool.\n\n## Login\nUsers sign in with a password.\n",
        );

        let report = syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert_eq!(report.indexed, 2);
        let hits = store.search("p", &query("verify password")).unwrap();
        assert_eq!(hits[0].path.as_deref(), Some("src/auth.rs"));
        assert_eq!(hits[0].label.as_deref(), Some("fn verify_password"));
        assert_eq!((hits[0].start_line, hits[0].end_line), (1, 3));
        let docs = store.search("p", &query("how do users sign in")).unwrap();
        assert!(
            docs.iter()
                .any(|hit| hit.path.as_deref() == Some("README.md"))
        );
    }

    #[test]
    fn syncing_an_unchanged_repository_reads_nothing_again() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/auth.rs", AUTH);
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        let again = syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert_eq!(again.indexed, 0);
        assert_eq!(again.unchanged, 1);
    }

    #[test]
    fn a_changed_file_is_read_again_and_a_deleted_one_is_forgotten() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/auth.rs", AUTH);
        write(dir.path(), "src/old.rs", "pub fn legacy_login() {\n}\n");
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        write(
            dir.path(),
            "src/auth.rs",
            &AUTH.replace("verify_password", "check_credentials"),
        );
        fs::remove_file(dir.path().join("src/old.rs")).unwrap();
        let report = syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert_eq!((report.indexed, report.removed), (1, 1));
        assert_eq!(
            store
                .search("p", &query("check credentials"))
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .search("p", &query("verify_password"))
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .search("p", &query("legacy_login"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn apply_paths_updates_only_the_paths_it_is_given() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/auth.rs", AUTH);
        write(
            dir.path(),
            "src/other.rs",
            "pub fn untouched_function() {\n}\n",
        );
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        write(dir.path(), "src/auth.rs", "pub fn brand_new_name() {\n}\n");
        write(
            dir.path(),
            "src/other.rs",
            "pub fn changed_but_not_reported() {\n}\n",
        );
        let report = syncer
            .apply_paths(&mut store, "p", dir.path(), &["src/auth.rs".to_string()])
            .unwrap();

        assert_eq!(report.indexed, 1);
        assert_eq!(
            store.search("p", &query("brand_new_name")).unwrap().len(),
            1
        );
        assert!(
            store
                .search("p", &query("changed_but_not_reported"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .search("p", &query("untouched_function"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_file_that_no_longer_exists_is_forgotten_when_its_path_is_reported() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/auth.rs", AUTH);
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();
        fs::remove_file(dir.path().join("src/auth.rs")).unwrap();

        let report = syncer
            .apply_paths(&mut store, "p", dir.path(), &["src/auth.rs".to_string()])
            .unwrap();

        assert_eq!(report.removed, 1);
        assert!(
            store
                .search("p", &query("verify_password"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn secrets_are_kept_out_and_listed_as_held_back() {
        let (dir, mut store, mut syncer) = setup();
        write(
            dir.path(),
            ".env",
            "DATABASE_URL=postgres://u:p@h/db\nSTRIPE=sk-live-1234567890abcdefghijklmnopqrstuv\n",
        );
        write(
            dir.path(),
            "keys/server.pem",
            "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n",
        );
        write(
            dir.path(),
            "src/config.rs",
            "pub const API_TOKEN: &str = \"abcdef1234567890\";\npub fn connect() {}\n",
        );
        write(
            dir.path(),
            "src/key.rs",
            "// -----BEGIN RSA PRIVATE KEY-----\n// MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\n// -----END RSA PRIVATE KEY-----\npub fn nothing() {}\n",
        );

        let report = syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert!(report.held_back >= 3, "{report:?}");
        assert!(store.search("p", &query("postgres")).unwrap().is_empty());
        assert!(
            store
                .search("p", &query("abcdef1234567890"))
                .unwrap()
                .is_empty()
        );
        assert!(
            !store.search("p", &query("connect")).unwrap().is_empty(),
            "the rest of the file is still indexed"
        );
        let notes = store.health("p").unwrap().notes;
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == ".env" && note.contains("secret"))
        );
        assert!(notes.iter().any(|(path, _)| path == "src/key.rs"));
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == "src/config.rs" && note.contains("taken out"))
        );
    }

    #[test]
    fn blueprint_documents_are_searchable_survive_a_repository_sync_and_go_when_removed() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), "src/lib.rs", "pub fn alpha() {}");
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();
        let docs = vec![(
            "decisions".to_string(),
            "Decisions".to_string(),
            "We chose zebra-striped caching for the session store.".to_string(),
        )];
        syncer.sync_blueprint(&mut store, "p", &docs).unwrap();
        let found = store.search("p", &query("zebra caching")).unwrap();
        assert_eq!(found[0].path.as_deref(), Some("blueprint/decisions.md"));

        // A later look at the repository must not throw the design away.
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();
        assert!(
            !store
                .search("p", &query("zebra caching"))
                .unwrap()
                .is_empty()
        );

        syncer.sync_blueprint(&mut store, "p", &[]).unwrap();
        assert!(
            store
                .search("p", &query("zebra caching"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn gitignored_folders_and_binary_files_are_not_indexed() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), ".gitignore", "generated/\n");
        write(dir.path(), "generated/out.rs", "pub fn zqgenerated() {}\n");
        write(
            dir.path(),
            "node_modules/dep/index.js",
            "function zqdependency() {}\n",
        );
        fs::write(dir.path().join("data.rs"), [b'f', b'n', 0, 1, 2, 3]).unwrap();
        write(dir.path(), "src/lib.rs", "pub fn real_function() {}\n");

        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert_eq!(store.search("p", &query("real_function")).unwrap().len(), 1);
        assert!(store.search("p", &query("zqgenerated")).unwrap().is_empty());
        assert!(
            store
                .search("p", &query("zqdependency"))
                .unwrap()
                .is_empty()
        );
        let notes = store.health("p").unwrap().notes;
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == "data.rs" && note.contains("not a text file"))
        );
    }

    #[test]
    fn a_loojiignore_file_leaves_files_out() {
        let (dir, mut store, mut syncer) = setup();
        write(dir.path(), ".loojiignore", "private/\n");
        write(
            dir.path(),
            "private/plans.md",
            "# Plans\nZqprivate words.\n",
        );
        write(dir.path(), "public/plans.md", "# Plans\nZqpublic words.\n");

        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert!(store.search("p", &query("zqprivate")).unwrap().is_empty());
        assert!(!store.search("p", &query("zqpublic")).unwrap().is_empty());
    }

    #[test]
    fn a_pdfs_text_is_searchable_by_page_and_a_scanned_pdf_is_held_back() {
        let (dir, mut store, mut syncer) = setup();
        let write_bytes = |name: &str, bytes: Vec<u8>| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        write_bytes(
            "docs/policy.pdf",
            crate::pdf::fixtures::pdf(&["Introduction to returns", "Zebracorn refund window"]),
        );
        write_bytes("docs/scan.pdf", crate::pdf::fixtures::pdf(&[""]));

        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        let hits = store.search("p", &query("zebracorn")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path.as_deref(), Some("docs/policy.pdf"));
        assert_eq!(hits[0].label.as_deref(), Some("page 2"));
        let notes = store.health("p").unwrap().notes;
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == "docs/scan.pdf" && note.contains("no text layer")),
            "{notes:?}"
        );

        // Reading it again changes nothing.
        let report = syncer.sync_repo(&mut store, "p", dir.path()).unwrap();
        assert_eq!(report.indexed, 0);
    }

    #[test]
    fn files_attached_to_tasks_are_searchable_with_their_task_and_forgotten_when_removed() {
        let (dir, mut store, mut syncer) = setup();
        let attached = |stored: &str, name: &str, bytes: Vec<u8>| {
            fs::write(dir.path().join(stored), bytes).unwrap();
            AttachedFile {
                stored_name: stored.to_string(),
                file_name: name.to_string(),
                origin: "task: Fix refunds".to_string(),
                path: dir.path().join(stored),
            }
        };
        let files = vec![
            attached(
                "a.pdf",
                "Policy.pdf",
                crate::pdf::fixtures::pdf(&["Quokkaglow refund window"]),
            ),
            attached(
                "b.txt",
                "notes.txt",
                b"# Call notes\nThe wombatspring client wants invoices monthly.".to_vec(),
            ),
            attached("c.png", "shot.png", vec![1, 2, 3]),
        ];
        let first = syncer.sync_attachments(&mut store, "p", &files).unwrap();
        assert_eq!(first.indexed, 2);

        let pdf = store.search("p", &query("quokkaglow")).unwrap();
        assert_eq!(pdf[0].path.as_deref(), Some("attachment/a.pdf"));
        assert_eq!(
            pdf[0].label.as_deref(),
            Some("Policy.pdf (task: Fix refunds) · page 1")
        );
        let text = store.search("p", &query("wombatspring")).unwrap();
        assert!(
            text[0]
                .label
                .as_deref()
                .is_some_and(|label| label.starts_with("notes.txt (task: Fix refunds)")),
            "{:?}",
            text[0].label
        );

        // Nothing changed, nothing is read again. A repository sync leaves attachments alone.
        let again = syncer.sync_attachments(&mut store, "p", &files).unwrap();
        assert_eq!(again.indexed, 0);
        let repo = TempDir::new().unwrap();
        write(repo.path(), "README.md", "Nothing about it here.");
        syncer.sync_repo(&mut store, "p", repo.path()).unwrap();
        assert_eq!(store.search("p", &query("quokkaglow")).unwrap().len(), 1);

        // A removed attachment is forgotten.
        let report = syncer
            .sync_attachments(&mut store, "p", &files[1..])
            .unwrap();
        assert_eq!(report.removed, 1);
        assert!(store.search("p", &query("quokkaglow")).unwrap().is_empty());
        assert_eq!(store.search("p", &query("wombatspring")).unwrap().len(), 1);
    }

    #[test]
    fn a_picture_that_cannot_be_read_is_held_back_with_a_reason_and_forgotten_when_removed() {
        let (dir, mut store, mut syncer) = setup();
        fs::write(dir.path().join("p.png"), b"not really a picture").unwrap();
        let files = vec![AttachedFile {
            stored_name: "p.png".to_string(),
            file_name: "shot.png".to_string(),
            origin: "blueprint: architecture".to_string(),
            path: dir.path().join("p.png"),
        }];
        let report = syncer.sync_attachments(&mut store, "p", &files).unwrap();
        assert_eq!(report.held_back, 1);
        let notes = store.health("p").unwrap().notes;
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == "attachment/p.png" && note.starts_with("held back")),
            "{notes:?}"
        );
        let report = syncer.sync_attachments(&mut store, "p", &[]).unwrap();
        assert_eq!(report.removed, 1);
        assert!(store.health("p").unwrap().notes.is_empty());
    }

    #[test]
    fn a_file_that_does_not_parse_is_still_searchable_as_plain_text() {
        let (dir, mut store, mut syncer) = setup();
        write(
            dir.path(),
            "src/broken.rs",
            "pub fn half_written( {\n    unfinished_work();\n",
        );

        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();

        assert_eq!(
            store.search("p", &query("unfinished_work")).unwrap().len(),
            1
        );
        let notes = store.health("p").unwrap().notes;
        assert!(
            notes
                .iter()
                .any(|(path, note)| path == "src/broken.rs" && note.contains("plain text"))
        );
    }

    #[test]
    fn issues_are_added_changed_and_removed() {
        let (_dir, mut store, syncer) = setup();
        let issue = |id: &str, title: &str| IssueDoc {
            id: id.to_string(),
            title: title.to_string(),
            description: "Details here.".to_string(),
            status: "Todo".to_string(),
            priority: None,
            assignee: None,
            tags: Vec::new(),
        };
        let first = syncer
            .sync_issues(
                &mut store,
                "p",
                &[issue("a", "Fix upload"), issue("b", "Add export")],
            )
            .unwrap();
        assert_eq!(first.indexed, 2);
        let again = syncer
            .sync_issues(
                &mut store,
                "p",
                &[issue("a", "Fix upload"), issue("b", "Add export")],
            )
            .unwrap();
        assert_eq!((again.indexed, again.unchanged), (0, 2));

        let changed = syncer
            .sync_issues(&mut store, "p", &[issue("a", "Fix download")])
            .unwrap();
        assert_eq!((changed.indexed, changed.removed), (1, 1));
        assert!(store.search("p", &query("export")).unwrap().is_empty());
        assert_eq!(
            store.search("p", &query("download")).unwrap()[0].source,
            "a"
        );
    }

    #[test]
    fn commits_are_added_once() {
        let (_dir, mut store, syncer) = setup();
        let commit = CommitDoc {
            sha: "abc123".into(),
            author: "Maya".into(),
            date: 0,
            message: "Add retry to the uploader".into(),
            files: vec!["src/upload.rs".into()],
        };
        assert_eq!(
            syncer
                .sync_commits(&mut store, "p", std::slice::from_ref(&commit))
                .unwrap()
                .indexed,
            1
        );
        assert_eq!(
            syncer
                .sync_commits(&mut store, "p", &[commit])
                .unwrap()
                .indexed,
            0
        );
        let hits = store.search("p", &query("uploader retry")).unwrap();
        assert_eq!(hits[0].kind, SourceKind::Commit);
    }
}
