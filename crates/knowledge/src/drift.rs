//! Planned against actual. The blueprint's component diagram says what the system should be. The repository says what it
//! is. This reads the repository's own units (Cargo crates and npm packages) and how they depend on each other, and
//! compares them with the diagram: parts with no code, code no part claims, and dependencies the design never drew.
//!
//! Matching is by the file a node links to, or by name. It is a hint list, not a verdict, so it leans toward saying
//! less: nodes that stand for things outside the code (a database, a person, an outside service) are not expected to
//! have code, and a dependency counts as planned if the diagram joins the two parts in either direction.

use std::{collections::BTreeSet, path::Path};

use ignore::WalkBuilder;
use serde_json::Value;

/// A unit of code the repository's own manifests describe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    /// Where it lives, relative to the repository, with `/` separators. Empty for the repository itself.
    pub path: String,
    /// The other units of this repository it depends on, by name.
    pub deps: Vec<String>,
}

/// A part of the design, as far as the comparison needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedNode {
    pub label: String,
    pub kind: Option<String>,
    /// A file the node links to.
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Planned {
    pub nodes: Vec<PlannedNode>,
    /// Pairs of node positions the design joins.
    pub edges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Drift {
    /// Units of code the design accounts for.
    pub matched: usize,
    /// Parts of the design with no code behind them.
    pub missing: Vec<String>,
    /// Units of code no part of the design claims.
    pub unclaimed: Vec<String>,
    /// Dependencies in the code between two parts the design never joins: from, to.
    pub unplanned: Vec<(String, String)>,
}

impl Drift {
    /// Whether the code and the design agree on everything checked.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.unclaimed.is_empty() && self.unplanned.is_empty()
    }

    /// The disagreements as a reviewer reads them, or `None` when the code and the design agree. Each list is cut
    /// short, since the point is what to look out for and not a full audit.
    pub fn report(&self) -> Option<String> {
        const MAX: usize = 8;
        if self.is_clean() {
            return None;
        }
        let list = |items: &[String]| {
            let shown: Vec<&str> = items.iter().take(MAX).map(String::as_str).collect();
            let more = items.len().saturating_sub(MAX);
            if more > 0 {
                format!("{} and {more} more", shown.join(", "))
            } else {
                shown.join(", ")
            }
        };
        let mut text = String::new();
        if !self.unplanned.is_empty() {
            let pairs: Vec<String> = self
                .unplanned
                .iter()
                .map(|(from, to)| format!("{from} -> {to}"))
                .collect();
            text.push_str(&format!(
                "- Dependencies in the code that the design never drew: {}.\n",
                list(&pairs)
            ));
        }
        if !self.unclaimed.is_empty() {
            text.push_str(&format!(
                "- In the code, not in the design: {}.\n",
                list(&self.unclaimed)
            ));
        }
        if !self.missing.is_empty() {
            text.push_str(&format!(
                "- In the design, with no code behind it: {}.\n",
                list(&self.missing)
            ));
        }
        Some(text.trim_end().to_string())
    }
}

/// Node kinds that stand for something outside the code, so they are not expected to have any.
const OUTSIDE: [&str; 9] = [
    "database", "external", "client", "user", "queue", "cache", "storage", "file", "person",
];

fn normal(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The name and the dependency names a Cargo manifest declares.
fn read_cargo(text: &str) -> (Option<String>, Vec<String>) {
    let mut name = None;
    let mut deps = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            section = header.trim_end_matches(']').trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if section == "package" && key == "name" {
            name = Some(value.trim().trim_matches('"').to_string());
        } else if section == "dependencies"
            || section == "build-dependencies"
            || (section.starts_with("target.") && section.ends_with("dependencies"))
        {
            deps.push(key.split('.').next().unwrap_or(key).trim().to_string());
        }
    }
    (name, deps)
}

fn read_package_json(text: &str) -> (Option<String>, Vec<String>) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return (None, Vec::new());
    };
    let name = json.get("name").and_then(Value::as_str).map(str::to_string);
    let deps = ["dependencies", "peerDependencies"]
        .iter()
        .filter_map(|key| json.get(key).and_then(Value::as_object))
        .flat_map(|map| map.keys().cloned())
        .collect();
    (name, deps)
}

/// The units the repository's manifests describe, in path order. Folders the repository ignores are skipped.
pub fn actual_units(root: &Path) -> Vec<Unit> {
    let mut found: Vec<(String, String, Vec<String>)> = Vec::new();
    let walker = WalkBuilder::new(root).max_depth(Some(5)).build();
    for entry in walker.flatten() {
        let file = entry.file_name().to_string_lossy();
        if file != "Cargo.toml" && file != "package.json" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let (name, deps) = if file == "Cargo.toml" {
            read_cargo(&text)
        } else {
            read_package_json(&text)
        };
        // A manifest with no name is a workspace root, not a unit of code.
        let Some(name) = name else {
            continue;
        };
        let folder = entry.path().parent().unwrap_or(root);
        found.push((name, relative(root, folder), deps));
    }
    found.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    found.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let names: BTreeSet<String> = found.iter().map(|(name, ..)| normal(name)).collect();
    found
        .into_iter()
        .map(|(name, path, deps)| {
            let mut own: Vec<String> = deps
                .into_iter()
                .filter(|dep| names.contains(&normal(dep)) && normal(dep) != normal(&name))
                .collect();
            own.sort();
            own.dedup();
            Unit {
                name,
                path,
                deps: own,
            }
        })
        .collect()
}

/// Which units a node stands for: the ones its file is inside, else the ones whose name its label contains or is
/// contained in.
fn units_of(node: &PlannedNode, units: &[Unit]) -> Vec<usize> {
    if let Some(file) = &node.file {
        let file = file.replace('\\', "/");
        let inside: Vec<usize> = units
            .iter()
            .enumerate()
            .filter(|(_, unit)| {
                !unit.path.is_empty()
                    && (file == unit.path || file.starts_with(&format!("{}/", unit.path)))
            })
            .map(|(index, _)| index)
            .collect();
        if !inside.is_empty() {
            return inside;
        }
    }
    let label = normal(&node.label);
    if label.len() < 3 {
        return Vec::new();
    }
    let mut named: Vec<(usize, usize)> = units
        .iter()
        .enumerate()
        .filter(|(_, unit)| {
            let name = normal(&unit.name);
            name.len() >= 3 && (label.contains(&name) || name.contains(&label))
        })
        .map(|(index, unit)| (index, normal(&unit.name).len()))
        .collect();
    // The most specific name wins, so "looji-ui" is not mistaken for "looji".
    named.sort_by_key(|(_, length)| std::cmp::Reverse(*length));
    named
        .first()
        .map(|(index, _)| vec![*index])
        .unwrap_or_default()
}

/// Compares the design with the code.
pub fn compare(planned: &Planned, units: &[Unit], root: &Path) -> Drift {
    let node_units: Vec<Vec<usize>> = planned
        .nodes
        .iter()
        .map(|node| units_of(node, units))
        .collect();
    let mut drift = Drift::default();

    for (node, matched) in planned.nodes.iter().zip(&node_units) {
        if !matched.is_empty() {
            continue;
        }
        let outside = node.kind.as_deref().is_some_and(|kind| {
            OUTSIDE
                .iter()
                .any(|word| kind.to_lowercase().contains(word))
        });
        let file_exists = node.file.as_deref().is_some_and(|file| {
            let mut full = root.to_path_buf();
            full.extend(
                file.split(['/', '\\'])
                    .filter(|part| !part.is_empty() && *part != ".."),
            );
            full.exists()
        });
        if file_exists || (outside && node.file.is_none()) {
            continue;
        }
        drift.missing.push(node.label.clone());
    }

    let claimed: BTreeSet<usize> = node_units.iter().flatten().copied().collect();
    drift.matched = claimed.len();
    drift.unclaimed = units
        .iter()
        .enumerate()
        .filter(|(index, _)| !claimed.contains(index))
        .map(|(_, unit)| unit.name.clone())
        .collect();

    let nodes_of = |unit: usize| -> Vec<usize> {
        node_units
            .iter()
            .enumerate()
            .filter(|(_, matched)| matched.contains(&unit))
            .map(|(index, _)| index)
            .collect()
    };
    let joined = |a: usize, b: usize| {
        planned
            .edges
            .iter()
            .any(|&(from, to)| (from == a && to == b) || (from == b && to == a))
    };
    for (index, unit) in units.iter().enumerate() {
        let from = nodes_of(index);
        if from.is_empty() {
            continue;
        }
        for dep in &unit.deps {
            let Some(target) = units.iter().position(|other| &other.name == dep) else {
                continue;
            };
            let to = nodes_of(target);
            if to.is_empty() || from.iter().any(|a| to.contains(a)) {
                continue;
            }
            if !from.iter().any(|&a| to.iter().any(|&b| joined(a, b))) {
                drift.unplanned.push((unit.name.clone(), dep.clone()));
            }
        }
    }
    drift
}

#[cfg(test)]
mod report_tests {
    use super::Drift;

    #[test]
    fn agreement_has_no_report() {
        let drift = Drift {
            matched: 3,
            ..Drift::default()
        };
        assert_eq!(drift.report(), None);
    }

    #[test]
    fn each_kind_of_disagreement_is_listed_and_long_lists_are_cut() {
        let drift = Drift {
            matched: 1,
            missing: vec!["Search".into()],
            unclaimed: (0..10).map(|n| format!("tool{n}")).collect(),
            unplanned: vec![("ui".into(), "db".into())],
        };
        let report = drift.report().unwrap();
        assert!(report.contains("never drew: ui -> db."));
        assert!(report.contains("tool7 and 2 more"));
        assert!(!report.contains("tool8,"));
        assert!(report.contains("no code behind it: Search."));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn workspace() -> TempDir {
        let dir = TempDir::new().unwrap();
        let write = |path: &str, text: &str| {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        };
        write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        write(
            "crates/app/Cargo.toml",
            "[package]\nname = \"shop-app\"\n\n[dependencies]\nshop-core = { path = \"../core\" }\nserde = \"1\"\n",
        );
        write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"shop-core\"\n\n[dependencies]\nshop-db.workspace = true\n",
        );
        write("crates/db/Cargo.toml", "[package]\nname = \"shop-db\"\n");
        write(
            "crates/extras/Cargo.toml",
            "[package]\nname = \"shop-extras\"\n",
        );
        write("crates/core/src/lib.rs", "");
        dir
    }

    fn node(label: &str, kind: Option<&str>, file: Option<&str>) -> PlannedNode {
        PlannedNode {
            label: label.into(),
            kind: kind.map(str::to_string),
            file: file.map(str::to_string),
        }
    }

    #[test]
    fn units_and_their_dependencies_are_read_from_manifests_and_outside_crates_are_ignored() {
        let dir = workspace();
        let units = actual_units(dir.path());
        let names: Vec<&str> = units.iter().map(|unit| unit.name.as_str()).collect();
        assert_eq!(names, ["shop-app", "shop-core", "shop-db", "shop-extras"]);
        assert_eq!(units[0].deps, ["shop-core"]);
        assert_eq!(units[1].deps, ["shop-db"]);
        assert_eq!(units[0].path, "crates/app");
    }

    #[test]
    fn package_json_units_are_read_too() {
        let dir = TempDir::new().unwrap();
        let write = |path: &str, text: &str| {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        };
        write(
            "packages/web/package.json",
            "{\"name\": \"web\", \"dependencies\": {\"ui-kit\": \"1\", \"react\": \"18\"}}",
        );
        write("packages/ui-kit/package.json", "{\"name\": \"ui-kit\"}");
        let units = actual_units(dir.path());
        assert_eq!(units.len(), 2);
        let web = units.iter().find(|unit| unit.name == "web").unwrap();
        assert_eq!(web.deps, ["ui-kit"]);
    }

    #[test]
    fn a_part_with_no_code_and_code_with_no_part_are_both_reported() {
        let dir = workspace();
        let units = actual_units(dir.path());
        let planned = Planned {
            nodes: vec![
                node("App", None, None),
                node("Core logic", None, Some("crates/core/src/lib.rs")),
                node("Payments service", Some("service"), None),
                node("Orders database", Some("database"), None),
            ],
            edges: vec![(0, 1)],
        };
        let drift = compare(&planned, &units, dir.path());
        assert_eq!(
            drift.missing,
            ["Payments service"],
            "a database needs no code"
        );
        assert_eq!(drift.unclaimed, ["shop-db", "shop-extras"]);
        assert_eq!(drift.matched, 2);
    }

    #[test]
    fn a_dependency_the_design_never_drew_is_reported_and_a_drawn_one_is_not() {
        let dir = workspace();
        let units = actual_units(dir.path());
        let nodes = vec![
            node("App", None, None),
            node("Core", None, Some("crates/core/src/lib.rs")),
            node("Database layer", None, Some("crates/db/src/lib.rs")),
        ];
        let drawn = Planned {
            nodes: nodes.clone(),
            edges: vec![(0, 1), (1, 2)],
        };
        assert!(compare(&drawn, &units, dir.path()).unplanned.is_empty());

        let missing_link = Planned {
            nodes,
            edges: vec![(0, 1)],
        };
        let drift = compare(&missing_link, &units, dir.path());
        assert_eq!(
            drift.unplanned,
            [("shop-core".to_string(), "shop-db".to_string())]
        );
    }

    #[test]
    fn an_edge_drawn_the_other_way_still_counts_as_planned() {
        let dir = workspace();
        let units = actual_units(dir.path());
        let planned = Planned {
            nodes: vec![
                node("App", None, None),
                node("Core", None, Some("crates/core/src/lib.rs")),
            ],
            edges: vec![(1, 0)],
        };
        assert!(compare(&planned, &units, dir.path()).unplanned.is_empty());
    }

    #[test]
    fn a_node_whose_file_exists_is_not_missing_even_if_it_is_not_a_unit() {
        let dir = workspace();
        let units = actual_units(dir.path());
        let planned = Planned {
            nodes: vec![node("Entry", None, Some("crates/core/src/lib.rs"))],
            edges: vec![],
        };
        // The file is inside a unit, so the node stands for it.
        assert!(compare(&planned, &units, dir.path()).missing.is_empty());
        let stray = Planned {
            nodes: vec![node("Notes", None, Some("README.md"))],
            edges: vec![],
        };
        assert_eq!(
            compare(&stray, &units, dir.path()).missing,
            ["Notes"],
            "README.md does not exist here"
        );
    }

    #[test]
    fn a_short_label_does_not_match_by_name_and_the_most_specific_name_wins() {
        let units = vec![
            Unit {
                name: "looji".into(),
                path: "".into(),
                deps: vec![],
            },
            Unit {
                name: "looji-ui".into(),
                path: "crates/ui".into(),
                deps: vec![],
            },
        ];
        assert!(units_of(&node("UI", None, None), &units).is_empty());
        assert_eq!(
            units_of(&node("looji-ui (GPUI app)", None, None), &units),
            vec![1]
        );
    }
}
