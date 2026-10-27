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
use looji_diagram::{Graph, GraphKind, Layout, layout, parse_graph, to_mermaid, to_svg};
use looji_domain::{BlueprintDoc, BlueprintRepository, CodeSourceRepository, Timestamp};

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
        });
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
        cx.notify();
        false
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
    if let Some(first) = state.docs.first() {
        list = list.child(caption(format!("Version {}", first.doc.version)));
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
                .track_scroll(&state.scroll)
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
                .child(draw(diagram, theme)),
        )
        .into_any_element()
}

/// The diagram, drawn from its layout: groups behind, edges as paths, nodes on top.
fn draw(diagram: &DiagramView, theme: &Theme) -> AnyElement {
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
        surface = surface.child(inner);
    }
    surface.into_any_element()
}
