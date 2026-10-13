//! The projects of the current workspace and which one is selected.
//!
//! Only the active workspace's projects are held here. Switching workspaces replaces the whole
//! list, so a project from one workspace can't stay selected in another.

use looji_domain::{
    Project, ProjectId, ProjectRepository, RepoResult, Task, TaskRepository, WorkspaceId,
};

pub struct ProjectList {
    items: Vec<Project>,
    active: Option<ProjectId>,
}

impl ProjectList {
    /// Loads the projects of `workspace`, selecting the first one. No projects means no selection.
    pub fn for_workspace<S: ProjectRepository>(
        store: &S,
        workspace: WorkspaceId,
    ) -> RepoResult<Self> {
        let items = store.list_for_workspace(workspace)?;
        let active = items.first().map(Project::id);
        Ok(Self { items, active })
    }

    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            active: None,
        }
    }

    pub fn items(&self) -> &[Project] {
        &self.items
    }

    pub fn active_id(&self) -> Option<ProjectId> {
        self.active
    }

    pub fn active(&self) -> Option<&Project> {
        self.find_by_id(self.active?)
    }

    pub fn find_by_id(&self, id: ProjectId) -> Option<&Project> {
        self.items.iter().find(|project| project.id() == id)
    }

    /// Selects a project from this list. Ignores ids that aren't in it.
    pub fn select(&mut self, id: ProjectId) {
        if self.items.iter().any(|project| project.id() == id) {
            self.active = Some(id);
        }
    }

    /// Replaces a project with its stored copy. Keeps the selection.
    pub fn replace(&mut self, project: Project) {
        if let Some(slot) = self
            .items
            .iter_mut()
            .find(|existing| existing.id() == project.id())
        {
            *slot = project;
        }
    }

    /// Adds a newly created project and selects it.
    pub fn add_and_select(&mut self, project: Project) {
        self.active = Some(project.id());
        self.items.push(project);
    }
}

/// The tasks of one project. The only place the board reads tasks; always filtered by project.
pub fn load_board<S: TaskRepository>(store: &S, project: ProjectId) -> RepoResult<Vec<Task>> {
    store.list_for_project(project)
}

#[cfg(test)]
mod tests {
    use looji_domain::{Position, Status, Task, TaskRepository, Workspace, WorkspaceRepository};
    use looji_persistence::SqliteStore;

    use super::*;

    fn workspace(store: &SqliteStore, name: &str) -> Workspace {
        let workspace = Workspace::new(name).unwrap();
        WorkspaceRepository::save(store, &workspace).unwrap();
        workspace
    }

    fn project(store: &SqliteStore, workspace: &Workspace, name: &str) -> Project {
        let project = Project::new(workspace.id(), name).unwrap();
        ProjectRepository::save(store, &project).unwrap();
        project
    }

    fn task(store: &SqliteStore, project: &Project, title: &str) {
        // Each task goes after the last, since keys are unique within a column.
        let last = TaskRepository::list_for_project(store, project.id())
            .unwrap()
            .last()
            .map(|task| task.position().as_str().to_string());
        let key = looji_domain::after(last.as_deref()).unwrap();
        let task = Task::new(
            project.id(),
            title,
            Status::Todo,
            Position::new(key).unwrap(),
        )
        .unwrap();
        TaskRepository::save(store, &task).unwrap();
    }

    #[test]
    fn selects_the_first_project_of_the_workspace() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = workspace(&store, "Work");
        project(&store, &work, "Website");
        project(&store, &work, "API");

        let list = ProjectList::for_workspace(&store, work.id()).unwrap();

        assert_eq!(list.active_id(), list.items().first().map(Project::id));
    }

    #[test]
    fn a_workspace_without_projects_has_no_selection() {
        let store = SqliteStore::open_in_memory().unwrap();
        let empty = workspace(&store, "Empty");

        let list = ProjectList::for_workspace(&store, empty.id()).unwrap();

        assert!(list.items().is_empty());
        assert_eq!(list.active_id(), None);
    }

    #[test]
    fn switching_workspace_never_keeps_the_previous_workspaces_project() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = workspace(&store, "Work");
        let home = workspace(&store, "Home");
        let work_site = project(&store, &work, "Website");
        let home_site = project(&store, &home, "Website");

        let in_work = ProjectList::for_workspace(&store, work.id()).unwrap();
        assert_eq!(in_work.active_id(), Some(work_site.id()));

        let in_home = ProjectList::for_workspace(&store, home.id()).unwrap();
        assert_eq!(in_home.active_id(), Some(home_site.id()));
        assert!(
            in_home
                .items()
                .iter()
                .all(|p| p.workspace_id() == home.id())
        );
        assert!(in_home.find_by_id(work_site.id()).is_none());
    }

    #[test]
    fn selecting_a_project_from_another_workspace_is_ignored() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = workspace(&store, "Work");
        let home = workspace(&store, "Home");
        let first = project(&store, &work, "Website");
        let foreign = project(&store, &home, "Garden");

        let mut list = ProjectList::for_workspace(&store, work.id()).unwrap();
        list.select(foreign.id());

        assert_eq!(list.active_id(), Some(first.id()));
    }

    #[test]
    fn each_board_shows_only_its_own_projects_tasks() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = workspace(&store, "Work");
        let website = project(&store, &work, "Website");
        let api = project(&store, &work, "API");
        task(&store, &website, "Write the landing page");
        task(&store, &api, "Design the endpoints");
        task(&store, &api, "Add auth");

        let website_board = load_board(&store, website.id()).unwrap();
        let api_board = load_board(&store, api.id()).unwrap();

        let titles = |board: &[Task]| -> Vec<String> {
            let mut titles: Vec<String> = board.iter().map(|t| t.title().to_string()).collect();
            titles.sort();
            titles
        };
        assert_eq!(titles(&website_board), vec!["Write the landing page"]);
        assert_eq!(titles(&api_board), vec!["Add auth", "Design the endpoints"]);
    }

    #[test]
    fn a_project_created_in_the_list_becomes_the_selection() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = workspace(&store, "Work");
        project(&store, &work, "Website");
        let mut list = ProjectList::for_workspace(&store, work.id()).unwrap();

        let api = project(&store, &work, "API");
        list.add_and_select(api.clone());

        assert_eq!(list.active_id(), Some(api.id()));
    }
}
