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
    blueprint::{build_blueprint_request, parse_blueprint},
    generation::{Update, run_with},
};
use looji_diagram::{Graph, GraphKind, Layout, Link, layout, parse_graph, to_mermaid, to_svg};
use looji_domain::{BlueprintDoc, BlueprintRepository, CodeSourceRepository, Timestamp};
use looji_knowledge::drift::{Drift, Planned, PlannedNode, actual_units, compare};

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
    DocView { doc, diagrams }
}

/// A generation in progress.
struct Run {
    steps: Vec<String>,
    queue: Arc<Mutex<Vec<Update>>>,
    cancel: CancellationToken,
    _reader: Task<()>,
}

pub(super) struct BlueprintState {
    docs: Vec<DocView>,
    selected: usize,
    idea: TextEdit,
    running: Option<Run>,
    notice: Option<String>,
    error: Option<String>,
    /// The diagram whose copy button last said "Copied": the document, the diagram, and which format.
    copied: Option<(usize, usize, &'static str)>,
    scroll: ScrollHandle,
    /// The design compared with the code, when the project has a repository with units of code.
    drift: Option<Drift>,
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
            running: None,
            notice: None,
            error: None,
            copied: None,
            scroll: ScrollHandle::new(),
            drift: None,
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
    pub(super) fn generate_blueprint(&mut self, cx: &mut Context<Self>) {
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
        state.error = None;
        state.notice = None;
        let root = store(cx).load_path(id).ok().flatten();
        let model = super::ai_settings::model_for(cx, Some(id));
        let provider = super::ai_settings::provider_for(cx, Some(id));
        let knowledge_path = cx.global::<AppDatabase>().knowledge_path.clone();
        let request = build_blueprint_request(&model, &name, &description, &idea, root.is_some());

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
                    Some(root) => {
                        super::agent::agent_for(provider, &knowledge_path, id, Path::new(&root))
                    }
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
        let updates = match run.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        let mut finished = None;
        for update in updates {
            match update {
                Update::Step(step) => run.steps.push(step),
                Update::Finished(text) => finished = Some(parse_blueprint(&text)),
                Update::Failed(err) => finished = Some(Err(err.to_string())),
                Update::Text(_) | Update::Source(_) => {}
            }
        }
        let Some(outcome) = finished else {
            cx.notify();
            return true;
        };
        state.running = None;
        let mut refresh = false;
        match outcome {
            Ok(parsed) => {
                let docs: Vec<BlueprintDoc> = parsed
                    .docs
                    .iter()
                    .map(|doc| {
                        BlueprintDoc::new(
                            &doc.kind,
                            &doc.title,
                            &doc.body,
                            &serde_json::Value::Array(doc.diagrams.clone()).to_string(),
                        )
                    })
                    .collect();
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
                        state.selected = 0;
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
            .map(|doc| (doc.kind, doc.title, doc.body))
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
        let text = if format == "Mermaid" {
            to_mermaid(&view.graph)
        } else {
            to_svg(&view.graph)
        };
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
            && state.idea.handle_key(&event.keystroke)
        {
            cx.notify();
        }
    }
}

/// The design's component diagrams as one planned graph. Nodes keep the order they were drawn in.
fn planned_from(docs: &[DocView]) -> Planned {
    let mut planned = Planned::default();
    for diagram in docs
        .iter()
        .flat_map(|doc| &doc.diagrams)
        .filter(|diagram| diagram.graph.kind == GraphKind::Component)
    {
        let base = planned.nodes.len();
        let graph = &diagram.graph;
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
    generator = generator.child(crate::components::edit_box(
        &state.idea,
        "Describe the system",
        state.running.is_none(),
        theme,
    ));
    let action = match &state.running {
        None => button(
            "blueprint-generate",
            if state.docs.is_empty() {
                "Generate blueprint"
            } else {
                "Generate a new version"
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
        generator = generator.child(caption("Researching and writing\u{2026}".to_string()));
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
            column = column
                .child(
                    div()
                        .text_size(theme.typography.title)
                        .font_weight(FontWeight::MEDIUM)
                        .child(doc.doc.title.clone()),
                )
                .child(markdown_view::view(&doc.doc.body, theme));
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
            match format {
                "Mermaid" => "Copy Mermaid",
                _ => "Copy SVG",
            }
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
                            button(
                                ("diagram-svg", doc * 100 + index),
                                label("SVG"),
                                false,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.copy_diagram(doc, index, "SVG", cx)
                                },
                            )),
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
