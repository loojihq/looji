use looji_domain::{
    Priority, Project, ProjectFilterRepository, ProjectId, ProjectRepository, Status, TagMatch,
    TaskFilter, TeamMemberId, Workspace, WorkspaceId, WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

fn store_at(dir: &TempDir) -> (SqliteStore, std::path::PathBuf) {
    let path = dir.path().join("looji.db");
    (SqliteStore::open(&path).unwrap(), path)
}

fn workspace(store: &SqliteStore) -> Workspace {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn project(store: &SqliteStore, workspace: WorkspaceId, name: &str) -> Project {
    let project = Project::new(workspace, name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn busy_filter() -> TaskFilter {
    let mut filter = TaskFilter {
        tag_match: TagMatch::All,
        ..Default::default()
    };
    filter.toggle_status(Status::Todo);
    filter.toggle_status(Status::InProgress);
    filter.toggle_priority(None);
    filter.toggle_priority(Some(Priority::High));
    filter.toggle_assignee(None);
    filter.toggle_assignee(Some(TeamMemberId::new()));
    filter.toggle_tag("bug");
    filter.toggle_tag("ui");
    filter
}

#[test]
fn a_saved_filter_is_restored_exactly_after_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let website = project(&store, workspace(&store).id(), "website");
    let filter = busy_filter();
    store.save_filter(website.id(), &filter).unwrap();
    drop(store);

    let reopened = SqliteStore::open(&path).unwrap();

    assert!(same_selection(
        &reopened.load_filter(website.id()).unwrap(),
        &filter
    ));
}

#[test]
fn another_project_keeps_its_own_filter() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let home = workspace(&store).id();
    let website = project(&store, home, "website");
    let docs = project(&store, home, "docs");
    let mut filter = TaskFilter::default();
    filter.toggle_status(Status::Done);
    store.save_filter(website.id(), &filter).unwrap();

    assert_eq!(store.load_filter(docs.id()).unwrap(), TaskFilter::default());
    assert_eq!(store.load_filter(website.id()).unwrap(), filter);
}

#[test]
fn saving_an_empty_filter_forgets_the_saved_one() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let website = project(&store, workspace(&store).id(), "website");
    store.save_filter(website.id(), &busy_filter()).unwrap();

    store
        .save_filter(website.id(), &TaskFilter::default())
        .unwrap();

    assert_eq!(
        store.load_filter(website.id()).unwrap(),
        TaskFilter::default()
    );
}

#[test]
fn a_row_that_no_longer_parses_is_left_out_and_the_rest_loads() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let website = project(&store, workspace(&store).id(), "website");
    let mut filter = TaskFilter::default();
    filter.toggle_status(Status::Done);
    store.save_filter(website.id(), &filter).unwrap();
    let connection = open_database(&path).unwrap();
    connection
        .execute(
            "INSERT INTO project_filters (project_id, facet, value) VALUES (?1, 'status', 'bogus')",
            [website.id().as_uuid().to_string()],
        )
        .unwrap();

    assert_eq!(store.load_filter(website.id()).unwrap(), filter);
}

#[test]
fn the_database_rejects_an_unknown_facet() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let website = project(&store, workspace(&store).id(), "website");
    let connection = open_database(&path).unwrap();

    let inserted = connection.execute(
        "INSERT INTO project_filters (project_id, facet, value) VALUES (?1, 'colour', 'red')",
        [website.id().as_uuid().to_string()],
    );

    assert!(inserted.is_err());
}

fn rows_for(path: &std::path::Path, project: ProjectId) -> i64 {
    let connection = open_database(path).unwrap();
    connection
        .query_row(
            "SELECT COUNT(*) FROM project_filters WHERE project_id = ?1",
            [project.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn deleting_a_project_removes_its_filter_rows() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let website = project(&store, workspace(&store).id(), "website");
    store.save_filter(website.id(), &busy_filter()).unwrap();
    assert!(rows_for(&path, website.id()) > 0);

    ProjectRepository::delete(&store, website.id()).unwrap();

    assert_eq!(rows_for(&path, website.id()), 0);
}

#[test]
fn deleting_a_workspace_removes_its_projects_filter_rows() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let doomed = workspace(&store);
    let website = project(&store, doomed.id(), "website");
    store.save_filter(website.id(), &busy_filter()).unwrap();

    store.delete_workspace(doomed.id(), None).unwrap();

    assert_eq!(rows_for(&path, website.id()), 0);
}

/// Whether two filters select the same values. The order of values within a facet doesn't matter,
/// since the storage doesn't keep one.
fn same_selection(left: &TaskFilter, right: &TaskFilter) -> bool {
    fn same<T: PartialEq>(left: &[T], right: &[T]) -> bool {
        left.len() == right.len() && left.iter().all(|value| right.contains(value))
    }
    same(&left.statuses, &right.statuses)
        && same(&left.priorities, &right.priorities)
        && same(&left.assignees, &right.assignees)
        && same(&left.tags, &right.tags)
        && left.tag_match == right.tag_match
}
