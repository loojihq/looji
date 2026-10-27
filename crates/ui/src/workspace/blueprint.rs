//! The blueprint: the documents and diagrams that define a project. It opens in its own window from the project
//! header. Generating one runs an agent that researches the repository, and the result replaces the stored blueprint
//! as the next version. Diagrams are drawn here from the diagram crate's layout, so they follow the theme, and each
//! can be copied as Mermaid or SVG.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{prelude::*, *};
use looji_ai::{
    CancellationToken,
    blueprint::{
        OutlineItem, build_blueprint_request, build_outline_request, build_revision_request,
        parse_blueprint, parse_outline,
    },
    generation::{Update, run_with},
};
use looji_diagram::{
    Graph, GraphKind, Layout, Link, Wireframe, is_wireframe, layout, parse_graph, parse_wireframe,
    to_mermaid, to_png, to_svg,
};
use looji_domain::{
    BlueprintDoc, BlueprintMedia, BlueprintRepository, CodeSourceRepository, MediaKind, Timestamp,
};
use looji_knowledge::drift::{Drift, Planned, PlannedNode, actual_units, compare};
use looji_persistence::attachments::{import_attachment, remove_attachment};

use super::{WorkspaceView, markdown_view, store, task_dialog::button};
use crate::{database::AppDatabase, text_edit::TextEdit, theme::Theme};

/// How often a running generation is checked for new steps.
const TICK: Duration = Duration::from_millis(120);

/// A diagram, checked and laid out.
pub(super) struct DiagramView {
    graph: Graph,
    layout: Layout,
}

/// A document with its diagrams ready to draw.
pub(super) struct DocView {
    doc: BlueprintDoc,
    diagrams: Vec<DiagramView>,
    wireframes: Vec<Wireframe>,
}

/// The wireframes a document's stored diagrams hold.
fn wireframes_of(diagrams: &str) -> Vec<Wireframe> {
    serde_json::from_str::<Vec<serde_json::Value>>(diagrams)
        .unwrap_or_default()
        .iter()
        .filter(|json| is_wireframe(json))
        .filter_map(|json| parse_wireframe(json).ok())
        .collect()
}

fn doc_view(doc: BlueprintDoc) -> DocView {
    let diagrams = serde_json::from_str::<Vec<serde_json::Value>>(&doc.diagrams)
        .unwrap_or_default()
        .iter()
        .filter_map(|json| parse_graph(json).ok())
        .map(|graph| DiagramView {
            layout: layout(&graph),
            graph,
        })
        .collect();
    let wireframes = wireframes_of(&doc.diagrams);
    DocView {
        doc,
        diagrams,
        wireframes,
    }
}

/// A generation in progress.
struct Run {
    /// Planning the outline, not writing yet.
    outlining: bool,
    /// The document being changed, when this is a revision and not a new blueprint.
    revising: Option<usize>,
    steps: Vec<String>,
    queue: Arc<Mutex<Vec<Update>>>,
    cancel: CancellationToken,
    _reader: Task<()>,
}

pub(super) struct BlueprintState {
    docs: Vec<DocView>,
    selected: usize,
    idea: TextEdit,
    /// What to change in the open document.
    revise: TextEdit,
    /// Whether typing goes to the revision box. Otherwise it goes to the idea box.
    revise_focused: bool,
    running: Option<Run>,
    notice: Option<String>,
    error: Option<String>,
    /// The diagram whose copy button last said "Copied": the document, the diagram, and which format.
    copied: Option<(usize, usize, &'static str)>,
    scroll: ScrollHandle,
    /// The design compared with the code, when the project has a repository with units of code.
    drift: Option<Drift>,
    /// Images and files kept with the sections, which stay across versions.
    media: Vec<BlueprintMedia>,
    /// The outline waiting for the user's say, each document with whether it stays.
    outline: Option<Vec<(OutlineItem, bool)>>,
    /// Every version the blueprint has had, newest first.
    versions: Vec<u32>,
    /// An older version being looked at, instead of the current one.
    viewing: Option<u32>,
}

impl WorkspaceView {
    pub(super) fn open_blueprint(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let id = project.id();
        let description = project.description().unwrap_or_default().to_string();
        let docs = store(cx)
            .blueprint(id)
            .unwrap_or_default()
            .into_iter()
            .map(doc_view)
            .collect();
        self.blueprint = Some(BlueprintState {
            docs,
            selected: 0,
            idea: TextEdit::new(&description),
            revise: TextEdit::default(),
            revise_focused: false,
            running: None,
            notice: None,
            error: None,
            copied: None,
            scroll: ScrollHandle::new(),
            drift: None,
            media: store(cx).blueprint_media(id).unwrap_or_default(),
            outline: None,
            versions: Vec::new(),
            viewing: None,
        });
        self.load_versions(cx);
        self.index_blueprint(cx);
        self.refresh_drift(cx);
        let main = cx.entity();
        cx.defer(move |cx| {
            super::dialog_window::open(cx, main, super::dialog_window::Kind::Blueprint)
        });
        cx.notify();
    }

    /// Drops the blueprint's state without a context, for when its window has already closed.
    pub(super) fn close_blueprint_state(&mut self) {
        if let Some(Run { cancel, .. }) = self.blueprint.as_ref().and_then(|b| b.running.as_ref()) {
            cancel.cancel();
        }
        self.blueprint = None;
    }

    pub(super) fn close_blueprint(&mut self, cx: &mut Context<Self>) {
        if let Some(Run { cancel, .. }) = self.blueprint.as_ref().and_then(|b| b.running.as_ref()) {
            cancel.cancel();
        }
        self.blueprint = None;
        cx.notify();
    }

    /// Starts writing a blueprint from the idea in the box.
    /// Plans the outline first, for the user to adjust before anything is written.
    pub(super) fn generate_blueprint(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.blueprint.as_mut() {
            state.outline = None;
        }
        self.start_run(None, true, cx);
    }

    /// Writes the blueprint from the outline the user agreed.
    pub(super) fn write_blueprint(&mut self, cx: &mut Context<Self>) {
        self.start_run(None, false, cx);
    }

    /// Drops the outline, to start again.
    fn discard_outline(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.blueprint.as_mut() {
            state.outline = None;
        }
        cx.notify();
    }

    /// Keeps a document in the outline, or leaves it out.
    fn toggle_outline_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .blueprint
            .as_mut()
            .and_then(|state| state.outline.as_mut())
            .and_then(|outline| outline.get_mut(index))
        {
            entry.1 = !entry.1;
        }
        cx.notify();
    }

    /// Asks for a change to the open document, written as a new version of the blueprint.
    pub(super) fn revise_document(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.blueprint.as_ref() else {
            return;
        };
        if state.viewing.is_some() || state.revise.trim().is_empty() {
            return;
        }
        let index = state.selected;
        self.start_run(Some(index), false, cx);
    }

    /// Starts an agent: on the whole blueprint, or on one document of it.
    fn start_run(&mut self, revising: Option<usize>, outlining: bool, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let (id, name, description) = (
            project.id(),
            project.name().to_string(),
            project.description().unwrap_or_default().to_string(),
        );
        let Some(state) = self.blueprint.as_mut() else {
            return;
        };
        if state.running.is_some() {
            return;
        }
        let idea = state.idea.trim().to_string();
        let change = state.revise.trim().to_string();
        let agreed: Vec<OutlineItem> = state
            .outline
            .iter()
            .flatten()
            .filter(|(_, kept)| *kept)
            .map(|(item, _)| item.clone())
            .collect();
        let document = revising
            .and_then(|index| state.docs.get(index))
            .map(|view| {
                (
                    view.doc.kind.clone(),
                    view.doc.title.clone(),
                    view.doc.body.clone(),
                    view.doc.diagrams.clone(),
                )
            });
        if revising.is_some() && document.is_none() {
            return;
        }
        state.error = None;
        state.notice = None;
        let root = store(cx).load_path(id).ok().flatten();
        let model = super::ai_settings::model_for(cx, Some(id));
        let provider = super::ai_settings::provider_for(cx, Some(id));
        let knowledge_path = cx.global::<AppDatabase>().knowledge_path.clone();
        let request = match &document {
            _ if outlining => {
                build_outline_request(&model, &name, &description, &idea, root.is_some())
            }
            Some((kind, title, body, diagrams)) => build_revision_request(
                &model,
                &name,
                (kind, title, body, diagrams),
                &change,
                root.is_some(),
            ),
            None => {
                build_blueprint_request(&model, &name, &description, &idea, &agreed, root.is_some())
            }
        };

        let cancel = CancellationToken::new();
        let queue: Arc<Mutex<Vec<Update>>> = Arc::new(Mutex::new(Vec::new()));
        let reader = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_going = this
                    .update(cx, |view, cx| view.read_blueprint_run(cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });
        if let Some(state) = self.blueprint.as_mut() {
            state.running = Some(Run {
                outlining,
                revising,
                steps: Vec::new(),
                queue: Arc::clone(&queue),
                cancel: cancel.clone(),
                _reader: reader,
            });
        }
        cx.notify();

        let future = {
            let queue = Arc::clone(&queue);
            async move {
                let push = move |update: Update| {
                    if let Ok(mut queue) = queue.lock() {
                        queue.push(update);
                    }
                };
                // With a repository the agent researches it. Without one the model writes from the description.
                let provider = match root {
                    Some(root) => super::agent::agent_for(
                        provider,
                        &knowledge_path,
                        id,
                        Path::new(&root),
                        true,
                    ),
                    None => looji_ai::connect(provider),
                };
                match provider {
                    Ok(provider) => run_with(provider.as_ref(), request, cancel, push).await,
                    Err(err) => push(Update::Failed(err)),
                }
            }
        };
        drop(looji_ai::runtime().spawn(future));
    }

    /// Takes what the generation has reported. Returns whether to keep reading.
    fn read_blueprint_run(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.blueprint.as_mut() else {
            return false;
        };
        let Some(run) = state.running.as_mut() else {
            return false;
        };
        let revising = run.revising;
        let outlining = run.outlining;
        let updates = match run.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        let mut finished = None;
        for update in updates {
            match update {
                Update::Step(step) => run.steps.push(step),
                Update::Finished(text) => finished = Some(Ok(text)),
                Update::Failed(err) => finished = Some(Err(err.to_string())),
                Update::Text(_) | Update::Source(_) => {}
            }
        }
        let Some(reply) = finished else {
            cx.notify();
            return true;
        };
        state.running = None;
        if outlining {
            match reply.and_then(|text| parse_outline(&text)) {
                Ok(items) => {
                    state.outline = Some(items.into_iter().map(|item| (item, true)).collect());
                }
                Err(message) => state.error = Some(message),
            }
            cx.notify();
            return false;
        }
        let outcome = reply.and_then(|text| parse_blueprint(&text));
        let mut refresh = false;
        match outcome {
            Ok(parsed) => {
                let made = |doc: &looji_ai::blueprint::ParsedDoc, kind: &str| {
                    BlueprintDoc::new(
                        kind,
                        &doc.title,
                        &doc.body,
                        &serde_json::Value::Array(doc.diagrams.clone()).to_string(),
                    )
                };
                let docs: Vec<BlueprintDoc> = match (revising, parsed.docs.first()) {
                    // A revision replaces the one document and keeps the rest as they are.
                    (Some(index), Some(changed)) => {
                        let state = self.blueprint.as_ref().expect("the blueprint is open");
                        state
                            .docs
                            .iter()
                            .enumerate()
                            .map(|(position, view)| {
                                if position == index {
                                    made(changed, &view.doc.kind)
                                } else {
                                    view.doc.clone()
                                }
                            })
                            .collect()
                    }
                    _ => parsed.docs.iter().map(|doc| made(doc, &doc.kind)).collect(),
                };
                let project = self.projects.active_id();
                let saved = project
                    .map(|project| store(cx).replace_blueprint(project, &docs, Timestamp::now()));
                let state = self.blueprint.as_mut().expect("the blueprint is open");
                match saved {
                    Some(Ok(version)) => {
                        let project = project.expect("saved for a project");
                        state.docs = store(cx)
                            .blueprint(project)
                            .unwrap_or_default()
                            .into_iter()
                            .map(doc_view)
                            .collect();
                        state.outline = None;
                        state.selected = revising.unwrap_or(0);
                        state.revise = TextEdit::default();
                        refresh = true;
                        state.notice = Some(if parsed.diagrams_dropped > 0 {
                            format!(
                                "Version {version} written. {} diagram(s) were left out because they could not be drawn.",
                                parsed.diagrams_dropped
                            )
                        } else {
                            format!("Version {version} written.")
                        });
                    }
                    Some(Err(err)) => {
                        state.error = Some(format!("Could not save the blueprint: {err}"))
                    }
                    None => {}
                }
            }
            Err(message) => state.error = Some(message),
        }
        if refresh {
            self.load_versions(cx);
            self.index_blueprint(cx);
            self.refresh_drift(cx);
        }
        cx.notify();
        false
    }

    fn load_versions(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let versions = store(cx).blueprint_versions(project).unwrap_or_default();
        if let Some(state) = self.blueprint.as_mut() {
            state.versions = versions;
            state.viewing = None;
        }
    }

    /// Shows one version of the blueprint. The newest is the current one, and can be changed. Older ones are for
    /// reading, and can be restored.
    fn view_version(&mut self, version: u32, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let docs: Vec<DocView> = store(cx)
            .blueprint_version(project, version)
            .unwrap_or_default()
            .into_iter()
            .map(doc_view)
            .collect();
        let Some(state) = self.blueprint.as_mut() else {
            return;
        };
        if docs.is_empty() {
            return;
        }
        let current = state.versions.first().copied() == Some(version);
        state.docs = docs;
        state.selected = 0;
        state.viewing = (!current).then_some(version);
        cx.notify();
    }

    /// Makes the version being looked at the current one again, as a new version.
    fn restore_version(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(state)) = (self.projects.active_id(), self.blueprint.as_ref())
        else {
            return;
        };
        if state.viewing.is_none() {
            return;
        }
        let docs: Vec<BlueprintDoc> = state.docs.iter().map(|view| view.doc.clone()).collect();
        match store(cx).replace_blueprint(project, &docs, Timestamp::now()) {
            Ok(version) => {
                let restored = store(cx)
                    .blueprint(project)
                    .unwrap_or_default()
                    .into_iter()
                    .map(doc_view)
                    .collect();
                if let Some(state) = self.blueprint.as_mut() {
                    state.docs = restored;
                    state.selected = 0;
                    state.notice = Some(format!("Restored as version {version}."));
                }
                self.load_versions(cx);
                self.index_blueprint(cx);
                self.refresh_drift(cx);
            }
            Err(err) => {
                if let Some(state) = self.blueprint.as_mut() {
                    state.error = Some(format!("Could not restore it: {err}"));
                }
            }
        }
        cx.notify();
    }

    /// Puts the blueprint's documents into the project's knowledge, so searches, chat and reviews can find the design.
    fn index_blueprint(&self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let docs: Vec<(String, String, String)> = store(cx)
            .blueprint(project)
            .unwrap_or_default()
            .into_iter()
            .map(|doc| {
                // A wireframe has no words of its own in the body, so its outline is added for search to find.
                let mut body = doc.body;
                for wireframe in wireframes_of(&doc.diagrams) {
                    body.push_str("\n\n");
                    body.push_str(&wireframe.outline());
                }
                (doc.kind, doc.title, body)
            })
            .collect();
        let path = cx.global::<AppDatabase>().knowledge_path.clone();
        let key = project.as_uuid().to_string();
        let outcome = looji_knowledge::KnowledgeStore::open(&path).and_then(|mut knowledge| {
            looji_knowledge::Syncer::new()?.sync_blueprint(&mut knowledge, &key, &docs)
        });
        if let Err(err) = outcome {
            eprintln!("looji: the blueprint could not be indexed: {err}");
        }
    }

    /// Asks for files, then copies each into the attachments folder and keeps it with the section.
    fn add_section_media(&mut self, doc_kind: String, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add to this section".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let dir = cx.update(|cx| cx.global::<AppDatabase>().attachments_dir.clone());
            for source in paths {
                let dir = dir.clone();
                let imported = cx
                    .background_executor()
                    .spawn(async move { import_attachment(&source, &dir) })
                    .await;
                let doc_kind = doc_kind.clone();
                let _ = this.update(cx, |view, cx| {
                    let Some(state) = view.blueprint.as_mut() else {
                        return;
                    };
                    match imported {
                        Ok(file) => {
                            let media = BlueprintMedia {
                                doc_kind,
                                kind: MediaKind::from_file_name(&file.file_name),
                                stored_name: file.stored_name.clone(),
                                file_name: file.file_name,
                                created_at: Timestamp::now(),
                            };
                            match store(cx).add_blueprint_media(project, &media) {
                                Ok(()) => state.media.push(media),
                                Err(err) => {
                                    let _ = remove_attachment(
                                        &cx.global::<AppDatabase>().attachments_dir,
                                        &file.stored_name,
                                    );
                                    state.error = Some(format!("Could not add that file: {err}"));
                                }
                            }
                        }
                        Err(err) => state.error = Some(format!("Could not add that file: {err}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Removes a file from a section and deletes its copy.
    fn remove_section_media(
        &mut self,
        doc_kind: String,
        stored_name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        match store(cx).remove_blueprint_media(project, &doc_kind, &stored_name) {
            Ok(()) => {
                let dir = cx.global::<AppDatabase>().attachments_dir.clone();
                if let Err(err) = remove_attachment(&dir, &stored_name) {
                    eprintln!("could not delete the file of removed media: {err}");
                }
                if let Some(state) = self.blueprint.as_mut() {
                    state.media.retain(|item| {
                        !(item.doc_kind == doc_kind && item.stored_name == stored_name)
                    });
                }
            }
            Err(err) => {
                if let Some(state) = self.blueprint.as_mut() {
                    state.error = Some(format!("Could not remove the file: {err}"));
                }
            }
        }
        cx.notify();
    }

    /// Opens the planner with the milestones written in as the goal, so each can be broken into tasks that name the
    /// component they build and say when they are done. The user reads the plan and publishes it as they do any other.
    fn plan_from_milestones(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.blueprint.as_ref() else {
            return;
        };
        let Some(milestones) = state.docs.iter().find(|view| view.doc.kind == "milestones") else {
            return;
        };
        let components: Vec<String> = planned_from(&state.docs)
            .nodes
            .into_iter()
            .map(|node| node.label)
            .collect();
        let mut goal = format!(
            "Break these milestones into tasks. Tag each task with the component it builds, and give each task 2 to 4 \"done when\" points. Plan only the work that is not done yet.

{}",
            milestones.doc.body
        );
        if !components.is_empty() {
            goal.push_str(&format!(
                "

Components: {}.",
                components.join(", ")
            ));
        }
        let goal: String = goal.chars().take(looji_domain::MAX_GOAL_CHARS).collect();
        if self.plan_wizard.is_some() {
            return;
        }
        if self.unfinished_plan(cx).is_some() {
            if let Some(state) = self.blueprint.as_mut() {
                state.notice = Some(
                    "Finish or discard the plan in progress first, then plan the milestones."
                        .to_string(),
                );
            }
            return cx.notify();
        }
        self.start_plan_wizard(window, cx);
        if let Some(dialog) = self.plan_wizard.as_mut() {
            dialog.wizard.set_goal(&goal);
        }
        cx.notify();
    }

    /// Compares the design's component diagrams with the repository's units of code.
    pub(super) fn refresh_drift(&mut self, cx: &mut Context<Self>) {
        let root = self
            .projects
            .active_id()
            .and_then(|project| store(cx).load_path(project).ok().flatten());
        let Some(state) = self.blueprint.as_mut() else {
            return;
        };
        state.drift = root.and_then(|root| {
            let root = Path::new(&root);
            let units = actual_units(root);
            if units.is_empty() {
                return None;
            }
            let planned = planned_from(&state.docs);
            (!planned.nodes.is_empty()).then(|| compare(&planned, &units, root))
        });
        cx.notify();
    }

    pub(super) fn stop_blueprint_run(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.blueprint.as_mut()
            && let Some(run) = state.running.take()
        {
            run.cancel.cancel();
        }
        cx.notify();
    }

    fn copy_diagram(
        &mut self,
        doc: usize,
        diagram: usize,
        format: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.blueprint.as_mut() else {
            return;
        };
        let Some(view) = state.docs.get(doc).and_then(|d| d.diagrams.get(diagram)) else {
            return;
        };
        let text = to_mermaid(&view.graph);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        state.copied = Some((doc, diagram, format));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Some(state) = view.blueprint.as_mut() {
                    state.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Asks where to save a diagram, then writes it as an SVG or a PNG image.
    fn save_diagram(&mut self, doc: usize, diagram: usize, png: bool, cx: &mut Context<Self>) {
        let Some(state) = self.blueprint.as_mut() else {
            return;
        };
        let Some(view) = state.docs.get(doc).and_then(|d| d.diagrams.get(diagram)) else {
            return;
        };
        let graph = view.graph.clone();
        let name = if graph.title.is_empty() {
            "diagram".to_string()
        } else {
            super::export::file_name_part(&graph.title)
        };
        let suggested = format!("{name}.{}", if png { "png" } else { "svg" });
        let picked = cx.prompt_for_new_path(&super::export::documents_folder(), Some(&suggested));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let bytes = if png {
                        to_png(&graph, 2.0)?
                    } else {
                        to_svg(&graph).into_bytes()
                    };
                    std::fs::write(&path, bytes)
                        .map(|_| path)
                        .map_err(|err| format!("The image could not be saved: {err}"))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Some(state) = view.blueprint.as_mut() {
                    match outcome {
                        Ok(path) => state.notice = Some(format!("Saved to {}.", path.display())),
                        Err(message) => state.error = Some(message),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Goes where a diagram node points: a file shows in the file manager, a section opens in the blueprint, and an
    /// issue opens its task.
    fn follow_link(&mut self, link: Link, cx: &mut Context<Self>) {
        match link {
            Link::File(path) => {
                let Some(project) = self.projects.active_id() else {
                    return;
                };
                if let Ok(Some(root)) = store(cx).load_path(project) {
                    // Built piece by piece, so the separators are the system's own, and a target can't climb out of
                    // the repository.
                    let mut full = Path::new(&root).to_path_buf();
                    for part in path.split(['/', '\\']).filter(|part| !part.is_empty()) {
                        if part == ".." || part.contains(':') {
                            return;
                        }
                        full.push(part);
                    }
                    cx.reveal_path(&full);
                }
            }
            Link::Section(title) => {
                let wanted = title.to_lowercase();
                if let Some(state) = self.blueprint.as_mut()
                    && let Some(index) = state.docs.iter().position(|view| {
                        let doc = &view.doc;
                        doc.title.to_lowercase() == wanted
                            || kind_label(&doc.kind).to_lowercase() == wanted
                    })
                {
                    state.selected = index;
                    cx.notify();
                }
            }
            Link::Issue(target) => {
                let wanted = target.to_lowercase();
                let found = self
                    .board
                    .iter()
                    .find(|task| {
                        task.id().as_uuid().to_string() == wanted
                            || task.title().to_lowercase() == wanted
                    })
                    .map(|task| task.id());
                match found {
                    Some(task) => self.open_panel(task, cx),
                    None => {
                        if let Some(state) = self.blueprint.as_mut() {
                            state.notice =
                                Some(format!("No task called \"{target}\" on this board."));
                            cx.notify();
                        }
                    }
                }
            }
        }
    }

    /// Keys while the blueprint is open: Escape closes it, and typing goes to the idea box.
    pub(super) fn blueprint_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            return self.close_blueprint(cx);
        }
        if let Some(state) = self.blueprint.as_mut()
            && state.running.is_none()
        {
            let field = if state.revise_focused {
                &mut state.revise
            } else {
                &mut state.idea
            };
            if field.handle_key(&event.keystroke) {
                cx.notify();
            }
        }
    }
}

/// The design's component diagrams as one planned graph. Nodes keep the order they were drawn in.
fn planned_from(docs: &[DocView]) -> Planned {
    planned_from_graphs(docs.iter().flat_map(|doc| &doc.diagrams).map(|d| &d.graph))
}

/// What a review should know about the code and the design disagreeing: the saved blueprint's component diagrams
/// against the repository's own crates and packages. `None` when there is no blueprint, no code to compare, or
/// nothing disagrees. This reads the repository, so it is not for the UI thread.
pub(super) fn review_drift(root: &Path, docs: &[BlueprintDoc]) -> Option<String> {
    let graphs: Vec<Graph> = docs
        .iter()
        .flat_map(|doc| {
            serde_json::from_str::<Vec<serde_json::Value>>(&doc.diagrams).unwrap_or_default()
        })
        .filter_map(|json| parse_graph(&json).ok())
        .collect();
    let planned = planned_from_graphs(graphs.iter());
    if planned.nodes.is_empty() {
        return None;
    }
    let units = actual_units(root);
    if units.is_empty() {
        return None;
    }
    compare(&planned, &units, root).report()
}

fn planned_from_graphs<'a>(graphs: impl Iterator<Item = &'a Graph>) -> Planned {
    let mut planned = Planned::default();
    for graph in graphs.filter(|graph| graph.kind == GraphKind::Component) {
        let base = planned.nodes.len();
        planned
            .nodes
            .extend(graph.nodes.iter().map(|node| PlannedNode {
                label: node.label.clone(),
                kind: node.kind.clone(),
                file: match &node.link {
                    Some(Link::File(path)) => Some(path.clone()),
                    _ => None,
                },
            }));
        for edge in &graph.edges {
            let find = |id: &str| graph.nodes.iter().position(|node| node.id == id);
            if let (Some(from), Some(to)) = (find(&edge.from), find(&edge.to)) {
                planned.edges.push((base + from, base + to));
            }
        }
    }
    planned
}

/// What the code and the design disagree about, under the architecture document.
fn drift_block(drift: &Drift, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let line = |text: String| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(text)
    };
    let mut block = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.md)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child("Planned vs actual"),
        );
    if drift.is_clean() {
        block = block.child(line(format!(
            "The code and the design agree. {} parts of the design match code.",
            drift.matched
        )));
    }
    if !drift.missing.is_empty() {
        block = block.child(line(format!(
            "In the design, with no code behind it: {}.",
            drift.missing.join(", ")
        )));
    }
    if !drift.unclaimed.is_empty() {
        block = block.child(line(format!(
            "In the code, not in the design: {}.",
            drift.unclaimed.join(", ")
        )));
    }
    if !drift.unplanned.is_empty() {
        let pairs: Vec<String> = drift
            .unplanned
            .iter()
            .map(|(from, to)| format!("{from} depends on {to}"))
            .collect();
        block = block.child(line(format!(
            "In the code, but not drawn in the design: {}.",
            pairs.join(", ")
        )));
    }
    block
        .child(
            div().flex().flex_row().child(
                crate::components::button(
                    "drift-check",
                    "Check again",
                    crate::components::Variant::Ghost,
                    theme,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_drift(cx))),
            ),
        )
        .into_any_element()
}

/// The button in the project header that opens the blueprint.
pub(super) fn button_open(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    crate::components::button_icon(
        "project-blueprint",
        "file-text",
        "Blueprint",
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_blueprint(cx)))
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "overview" => "Overview",
        "architecture" => "Architecture",
        "data_model" => "Data model",
        "interfaces" => "Interfaces",
        "decisions" => "Decisions",
        "risks" => "Risks",
        "milestones" => "Milestones",
        _ => "Notes",
    }
}

/// The blueprint's panel: the document list on the left, the open document on the right, and the generator on top.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let state = view.blueprint.as_ref()?;
    let caption = |text: String| {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(text)
    };

    // The generator.
    let mut generator = div().flex().flex_col().gap(theme.spacing.sm).child(caption(
        "What should the blueprint cover? It researches the repository first when one is connected.".to_string(),
    ));
    generator = generator.child(
        div()
            .id("blueprint-idea")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    if let Some(state) = this.blueprint.as_mut() {
                        state.revise_focused = false;
                    }
                    cx.notify();
                }),
            )
            .child(crate::components::edit_box(
                &state.idea,
                "Describe the system",
                state.running.is_none() && !state.revise_focused,
                theme,
            )),
    );
    if let (None, Some(outline)) = (&state.running, &state.outline) {
        // The outline, for the user to keep or drop documents from before anything is written.
        generator = generator.child(caption(
            "This is the plan. Click a document to leave it out, or change the description above and plan again."
                .to_string(),
        ));
        for (index, (item, kept)) in outline.iter().enumerate() {
            generator = generator.child(
                crate::components::segment(*kept, theme)
                    .id(("blueprint-outline", index))
                    .justify_start()
                    .child(div().child(format!(
                        "{} {}: {}",
                        if *kept { "\u{2713}" } else { "\u{2717}" },
                        item.title,
                        item.summary
                    )))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_outline_item(index, cx)
                    })),
            );
        }
    }
    let has_outline = state.outline.is_some();
    let action = match &state.running {
        None if has_outline => div()
            .flex()
            .flex_row()
            .gap(theme.spacing.sm)
            .child(
                button("blueprint-write", "Write blueprint", true, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.write_blueprint(cx))),
            )
            .child(
                button("blueprint-replan", "Plan again", false, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.generate_blueprint(cx)),
                ),
            )
            .child(
                button("blueprint-discard", "Cancel", false, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.discard_outline(cx))),
            )
            .into_any_element(),
        None => button(
            "blueprint-generate",
            if state.docs.is_empty() {
                "Plan the blueprint"
            } else {
                "Plan a new version"
            },
            true,
            theme,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.generate_blueprint(cx)))
        .into_any_element(),
        Some(_) => button("blueprint-stop", "Stop", false, theme)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop_blueprint_run(cx)))
            .into_any_element(),
    };
    generator = generator.child(action);
    if let Some(run) = &state.running {
        let writing = state.running.as_ref().is_some_and(|run| !run.outlining);
        generator = generator.child(caption(
            if writing {
                "Researching and writing\u{2026}"
            } else {
                "Planning the outline\u{2026}"
            }
            .to_string(),
        ));
        let skipped = run.steps.len().saturating_sub(4);
        for step in &run.steps[skipped..] {
            generator = generator.child(caption(format!("\u{2022} {step}")));
        }
    }
    if let Some(notice) = &state.notice {
        generator = generator.child(caption(notice.clone()));
    }
    if let Some(error) = &state.error {
        generator = generator.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(error.clone()),
        );
    }

    // The document list.
    let mut list = div().flex().flex_col().gap(px(4.)).w(px(190.)).flex_none();
    for (index, doc) in state.docs.iter().enumerate() {
        list = list.child(
            crate::components::segment(index == state.selected, theme)
                .id(("blueprint-doc", index))
                .justify_start()
                .child(div().child(format!(
                    "{}{}",
                    kind_label(&doc.doc.kind),
                    if doc.doc.title.eq_ignore_ascii_case(kind_label(&doc.doc.kind)) {
                        String::new()
                    } else {
                        format!(": {}", doc.doc.title)
                    }
                )))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(state) = this.blueprint.as_mut() {
                        state.selected = index;
                    }
                    cx.notify();
                })),
        );
    }
    if !state.versions.is_empty() {
        list = list.child(caption("Versions".to_string()));
        let mut chips = div().flex().flex_row().flex_wrap().gap(px(4.));
        for (position, version) in state.versions.iter().take(6).copied().enumerate() {
            let shown = state.viewing.unwrap_or(state.versions[0]) == version;
            chips = chips.child(
                crate::components::segment(shown, theme)
                    .id(("blueprint-version", position))
                    .child(if position == 0 {
                        format!("{version} (current)")
                    } else {
                        version.to_string()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.view_version(version, cx)
                    })),
            );
        }
        list = list.child(chips);
    }

    // The open document.
    let document = match state.docs.get(state.selected) {
        None => div()
            .flex_1()
            .child(caption(
                "No blueprint yet. Describe the system above and generate one.".to_string(),
            ))
            .into_any_element(),
        Some(doc) => {
            let kind = doc.doc.kind.clone();
            let mut column = div()
                .id("blueprint-document")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .flex_1()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&state.scroll);
            if let Some(version) = state.viewing {
                column = column.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.spacing.sm)
                        .child(caption(format!(
                            "You are reading version {version}, an older one."
                        )))
                        .child(
                            button("blueprint-restore", "Restore this version", false, theme)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.restore_version(cx)
                                })),
                        ),
                );
            }
            column = column.child(
                div()
                    .text_size(theme.typography.title)
                    .font_weight(FontWeight::MEDIUM)
                    .child(doc.doc.title.clone()),
            );
            if state.viewing.is_none() {
                // Asking for a change to just this document.
                column = column.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.spacing.sm)
                        .child(
                            div()
                                .id("blueprint-revise")
                                .flex_1()
                                .min_w_0()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                        if let Some(state) = this.blueprint.as_mut() {
                                            state.revise_focused = true;
                                        }
                                        cx.notify();
                                    }),
                                )
                                .child(crate::components::edit_box(
                                    &state.revise,
                                    "Ask for a change to this document",
                                    state.running.is_none() && state.revise_focused,
                                    theme,
                                )),
                        )
                        .child(
                            button("blueprint-revise-go", "Revise", false, theme).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.revise_document(cx)),
                            ),
                        ),
                );
            }
            column = column.child(markdown_view::view(&doc.doc.body, theme));
            column = column.child(section_media(state, &kind, theme, cx));
            if doc.doc.kind == "milestones" && state.viewing.is_none() {
                column = column.child(div().flex().flex_row().child(
                    button("blueprint-plan", "Plan these as tasks", true, theme).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.plan_from_milestones(window, cx)
                        }),
                    ),
                ));
            }
            for (index, diagram) in doc.diagrams.iter().enumerate() {
                column = column.child(diagram_block(
                    state,
                    state.selected,
                    index,
                    diagram,
                    theme,
                    cx,
                ));
            }
            for wireframe in &doc.wireframes {
                column = column.child(super::wireframe_view::block(wireframe, theme));
            }
            if doc.doc.kind == "architecture"
                && state.viewing.is_none()
                && let Some(drift) = &state.drift
            {
                column = column.child(drift_block(drift, theme, cx));
            }
            column.into_any_element()
        }
    };

    Some(
        div()
            .id("blueprint-panel")
            .flex()
            .flex_col()
            .gap(theme.spacing.md)
            .size_full()
            .p(theme.spacing.lg)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(generator)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.lg)
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .child(document),
            )
            .into_any_element(),
    )
}

/// The images and files kept with a section, and the button that adds more.
fn section_media(
    state: &BlueprintState,
    doc_kind: &str,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let dir = cx.global::<AppDatabase>().attachments_dir.clone();
    let mut tiles = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(theme.spacing.sm)
        .items_center();
    for (index, item) in state
        .media
        .iter()
        .filter(|item| item.doc_kind == doc_kind)
        .enumerate()
    {
        let path = dir.join(&item.stored_name);
        let body = match item.kind {
            MediaKind::Image => div()
                .w(px(180.))
                .h(px(120.))
                .rounded(theme.radius.md)
                .overflow_hidden()
                .border_1()
                .border_color(theme.palette.border)
                .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
                .into_any_element(),
            _ => div()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .pr(px(30.))
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .bg(theme.palette.surface)
                .text_size(theme.typography.caption)
                .child(item.file_name.clone())
                .into_any_element(),
        };
        let (remove_kind, remove_name, open) =
            (item.doc_kind.clone(), item.stored_name.clone(), path);
        tiles = tiles.child(
            div()
                .id(("section-media", index))
                .relative()
                .cursor_pointer()
                .child(body)
                .child(
                    div().absolute().top(px(4.)).right(px(4.)).child(
                        crate::components::icon_button(("section-media-remove", index), "x", theme)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.remove_section_media(
                                    remove_kind.clone(),
                                    remove_name.clone(),
                                    cx,
                                )
                            })),
                    ),
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_with_system(&open))),
        );
    }
    let kind = doc_kind.to_string();
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(tiles)
        .child(
            div().flex().flex_row().child(
                crate::components::button(
                    "section-media-add",
                    "Add an image or file",
                    crate::components::Variant::Ghost,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.add_section_media(kind.clone(), cx)
                })),
            ),
        )
        .into_any_element()
}

/// One diagram with its title and the copy buttons.
fn diagram_block(
    state: &BlueprintState,
    doc: usize,
    index: usize,
    diagram: &DiagramView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let label = |format: &'static str| {
        if state.copied == Some((doc, index, format)) {
            "Copied"
        } else {
            "Copy Mermaid"
        }
    };
    let kind = match diagram.graph.kind {
        GraphKind::Component => "Components",
        GraphKind::Flow => "Flow",
        GraphKind::Entity => "Data",
        GraphKind::Deployment => "Deployment",
    };
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(if diagram.graph.title.is_empty() {
                            kind.to_string()
                        } else {
                            format!("{kind}: {}", diagram.graph.title)
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.xs)
                        .child(
                            button(
                                ("diagram-mermaid", doc * 100 + index),
                                label("Mermaid"),
                                false,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.copy_diagram(doc, index, "Mermaid", cx)
                                },
                            )),
                        )
                        .child(
                            button(("diagram-svg", doc * 100 + index), "Save SVG", false, theme)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.save_diagram(doc, index, false, cx)
                                })),
                        )
                        .child(
                            button(("diagram-png", doc * 100 + index), "Save PNG", false, theme)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.save_diagram(doc, index, true, cx)
                                })),
                        ),
                ),
        )
        .child(
            div()
                .id(("diagram-scroll", doc * 100 + index))
                .overflow_x_scroll()
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .bg(theme.palette.surface)
                .child(draw(diagram, theme, cx)),
        )
        .into_any_element()
}

/// The diagram, drawn from its layout: groups behind, edges as paths, nodes on top.
fn draw(diagram: &DiagramView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let layout = &diagram.layout;
    let graph = &diagram.graph;
    let mut surface = div()
        .relative()
        .w(px(layout.width))
        .h(px(layout.height))
        .flex_none();
    for group in &layout.groups {
        surface = surface.child(
            div()
                .absolute()
                .left(px(group.x))
                .top(px(group.y))
                .w(px(group.w))
                .h(px(group.h))
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .bg(theme.palette.surface_hover)
                .child(
                    div()
                        .px(theme.spacing.sm)
                        .pt(px(3.))
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(graph.groups[group.index].label.clone()),
                ),
        );
    }

    let routes: Vec<(Vec<(f32, f32)>, bool)> = layout
        .routes
        .iter()
        .map(|route| {
            (
                route.points.clone(),
                graph.edges[route.edge].style == looji_diagram::EdgeStyle::Dashed,
            )
        })
        .collect();
    let line = theme.palette.border_control;
    surface = surface.child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for (points, dashed) in &routes {
                    let mut builder = if *dashed {
                        PathBuilder::stroke(px(1.2)).dash_array(&[px(5.), px(4.)])
                    } else {
                        PathBuilder::stroke(px(1.2))
                    };
                    let at = |(x, y): (f32, f32)| bounds.origin + point(px(x), px(y));
                    builder.move_to(at(points[0]));
                    for next in &points[1..] {
                        builder.line_to(at(*next));
                    }
                    if let Some(&(x, y)) = points.last() {
                        // A small arrowhead at the end, pointing along the last segment.
                        let before = points[points.len() - 2];
                        let (dx, dy) = (x - before.0, y - before.1);
                        let length = (dx * dx + dy * dy).sqrt().max(1.0);
                        let (ux, uy) = (dx / length, dy / length);
                        let base = (x - ux * 9.0, y - uy * 9.0);
                        let (px_, py_) = (-uy * 4.5, ux * 4.5);
                        builder.move_to(at((x, y)));
                        builder.line_to(at((base.0 + px_, base.1 + py_)));
                        builder.move_to(at((x, y)));
                        builder.line_to(at((base.0 - px_, base.1 - py_)));
                    }
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, line);
                    }
                }
            },
        )
        .absolute()
        .size_full(),
    );

    for route in &layout.routes {
        if let Some(label) = &graph.edges[route.edge].label {
            let middle = route.points[route.points.len() / 2];
            let before = route.points[route.points.len() / 2 - 1];
            surface = surface.child(
                div()
                    .absolute()
                    .left(px((middle.0 + before.0) / 2.0 - 40.0))
                    .top(px((middle.1 + before.1) / 2.0 - 18.0))
                    .w(px(80.))
                    .flex()
                    .justify_center()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(label.clone()),
            );
        }
    }
    for node in &layout.nodes {
        let source = &graph.nodes[node.index];
        let mut inner = div()
            .id(("diagram-node", node.index))
            .absolute()
            .left(px(node.x))
            .top(px(node.y))
            .w(px(node.w))
            .h(px(node.h))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .bg(theme.palette.surface_raised)
            .text_size(theme.typography.body)
            .child(source.label.clone());
        if let Some(kind) = &source.kind {
            inner = inner.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(kind.clone()),
            );
        }
        if let Some(link) = source.link.clone() {
            // A node that points somewhere says so with its outline, and a click goes there.
            inner = inner
                .border_color(theme.palette.accent)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.follow_link(link.clone(), cx)
                }));
        }
        surface = surface.child(inner);
    }
    surface.into_any_element()
}

#[cfg(test)]
mod review_tests {
    use super::review_drift;
    use looji_domain::BlueprintDoc;

    fn architecture(nodes: &str) -> BlueprintDoc {
        let diagram =
            format!(r#"[{{"type":"component","title":"Overview","nodes":[{nodes}],"edges":[]}}]"#);
        BlueprintDoc::new("architecture", "Architecture", "Body.", &diagram)
    }

    fn repository(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("looji-drift-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"shop\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn a_review_hears_about_parts_without_code_and_code_without_parts() {
        let root = repository("disagree");
        let docs = [architecture(r#"{"id":"b","label":"Billing"}"#)];
        let report = review_drift(&root, &docs).expect("they disagree");
        assert!(report.contains("Billing"), "{report}");
        assert!(report.contains("shop"), "{report}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_is_reported_without_a_blueprint_or_when_they_agree() {
        let root = repository("agree");
        assert_eq!(review_drift(&root, &[]), None);
        let docs = [architecture(r#"{"id":"s","label":"Shop"}"#)];
        assert_eq!(review_drift(&root, &docs), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
