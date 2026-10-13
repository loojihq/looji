//! The workspace list and the active selection.
//!
//! This is the single source of truth: the sidebar and the header both render
//! from it. The view changes it only after the matching storage write succeeds.

use looji_domain::{RepoResult, SettingsRepository, Workspace, WorkspaceId, WorkspaceRepository};

const FIRST_WORKSPACE_NAME: &str = "Personal";

pub struct WorkspaceList {
    items: Vec<Workspace>,
    active: WorkspaceId,
}

impl WorkspaceList {
    /// Loads the workspaces and resolves the active one.
    ///
    /// On first run this creates a default workspace. If the stored active id is
    /// missing or stale, the first workspace becomes active and is written back.
    pub fn load<S>(store: &S) -> RepoResult<Self>
    where
        S: WorkspaceRepository + SettingsRepository,
    {
        let mut items = store.list()?;
        if items.is_empty() {
            let first = Workspace::new(FIRST_WORKSPACE_NAME).expect("the default name is valid");
            store.save(&first)?;
            items.push(first);
        }

        let stored = store.active_workspace()?;
        let active = resolve_active(stored, &items).expect("the list has at least one workspace");
        if stored != Some(active) {
            store.set_active_workspace(active)?;
        }
        Ok(Self { items, active })
    }

    pub fn items(&self) -> &[Workspace] {
        &self.items
    }

    /// The id of the workspace the user is working in. Features that need "the current
    /// workspace" should call this rather than keep their own copy.
    pub fn active_workspace_id(&self) -> WorkspaceId {
        self.active
    }

    pub fn active(&self) -> &Workspace {
        self.find(self.active)
            .expect("the active id always refers to a loaded workspace")
    }

    pub fn find(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.items.iter().find(|workspace| workspace.id() == id)
    }

    pub(super) fn push(&mut self, workspace: Workspace) {
        self.items.push(workspace);
    }

    /// Replaces the workspace with the same id. Does nothing if there is none.
    pub(super) fn replace(&mut self, workspace: Workspace) {
        if let Some(slot) = self
            .items
            .iter_mut()
            .find(|existing| existing.id() == workspace.id())
        {
            *slot = workspace;
        }
    }

    /// Only call with an id that is in the list. Storage is written by the caller first.
    pub(super) fn set_active(&mut self, id: WorkspaceId) {
        self.active = id;
    }
}

/// Picks the stored workspace if it still exists, otherwise the first one.
/// Returns `None` only when there are no workspaces.
pub fn resolve_active(stored: Option<WorkspaceId>, items: &[Workspace]) -> Option<WorkspaceId> {
    stored
        .filter(|id| items.iter().any(|workspace| workspace.id() == *id))
        .or_else(|| items.first().map(Workspace::id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> Workspace {
        Workspace::new(name).unwrap()
    }

    #[test]
    fn keeps_a_stored_workspace_that_still_exists() {
        let items = [workspace("Personal"), workspace("Work")];
        let work = items[1].id();
        assert_eq!(resolve_active(Some(work), &items), Some(work));
    }

    #[test]
    fn falls_back_to_the_first_when_nothing_is_stored() {
        let items = [workspace("Personal"), workspace("Work")];
        assert_eq!(resolve_active(None, &items), Some(items[0].id()));
    }

    #[test]
    fn falls_back_to_the_first_when_the_stored_id_is_stale() {
        let items = [workspace("Personal")];
        let deleted = WorkspaceId::new();
        assert_eq!(resolve_active(Some(deleted), &items), Some(items[0].id()));
    }

    #[test]
    fn has_no_active_workspace_when_there_are_none() {
        assert_eq!(resolve_active(Some(WorkspaceId::new()), &[]), None);
    }
}

#[cfg(test)]
mod store_tests {
    use looji_domain::{SettingsRepository, WorkspaceRepository};
    use looji_persistence::SqliteStore;

    use super::*;

    #[test]
    fn first_run_creates_a_default_workspace_and_makes_it_active() {
        let store = SqliteStore::open_in_memory().unwrap();

        let list = WorkspaceList::load(&store).unwrap();

        assert_eq!(list.items().len(), 1);
        assert_eq!(list.active().name(), FIRST_WORKSPACE_NAME);
        assert_eq!(
            store.active_workspace().unwrap(),
            Some(list.active_workspace_id())
        );
    }

    #[test]
    fn a_stale_stored_id_falls_back_to_the_first_and_is_written_back() {
        let store = SqliteStore::open_in_memory().unwrap();
        let first = Workspace::new("Personal").unwrap();
        store.save(&first).unwrap();
        store.set_active_workspace(WorkspaceId::new()).unwrap();

        let list = WorkspaceList::load(&store).unwrap();

        assert_eq!(list.active_workspace_id(), first.id());
        assert_eq!(store.active_workspace().unwrap(), Some(first.id()));
    }

    #[test]
    fn a_valid_stored_selection_survives_a_reload() {
        let store = SqliteStore::open_in_memory().unwrap();
        let first = Workspace::new("Personal").unwrap();
        let second = Workspace::new("Work").unwrap();
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        store.set_active_workspace(second.id()).unwrap();

        let list = WorkspaceList::load(&store).unwrap();

        assert_eq!(list.active_workspace_id(), second.id());
    }
}
