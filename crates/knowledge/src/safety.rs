//! What never enters the knowledge base: files that aren't source, files that hold secrets, and secret text inside
//! files that are otherwise fine.
//!
//! The knowledge base feeds an AI provider, so anything indexed can end up in a request. Secrets are kept out at the
//! door: by file name, and by pattern inside the text. What can't be recognised can't be removed, so this is a
//! safeguard and not a promise. The index health view lists what was held back.

use std::path::Path;

use ignore::WalkBuilder;
use looji_domain::Language;

/// Largest file indexed, in bytes. A file this big is almost always generated or data.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// PDFs are bigger than text for what they say, so they get a larger limit.
pub const MAX_PDF_BYTES: u64 = 20 * 1024 * 1024;

/// Folders that are never read, whatever the ignore files say.
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    "vendor",
    ".idea",
    ".vscode",
];

/// Files that are generated or locked, and not worth reading.
const EXCLUDED_FILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "go.sum",
];

/// What a path is for the knowledge base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// A source file in a language the symbol parser reads.
    Code(Language),
    /// Prose: markdown and plain text.
    Doc,
    /// A PDF, read for its text layer.
    Pdf,
    /// Not indexed.
    Skip,
}

/// Whether a file name says the file holds a secret: environment files, keys, certificates, and credentials.
pub fn is_secret_path(path: &str) -> bool {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    if name == ".env" || name.starts_with(".env.") || name.ends_with(".env") {
        return !name.ends_with(".example")
            && !name.ends_with(".sample")
            && !name.ends_with(".template");
    }
    const EXTENSIONS: &[&str] = &[
        ".pem",
        ".key",
        ".pfx",
        ".p12",
        ".jks",
        ".keystore",
        ".crt",
        ".cer",
        ".kdbx",
    ];
    if EXTENSIONS.iter().any(|extension| name.ends_with(extension)) {
        return true;
    }
    name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.contains("secret")
        || name.starts_with("credentials")
        || name == ".npmrc"
        || name == ".pypirc"
        || name == ".netrc"
}

/// How a path is treated, from its name alone. Secrets, excluded folders and files, and unknown extensions are
/// skipped.
pub fn classify(path: &str) -> FileClass {
    let normalised = path.replace('\\', "/");
    if is_secret_path(&normalised) {
        return FileClass::Skip;
    }
    let mut parts = normalised.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() && EXCLUDED_DIRS.contains(&part) {
            return FileClass::Skip;
        }
    }
    let name = normalised.rsplit('/').next().unwrap_or(&normalised);
    if EXCLUDED_FILES.contains(&name) || name.ends_with(".min.js") || name.ends_with(".min.css") {
        return FileClass::Skip;
    }
    if let Some(language) = Language::from_path(&normalised) {
        return FileClass::Code(language);
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".pdf") {
        return FileClass::Pdf;
    }
    if lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".markdown")
        || lower.ends_with(".txt")
        || lower.ends_with(".rst")
    {
        return FileClass::Doc;
    }
    FileClass::Skip
}

/// Every file in the repository that the ignore rules leave in, relative to `root` with `/` between parts, in path
/// order. The rules are the repository's own `.gitignore` and a `.loojiignore` in the same format, plus the
/// built-in excluded folders. Symbolic links aren't followed.
fn all_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .add_custom_ignore_filename(".loojiignore")
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_some_and(|kind| kind.is_dir())
                && EXCLUDED_DIRS.contains(&name.as_ref()))
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        files.push(relative.to_string_lossy().replace('\\', "/"));
    }
    files.sort();
    files
}

/// The repository's files worth indexing: source files and docs, with secrets, binaries and generated files left out.
pub fn indexable_files(root: &Path) -> Vec<String> {
    all_files(root)
        .into_iter()
        .filter(|path| classify(path) != FileClass::Skip)
        .collect()
}

/// The files that were held back because their names say they hold secrets, so the health view can list them.
pub fn secret_files(root: &Path) -> Vec<String> {
    all_files(root)
        .into_iter()
        .filter(|path| is_secret_path(path))
        .collect()
}

/// Whether `bytes` look like text. A NUL byte means a binary file.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    !bytes.iter().take(8000).any(|byte| *byte == 0)
}

/// The text with secrets taken out, and whether anything was. A private key block makes the whole file a secret
/// (`None`). Lines that assign a value to a name like `password`, `token`, or `api_key`, and tokens in well-known
/// formats, have the value replaced by `[redacted]`.
pub fn redact(text: &str) -> Option<(String, bool)> {
    if has_private_key(text) {
        return None;
    }
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        match redact_line(line) {
            Some(replaced) => {
                changed = true;
                out.push_str(&replaced);
            }
            None => out.push_str(line),
        }
    }
    Some((out, changed))
}

/// A line without its comment marker, so a key pasted into a comment is still seen as one.
fn without_comment(line: &str) -> &str {
    line.trim().trim_start_matches(['/', '#', '*', ';']).trim()
}

/// Whether the text holds a private key: a `BEGIN ... PRIVATE KEY` line followed by the key's own base64 body. Source
/// code that only mentions the markers, in a string or a test, is not a key.
fn has_private_key(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    lines.iter().enumerate().any(|(index, line)| {
        let marker = without_comment(line);
        if !(marker.starts_with("-----BEGIN") && marker.contains("PRIVATE KEY-----")) {
            return false;
        }
        lines[index + 1..]
            .iter()
            .map(|next| without_comment(next))
            .find(|next| !next.is_empty())
            .is_some_and(|body| {
                body.len() >= 32
                    && body
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
            })
    })
}

/// Token prefixes that identify a credential by themselves, and the fewest characters after the prefix.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("sk-ant-", 20),
    ("sk-", 32),
    ("ghp_", 30),
    ("gho_", 30),
    ("github_pat_", 30),
    ("AKIA", 12),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("AIza", 30),
];

const SECRET_NAMES: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "access_key",
];

/// The line with its secrets replaced, or `None` when it has none.
fn redact_line(line: &str) -> Option<String> {
    let mut result = line.to_string();
    let mut changed = false;

    for (prefix, min_tail) in TOKEN_PREFIXES {
        let mut from = 0;
        while let Some(found) = result[from..].find(prefix) {
            let start = from + found;
            let tail: usize = result[start + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .map(char::len_utf8)
                .sum();
            if tail >= *min_tail {
                let end = start + prefix.len() + tail;
                result.replace_range(start..end, "[redacted]");
                changed = true;
                from = start + "[redacted]".len();
            } else {
                from = start + prefix.len();
            }
        }
    }

    // `name = "value"`, `name: value`, and `NAME=value`, where the name says it is a secret and the value is a
    // literal rather than an expression or an empty string.
    let lower = result.to_lowercase();
    if let Some(name_end) = SECRET_NAMES
        .iter()
        .filter_map(|name| lower.find(name).map(|at| at + name.len()))
        .min()
    {
        // The name has to end the identifier, so `tokenizer = "x"` is not a secret but `api_token = "x"` is.
        let after_name = result[name_end..]
            .trim_start_matches(['"', '\''])
            .trim_start();
        if let Some(value_part) = after_name
            .strip_prefix('=')
            .or_else(|| after_name.strip_prefix(':'))
        {
            let mut value = value_part.trim().trim_end_matches([',', ';']);
            // A typed declaration, `NAME: &str = "value"`, has its value after the equals sign.
            if let Some((_, assigned)) = value.rsplit_once('=') {
                value = assigned.trim();
            }
            // Only a quoted value counts. A bare word after a name is a type or an expression in code.
            let literal = value.starts_with('"') || value.starts_with('\'');
            let quoted_empty = value == "\"\"" || value == "''";
            let expression = value.contains('(')
                || value.contains("::")
                || value.starts_with("env")
                || value.starts_with("process")
                || value.starts_with("os.");
            if literal && !quoted_empty && !expression && value.len() >= 6 && value != "[redacted]"
            {
                let value_start = result.rfind(value)?;
                result.replace_range(value_start..value_start + value.len(), "\"[redacted]\"");
                changed = true;
            }
        }
    }
    changed.then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_files_are_recognised_by_name() {
        for path in [
            ".env",
            ".env.local",
            "config/prod.env",
            "keys/server.pem",
            "id_rsa",
            "ssh/id_ed25519.pub",
            "client_secret.json",
            "credentials.json",
            ".npmrc",
        ] {
            assert!(is_secret_path(path), "{path}");
        }
        for path in [
            ".env.example",
            "src/environment.rs",
            "docs/secrets-policy.md".replace("secrets", "keys").as_str(),
            "src/main.rs",
        ] {
            assert!(!is_secret_path(path), "{path}");
        }
    }

    #[test]
    fn files_are_classified_and_unwanted_ones_skipped() {
        assert_eq!(classify("src/main.rs"), FileClass::Code(Language::Rust));
        assert_eq!(classify("web/app.tsx"), FileClass::Code(Language::Tsx));
        assert_eq!(classify("README.md"), FileClass::Doc);
        assert_eq!(classify("docs/design/plan.md"), FileClass::Doc);
        assert_eq!(classify("node_modules/lib/index.js"), FileClass::Skip);
        assert_eq!(classify("target/debug/build.rs"), FileClass::Skip);
        assert_eq!(classify("Cargo.lock"), FileClass::Skip);
        assert_eq!(classify("assets/logo.png"), FileClass::Skip);
        assert_eq!(classify(".env"), FileClass::Skip);
        assert_eq!(classify("static/app.min.js"), FileClass::Skip);
    }

    #[test]
    fn a_private_key_makes_the_whole_file_a_secret() {
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\n-----END RSA PRIVATE KEY-----";
        assert_eq!(redact(key), None);
        assert_eq!(
            redact(&key.replace('\n', "\n// ")),
            None,
            "a key inside a comment is still a key"
        );
    }

    #[test]
    fn code_that_only_mentions_the_key_markers_is_not_a_key() {
        let source = "let begin = \"-----BEGIN PRIVATE KEY-----\";\nlet end = \"-----END PRIVATE KEY-----\";\n";
        assert_eq!(redact(source), Some((source.to_string(), false)));
    }

    #[test]
    fn tokens_in_well_known_formats_are_replaced() {
        let (text, changed) =
            redact("let key = \"sk-ant-api03-abcdefghijklmnopqrstuvwxyz\";\nlet ok = 1;").unwrap();
        assert!(changed);
        assert!(!text.contains("abcdefghijkl"));
        assert!(text.contains("let ok = 1;"));
        let (github, _) = redact("GH=ghp_abcdefghijklmnopqrstuvwxyz0123456789").unwrap();
        assert!(!github.contains("ghp_abc"));
    }

    #[test]
    fn a_named_secret_with_a_literal_value_is_replaced_but_code_that_reads_one_is_not() {
        let (text, changed) =
            redact("password = \"hunter2hunter2\"\napi_key: \"abcd1234efgh\"\n").unwrap();
        assert!(changed);
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("abcd1234"));
        let (code, changed) = redact("let password = read_password();\nlet token = env::var(\"TOKEN\");\nlet secret = \"\";\n").unwrap();
        assert!(!changed, "{code}");
        assert_eq!(
            code,
            "let password = read_password();\nlet token = env::var(\"TOKEN\");\nlet secret = \"\";\n"
        );
    }

    #[test]
    fn ordinary_text_is_untouched() {
        // Names that sound secret, followed by a type, are code and must not be rewritten.
        let source = "pub struct Charge {\n    pub card_token: String,\n}\nfn verify(password: &str) -> bool {\n    password.len() > 8\n}\n";
        let (text, changed) = redact(source).unwrap();
        assert!(!changed);
        assert_eq!(text, source);
    }

    #[test]
    fn binary_content_is_not_text() {
        assert!(looks_like_text(b"fn main() {}"));
        assert!(!looks_like_text(&[0x89, b'P', b'N', b'G', 0, 1, 2]));
    }
}
