use looji_domain::{RepoError, SettingsRepository, Workspace, WorkspaceId, WorkspaceRepository};
use looji_persistence::SqliteStore;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory store opens and migrates")
}

#[test]
fn create_then_list_returns_the_workspace() {
    let store = store();
    let workspace = Workspace::new("Personal").unwrap();

    WorkspaceRepository::save(&store, &workspace).unwrap();

    assert_eq!(WorkspaceRepository::list(&store).unwrap(), vec![workspace]);
}

#[test]
fn rename_is_saved_and_loads_back() {
    let store = store();
    let mut workspace = Workspace::new("Personal").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();

    workspace.rename("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();

    let loaded = WorkspaceRepository::find(&store, workspace.id()).unwrap();
    assert_eq!(loaded.unwrap().name(), "Work");
}

#[test]
fn saving_a_name_that_differs_only_in_case_is_rejected_by_the_index() {
    let store = store();
    WorkspaceRepository::save(&store, &Workspace::new("Work").unwrap()).unwrap();

    let result = WorkspaceRepository::save(&store, &Workspace::new("WORK").unwrap());

    assert!(
        matches!(result, Err(RepoError::NameTaken)),
        "got {result:?}"
    );
}

#[test]
fn renaming_onto_another_workspaces_name_is_rejected_and_keeps_the_old_name() {
    let store = store();
    WorkspaceRepository::save(&store, &Workspace::new("Work").unwrap()).unwrap();
    let mut personal = Workspace::new("Personal").unwrap();
    WorkspaceRepository::save(&store, &personal).unwrap();

    personal.rename("work").unwrap();
    let result = WorkspaceRepository::save(&store, &personal);

    assert!(
        matches!(result, Err(RepoError::NameTaken)),
        "got {result:?}"
    );
    let stored = WorkspaceRepository::find(&store, personal.id()).unwrap();
    assert_eq!(stored.unwrap().name(), "Personal");
}

#[test]
fn saving_a_workspace_under_its_own_name_is_not_a_conflict() {
    let store = store();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();

    assert!(WorkspaceRepository::save(&store, &workspace).is_ok());
}

#[test]
fn active_workspace_is_unset_until_chosen() {
    let store = store();
    assert_eq!(SettingsRepository::active_workspace(&store).unwrap(), None);
}

#[test]
fn active_workspace_round_trips_and_can_change() {
    let store = store();
    let first = WorkspaceId::new();
    let second = WorkspaceId::new();

    SettingsRepository::set_active_workspace(&store, first).unwrap();
    assert_eq!(
        SettingsRepository::active_workspace(&store).unwrap(),
        Some(first)
    );

    SettingsRepository::set_active_workspace(&store, second).unwrap();
    assert_eq!(
        SettingsRepository::active_workspace(&store).unwrap(),
        Some(second)
    );
}
