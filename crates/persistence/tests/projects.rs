use looji_domain::{
    Project, ProjectId, ProjectRepository, RepoError, Workspace, WorkspaceRepository,
    validate_project_name,
};
use looji_persistence::SqliteStore;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory store opens and migrates")
}

fn workspace(store: &SqliteStore, name: &str) -> Workspace {
    let workspace = Workspace::new(name).unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn project(store: &SqliteStore, workspace: &Workspace, name: &str) -> Result<Project, RepoError> {
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project)?;
    Ok(project)
}

#[test]
fn create_then_find_round_trips_the_description() {
    let store = store();
    let work = workspace(&store, "Work");
    let mut original = Project::new(work.id(), "Website").unwrap();
    original.set_description(Some("Marketing site")).unwrap();

    ProjectRepository::save(&store, &original).unwrap();

    assert_eq!(
        ProjectRepository::find(&store, work.id(), original.id()).unwrap(),
        Some(original)
    );
}

#[test]
fn an_empty_description_is_stored_as_none() {
    let store = store();
    let work = workspace(&store, "Work");
    let mut original = project(&store, &work, "Website").unwrap();
    original.set_description(Some("   ")).unwrap();
    ProjectRepository::save(&store, &original).unwrap();

    let loaded = ProjectRepository::find(&store, work.id(), original.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.description(), None);
}

#[test]
fn a_duplicate_name_in_the_same_workspace_is_rejected_by_the_index() {
    let store = store();
    let work = workspace(&store, "Work");
    project(&store, &work, "Website").unwrap();

    let result = project(&store, &work, "Website");

    assert!(
        matches!(result, Err(RepoError::NameTaken)),
        "got {result:?}"
    );
}

#[test]
fn a_name_differing_only_in_case_is_rejected_by_the_index() {
    let store = store();
    let work = workspace(&store, "Work");
    project(&store, &work, "Website").unwrap();

    let result = project(&store, &work, "WEBSITE");

    assert!(
        matches!(result, Err(RepoError::NameTaken)),
        "got {result:?}"
    );
}

#[test]
fn the_same_name_is_allowed_in_another_workspace() {
    let store = store();
    let work = workspace(&store, "Work");
    let home = workspace(&store, "Home");
    project(&store, &work, "Website").unwrap();

    let result = project(&store, &home, "Website");

    assert!(result.is_ok(), "got {result:?}");
}

#[test]
fn list_for_workspace_returns_only_that_workspaces_projects() {
    let store = store();
    let work = workspace(&store, "Work");
    let home = workspace(&store, "Home");
    let work_site = project(&store, &work, "Website").unwrap();
    let work_api = project(&store, &work, "API").unwrap();
    project(&store, &home, "Garden").unwrap();

    let listed = ProjectRepository::list_for_workspace(&store, work.id()).unwrap();

    let mut ids: Vec<ProjectId> = listed.iter().map(Project::id).collect();
    ids.sort_by_key(|id| id.as_uuid());
    let mut expected = vec![work_site.id(), work_api.id()];
    expected.sort_by_key(|id| id.as_uuid());
    assert_eq!(ids, expected);
}

#[test]
fn find_does_not_return_a_project_from_another_workspace() {
    let store = store();
    let work = workspace(&store, "Work");
    let home = workspace(&store, "Home");
    let website = project(&store, &work, "Website").unwrap();

    assert_eq!(
        ProjectRepository::find(&store, home.id(), website.id()).unwrap(),
        None
    );
    assert!(
        ProjectRepository::find(&store, work.id(), website.id())
            .unwrap()
            .is_some()
    );
}

#[test]
fn renaming_a_project_onto_a_sibling_name_is_rejected() {
    let store = store();
    let work = workspace(&store, "Work");
    project(&store, &work, "Website").unwrap();
    let mut api = project(&store, &work, "API").unwrap();

    api.rename("website").unwrap();
    let result = ProjectRepository::save(&store, &api);

    assert!(
        matches!(result, Err(RepoError::NameTaken)),
        "got {result:?}"
    );
}

#[test]
fn the_domain_and_the_index_agree_on_what_counts_as_a_duplicate() {
    let store = store();
    let work = workspace(&store, "Work");
    let existing = project(&store, &work, "Website").unwrap();
    let siblings = ProjectRepository::list_for_workspace(&store, work.id()).unwrap();

    // The domain rejects these before any write is attempted.
    for candidate in ["website", "  Website  ", "WEBSITE"] {
        assert!(
            validate_project_name(candidate, &siblings, None).is_err(),
            "domain should reject {candidate:?}"
        );
    }
    assert_eq!(
        validate_project_name("Website", &siblings, Some(existing.id())),
        Ok("Website".to_string())
    );
}
