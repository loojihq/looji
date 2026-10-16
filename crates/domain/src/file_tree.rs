//! The file tree of a repository: which entries are listed, how they're ordered, and how the flat list of paths
//! becomes a tree. Pure. The walk that finds the paths is in persistence.

/// Folders left out at any depth: git's own folder, build output, dependency folders, and tool caches. They're left
/// out even when the repository's ignore files don't name them, since a file tree rarely needs them.
pub const BUILT_IN_IGNORED: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    ".next",
    ".gradle",
    "__pycache__",
    ".venv",
    "venv",
    ".pytest_cache",
    ".mypy_cache",
];

/// Whether the built-in list leaves an entry out. `.git` is left out whatever its type, since a worktree or a
/// submodule has it as a file. Names are matched without regard to case, as Windows does.
pub fn is_built_in_ignored(name: &str, is_dir: bool) -> bool {
    name.eq_ignore_ascii_case(".git")
        || (is_dir
            && BUILT_IN_IGNORED
                .iter()
                .any(|ignored| ignored.eq_ignore_ascii_case(name)))
}

/// What a tree entry is. A symbolic link is listed but never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
}

/// One entry in the file tree. `path` is relative to the repository root, with `/` between parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub children: Vec<TreeNode>,
}

/// Builds the tree from relative paths. A folder that isn't listed on its own, but has listed contents, still
/// appears. Each level is ordered the same way for the same set of paths: folders first, then names compared
/// case-insensitively, then byte by byte. The order doesn't depend on the order the paths arrive in.
pub fn build_tree<'a>(entries: impl IntoIterator<Item = (&'a str, EntryKind)>) -> Vec<TreeNode> {
    let mut roots = Vec::new();
    for (path, kind) in entries {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        if !parts.is_empty() {
            insert(&mut roots, "", &parts, kind);
        }
    }
    sort_nodes(&mut roots);
    roots
}

fn insert(nodes: &mut Vec<TreeNode>, parent: &str, rest: &[&str], kind: EntryKind) {
    let Some((name, below)) = rest.split_first() else {
        return;
    };
    let path = if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    };
    let index = match nodes.iter().position(|node| node.name == *name) {
        Some(index) => index,
        None => {
            nodes.push(TreeNode {
                name: name.to_string(),
                path: path.clone(),
                kind: EntryKind::Directory,
                children: Vec::new(),
            });
            nodes.len() - 1
        }
    };
    let node = &mut nodes[index];
    if below.is_empty() {
        // A folder that has contents is a folder, whatever else is listed under its name.
        if node.children.is_empty() {
            node.kind = kind;
        }
    } else {
        node.kind = EntryKind::Directory;
        insert(&mut node.children, &path, below, kind);
    }
}

fn sort_nodes(nodes: &mut [TreeNode]) {
    nodes.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    for node in nodes {
        sort_nodes(&mut node.children);
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::file_tree::{EntryKind, TreeNode, build_tree, is_built_in_ignored};

    use EntryKind::{Directory, File};

    fn names(nodes: &[TreeNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.name.as_str()).collect()
    }

    #[test]
    fn git_is_ignored_as_a_folder_or_a_file() {
        assert!(is_built_in_ignored(".git", true));
        assert!(is_built_in_ignored(".git", false));
    }

    #[test]
    fn build_output_and_dependency_folders_are_ignored_but_not_files_with_those_names() {
        assert!(is_built_in_ignored("target", true));
        assert!(is_built_in_ignored("Node_Modules", true));
        assert!(!is_built_in_ignored("target", false));
        assert!(!is_built_in_ignored("main.rs", true));
    }

    #[test]
    fn folders_come_first_then_names_in_case_insensitive_order() {
        let tree = build_tree([
            ("README.md", File),
            ("src", Directory),
            ("alpha.txt", File),
            ("Docs", Directory),
            ("Zeta.txt", File),
        ]);
        assert_eq!(
            names(&tree),
            ["Docs", "src", "alpha.txt", "README.md", "Zeta.txt"]
        );
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_paths_arrive_in() {
        let forward = build_tree([("b.rs", File), ("a.rs", File), ("sub/c.rs", File)]);
        let backward = build_tree([("sub/c.rs", File), ("a.rs", File), ("b.rs", File)]);
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_folder_that_is_only_implied_by_its_contents_still_appears() {
        let tree = build_tree([("src/lib/mod.rs", File)]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "src");
        assert_eq!(tree[0].kind, Directory);
        assert_eq!(tree[0].children[0].path, "src/lib");
        assert_eq!(tree[0].children[0].children[0].path, "src/lib/mod.rs");
    }

    #[test]
    fn a_listed_file_is_not_a_folder_and_a_folder_with_contents_stays_one() {
        let tree = build_tree([("notes", File), ("docs", Directory), ("docs/a.md", File)]);
        let notes = tree.iter().find(|node| node.name == "notes").unwrap();
        let docs = tree.iter().find(|node| node.name == "docs").unwrap();
        assert_eq!(notes.kind, File);
        assert_eq!(docs.kind, Directory);
        assert_eq!(names(&docs.children), ["a.md"]);
    }
}
