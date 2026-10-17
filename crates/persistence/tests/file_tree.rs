//! The file tree of a repository on disk: what the ignore rules leave out, the order it comes back in, and symbolic
//! links, which are listed but never followed.

use std::{fs, path::Path};

use looji_domain::{EntryKind, TreeNode};
use looji_persistence::file_tree::{ScanError, ScannedTree, scan_repo_tree};
use tempfile::TempDir;

/// A small repository. The ignore files and the built-in list each leave something out, so every rule has an entry
/// that must not appear.
fn sample_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, ".gitignore", "*.log\ngenerated/\n");
    write(root, "README.md", "# sample\n");
    write(root, "Cargo.toml", "[package]\n");
    write(root, "debug.log", "noise\n");
    write(root, "generated/out.rs", "// generated\n");
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/util.rs", "pub fn util() {}\n");
    write(root, "src/build.rs", "// a file, not the build folder\n");
    write(root, "src/.gitignore", "local.txt\n");
    write(root, "src/local.txt", "ignored by src/.gitignore\n");
    write(root, "docs/guide.md", "# guide\n");
    write(root, "target/debug/app", "binary\n");
    write(root, "node_modules/pkg/index.js", "module\n");
    write(root, "build/output.o", "object\n");
    write(root, ".git/HEAD", "ref: refs/heads/master\n");
    dir
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Every path in the tree, depth first, folders before their contents.
fn all_paths(nodes: &[TreeNode]) -> Vec<String> {
    let mut paths = Vec::new();
    for node in nodes {
        paths.push(node.path.clone());
        paths.extend(all_paths(&node.children));
    }
    paths
}

fn scan(root: &Path) -> ScannedTree {
    scan_repo_tree(root).unwrap()
}

fn find<'a>(nodes: &'a [TreeNode], path: &str) -> Option<&'a TreeNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if let Some(found) = find(&node.children, path) {
            return Some(found);
        }
    }
    None
}

#[test]
fn ignored_paths_are_left_out_and_the_rest_is_listed() {
    let repo = sample_repo();
    let tree = scan(repo.path());
    let paths = all_paths(&tree.roots);

    for kept in [
        "README.md",
        "Cargo.toml",
        ".gitignore",
        "src/main.rs",
        "src/util.rs",
        "src/build.rs",
        "docs/guide.md",
    ] {
        assert!(
            paths.iter().any(|path| path == kept),
            "missing {kept}: {paths:?}"
        );
    }
    for left_out in [
        "debug.log",
        "generated",
        "generated/out.rs",
        "src/local.txt",
        "target",
        "node_modules",
        "build",
        "build/output.o",
        ".git",
    ] {
        assert!(
            !paths.iter().any(|path| path == left_out),
            "{left_out} should be ignored: {paths:?}"
        );
    }
    assert!(tree.skipped.is_empty(), "{:?}", tree.skipped);
}

#[test]
fn the_tree_comes_back_folders_first_in_a_fixed_order() {
    let repo = sample_repo();
    let tree = scan(repo.path());

    let top: Vec<&str> = tree.roots.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(
        top,
        ["docs", "src", ".gitignore", "Cargo.toml", "README.md"]
    );
    let src: Vec<&str> = find(&tree.roots, "src")
        .unwrap()
        .children
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(src, [".gitignore", "build.rs", "main.rs", "util.rs"]);
}

#[test]
fn scanning_twice_gives_the_same_tree() {
    let repo = sample_repo();
    assert_eq!(scan(repo.path()).roots, scan(repo.path()).roots);
}

#[test]
fn the_order_does_not_depend_on_the_order_files_were_created_in() {
    let forward = repo_with_files(&[
        "Cargo.toml",
        "README.md",
        "src/main.rs",
        "src/util.rs",
        "docs/guide.md",
    ]);
    let reverse = repo_with_files(&[
        "docs/guide.md",
        "src/util.rs",
        "src/main.rs",
        "README.md",
        "Cargo.toml",
    ]);
    assert_eq!(scan(forward.path()).roots, scan(reverse.path()).roots);
    assert_eq!(
        all_paths(&scan(reverse.path()).roots),
        [
            "docs",
            "docs/guide.md",
            "src",
            "src/main.rs",
            "src/util.rs",
            "Cargo.toml",
            "README.md"
        ]
    );
}

fn repo_with_files(files: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for relative in files {
        write(dir.path(), relative, "x\n");
    }
    dir
}

#[test]
fn a_symbolic_link_loop_is_listed_once_and_not_followed() {
    let repo = sample_repo();
    let root = repo.path();
    // A link inside src that points back at the repository root, and a link to a sibling folder.
    make_dir_symlink(root, "src/loop");
    make_dir_symlink(root, "docs/src-link");

    let tree = scan(root);
    let looped = find(&tree.roots, "src/loop").expect("the link is listed");
    assert_eq!(looped.kind, EntryKind::Symlink);
    assert!(looped.children.is_empty(), "a link must not be expanded");
    let linked = find(&tree.roots, "docs/src-link").expect("the second link is listed");
    assert_eq!(linked.kind, EntryKind::Symlink);
    assert!(linked.children.is_empty());
    assert_eq!(
        all_paths(&tree.roots)
            .iter()
            .filter(|path| path.ends_with("main.rs"))
            .count(),
        1,
        "the linked folder must not be walked through the link"
    );
}

#[test]
fn a_folder_that_cannot_be_read_is_an_error_not_an_empty_tree() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("no-such-repo");
    assert!(matches!(
        scan_repo_tree(&missing),
        Err(ScanError::Unreadable(_))
    ));
}

#[cfg(unix)]
fn make_dir_symlink(root: &Path, relative: &str) {
    let target = if relative.starts_with("docs") {
        root.join("src")
    } else {
        root.to_path_buf()
    };
    std::os::unix::fs::symlink(target, root.join(relative)).unwrap();
}

#[cfg(windows)]
fn make_dir_symlink(root: &Path, relative: &str) {
    let target = if relative.starts_with("docs") {
        root.join("src")
    } else {
        root.to_path_buf()
    };
    std::os::windows::fs::symlink_dir(target, root.join(relative))
        .expect("creating a symbolic link needs Developer Mode or an elevated shell");
}
