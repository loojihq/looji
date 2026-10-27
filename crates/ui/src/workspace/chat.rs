//! The task chat: a dialog about one task. It shows the conversation, streams the reply as it arrives, and keeps what
//! arrived when the user stops or the connection fails. Each message is saved as it's sent and as its reply ends, so
//! a conversation reloads after a restart. The model is remembered per task, and a change applies to the next
//! message. The reply's markdown is drawn by `markdown` and `chat_render`.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{prelude::*, *};
use looji_ai::{
    AiError, CancellationToken, ChatMessage as AiMessage, ChatProvider, ChatRequest, Source,
    chat::{HISTORY_BUDGET_TOKENS, TaskBrief, build_agent_request, build_chat_request},
    generation::{Update, run_with},
    models::{DEFAULT_MODEL, MODELS},
};
use looji_domain::{
    ChatMessage, ChatThreadId, CodeSourceRepository, DEFAULT_TOKEN_BUDGET, Hunk, MessageRole,
    ProjectId, Reply, ReplyState, SettingsRepository, TaskChatRepository, TaskId, resolve_model,
};
use looji_persistence::{SqliteStore, repo_index::IndexError};

use super::{WorkspaceView, store, task_dialog::button};
use crate::{components::edit_box, database::AppDatabase, text_edit::TextEdit, theme::Theme};

/// How often the reader checks the stream for new text.
const TICK: Duration = Duration::from_millis(60);
/// How long "Copied" stays on a button.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// A reply in progress: where it will be saved, what it has so far, and how to stop it.
pub(super) struct Active {
    pub thread: ChatThreadId,
    /// The position of the assistant message this reply fills in.
    pub position: u32,
    pub reply: Reply,
    pub cancel: CancellationToken,
    pub queue: Arc<Mutex<Vec<Update>>>,
    /// What an agent has done so far, oldest first, shown while it works.
    pub steps: Vec<String>,
    /// What an agent read, in the order it read it, without repeats.
    pub sources: Vec<Source>,
    pub _reader: Task<()>,
}

/// An open chat. The messages are reloaded from the store after each change, so the dialog always shows what's saved.
pub(super) struct ChatDialog {
    /// The task the conversation is about. For a conversation about the whole project, the project's own id.
    pub task: TaskId,
    pub project: ProjectId,
    /// The conversation is about the project's whole codebase, not one task.
    pub whole_project: bool,
    pub title: String,
    pub description: String,
    pub status: String,
    pub thread: Option<ChatThreadId>,
    pub messages: Vec<ChatMessage>,
    pub draft: TextEdit,
    pub model: String,
    /// Shown when the saved model was removed and the default was used instead.
    pub model_notice: Option<String>,
    pub model_menu: bool,
    /// Whether the code of the project's repository is sent with each message.
    pub use_code: bool,
    /// Whether the project has a connected repository to take code from.
    pub code_available: bool,
    pub active: Option<Active>,
    pub error: Option<String>,
    /// The code block whose copy button last said "Copied": the message position and the block's index in it.
    pub copied: Option<(u32, usize)>,
    /// What each reply drew on, by the reply's position. Kept while the dialog is open.
    pub sources: HashMap<u32, Vec<Source>>,
}

/// What a reply needs once its context is ready. Built before the reply starts, so retrieval can run in between.
struct Launch {
    position: u32,
    model: String,
    history: Vec<AiMessage>,
    title: String,
    description: String,
    status: String,
    whole_project: bool,
}

/// What a conversation is about, when it opens.
struct Subject {
    owner: TaskId,
    project: ProjectId,
    title: String,
    description: String,
    status: String,
    whole_project: bool,
}

/// The button in the project header that opens a chat about the project's whole codebase.
pub(super) fn codebase_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    crate::components::button_icon(
        "project-ask-ai",
        "sparkles",
        "Ask AI",
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_project_chat(window, cx)))
}

fn model_key(task: TaskId) -> String {
    format!("chat.model.{}", task.as_uuid())
}

fn code_key(task: TaskId) -> String {
    format!("chat.code.{}", task.as_uuid())
}

impl WorkspaceView {
    /// Opens the chat for the selected task. Nothing opens when no task is selected.
    pub(super) fn open_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.panel.as_ref().map(|panel| panel.task) else {
            return;
        };
        self.open_chat_for(task_id, window, cx);
    }

    /// Opens the AI chat about one task. Nothing opens when the task is no longer on the board.
    pub(super) fn open_chat_for(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.board.iter().find(|task| task.id() == task_id).cloned() else {
            return;
        };
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let subject = Subject {
            owner: task_id,
            project,
            title: task.title().to_string(),
            description: task.description().to_string(),
            status: task.status().as_str().to_string(),
            whole_project: false,
        };
        self.open_chat_subject(subject, window, cx);
    }

    /// Opens the chat about a task and asks for an explanation of it at once, so the button answers the question it
    /// names. Repository code is turned on when the project has a repository, so the answer can point at the code.
    pub(super) fn explain_issue(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_chat_for(task_id, window, cx);
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        if dialog.task != task_id {
            return;
        }
        if dialog.code_available {
            dialog.use_code = true;
        }
        dialog.draft = TextEdit::new(
            "Explain this issue: what it asks for, which parts of the code it touches, and where to start.",
        );
        self.send_chat_message(cx);
    }

    /// Asks the open chat a ready-made question at once. Repository code is turned on when the project has a
    /// repository, so the answer can use the commits.
    pub(super) fn ask_preset(&mut self, question: &str, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        if dialog.active.is_some() {
            return;
        }
        if dialog.code_available {
            dialog.use_code = true;
        }
        dialog.draft = TextEdit::new(question);
        self.send_chat_message(cx);
    }

    /// Opens the AI chat about the open project's whole codebase. It is saved with the project, under the project's
    /// own id.
    pub(super) fn open_project_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let id = project.id();
        let subject = Subject {
            owner: TaskId::from_uuid(id.as_uuid()),
            project: id,
            title: project.name().to_string(),
            description: project.description().unwrap_or_default().to_string(),
            status: String::new(),
            whole_project: true,
        };
        self.open_chat_subject(subject, window, cx);
    }

    fn open_chat_subject(&mut self, subject: Subject, window: &mut Window, cx: &mut Context<Self>) {
        let Subject {
            owner: task_id,
            project,
            title,
            description,
            status,
            whole_project,
        } = subject;
        let saved = store(cx).get_setting(&model_key(task_id)).ok().flatten();
        let choice = resolve_model(saved.as_deref(), MODELS, DEFAULT_MODEL);
        let code_available = store(cx).load_path(project).ok().flatten().is_some();
        let use_code = code_available
            && match store(cx)
                .get_setting(&code_key(task_id))
                .ok()
                .flatten()
                .as_deref()
            {
                Some(saved) => saved == "on",
                // A conversation about the codebase starts with the code on. One about a task starts with it off.
                None => whole_project,
            };
        let thread = store(cx)
            .threads(task_id)
            .ok()
            .and_then(|threads| threads.last().copied());
        let messages = thread
            .and_then(|thread| store(cx).messages(thread).ok())
            .unwrap_or_default();
        let sources: HashMap<u32, Vec<Source>> = thread
            .and_then(|thread| store(cx).message_sources(thread).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(position, text)| (position, sources_from_json(&text)))
            .collect();
        self.chat_dialog = Some(ChatDialog {
            task: task_id,
            project,
            whole_project,
            title,
            description,
            status,
            thread,
            messages,
            draft: TextEdit::default(),
            model: choice.model,
            model_notice: choice.notice.or_else(|| {
                (whole_project && !code_available).then(|| {
                    "No repository is connected. Connect one in the project's settings to ask about its code."
                        .to_string()
                })
            }),
            model_menu: false,
            use_code,
            code_available,
            active: None,
            error: None,
            copied: None,
            sources,
        });
        if choice_saved_missing(saved.as_deref()) {
            // The saved model is gone, so the notice explains it and the saved value is replaced by the default.
            let default = DEFAULT_MODEL.to_string();
            let _ = store(cx).set_setting(&model_key(task_id), &default);
        }
        self.chat_scroll.scroll_to_bottom();
        let main = cx.entity();
        cx.defer(move |cx| super::dialog_window::open(cx, main, super::dialog_window::Kind::Chat));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Closes the chat. A reply still streaming is stopped first, and what arrived is kept.
    pub(super) fn close_chat(&mut self, cx: &mut Context<Self>) {
        self.stop_chat_reply(cx);
        self.chat_dialog = None;
        cx.notify();
    }

    fn reload_chat(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        if let Some(thread) = dialog.thread {
            dialog.messages = store(cx).messages(thread).unwrap_or_default();
        }
        self.chat_scroll.scroll_to_bottom();
    }

    /// Sends the draft as a new message, then starts the reply.
    pub(super) fn send_chat_message(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        let text = dialog.draft.trim().to_string();
        if text.is_empty() || dialog.active.is_some() {
            return;
        }
        dialog.draft.clear();
        dialog.error = None;
        let task = dialog.task;
        let thread = match dialog.thread {
            Some(thread) => thread,
            None => match store(cx).start_thread(task) {
                Ok(thread) => {
                    if let Some(dialog) = self.chat_dialog.as_mut() {
                        dialog.thread = Some(thread);
                    }
                    thread
                }
                Err(err) => {
                    return self.chat_error(format!("Could not start the conversation: {err}"), cx);
                }
            },
        };
        if let Err(err) = store(cx).append_message(thread, MessageRole::User, &text, None) {
            return self.chat_error(format!("Could not save the message: {err}"), cx);
        }
        self.start_reply(thread, cx);
    }

    /// Starts a reply to the conversation as it's saved now, which ends with a user message.
    fn start_reply(&mut self, thread: ChatThreadId, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_ref() else {
            return;
        };
        let model = dialog.model.clone();
        let use_code = dialog.use_code;
        let project = dialog.project;
        let whole_project = dialog.whole_project;
        let (title, description, status) = (
            dialog.title.clone(),
            dialog.description.clone(),
            dialog.status.clone(),
        );
        let history: Vec<AiMessage> = store(cx)
            .messages(thread)
            .unwrap_or_default()
            .into_iter()
            .filter(|message| {
                message.state != ReplyState::Streaming
                    && !matches!(message.state, ReplyState::Failed { .. })
            })
            .map(|message| match message.role {
                MessageRole::User => AiMessage::user(message.content),
                MessageRole::Assistant => AiMessage::assistant(message.content),
            })
            .collect();
        let position =
            match store(cx).append_message(thread, MessageRole::Assistant, "", Some(&model)) {
                Ok(position) => position,
                Err(err) => return self.chat_error(format!("Could not save the reply: {err}"), cx),
            };

        let cancel = CancellationToken::new();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let reader = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep_reading = this
                    .update(cx, |view, cx| view.read_chat_updates(cx))
                    .unwrap_or(false);
                if !keep_reading {
                    break;
                }
            }
        });
        if let Some(dialog) = self.chat_dialog.as_mut() {
            dialog.active = Some(Active {
                thread,
                position,
                reply: Reply::streaming(),
                cancel: cancel.clone(),
                queue,
                steps: Vec::new(),
                sources: Vec::new(),
                _reader: reader,
            });
        }
        self.reload_chat(cx);
        cx.notify();

        let launch = Launch {
            position,
            model,
            history,
            title,
            description,
            status,
            whole_project,
        };
        let agent_root = if use_code && !fake_ai() {
            store(cx).load_path(project).ok().flatten()
        } else {
            None
        };
        if let Some(root) = agent_root {
            self.launch_agent_reply(launch, project, root, cancel, cx);
        } else if use_code {
            // Retrieval reads the repository and may refresh its index, so it runs off the UI thread on its own
            // connection. Its outcome decides what the model is shown, but the reply starts either way.
            let database_path = cx.global::<AppDatabase>().database_path.clone();
            let knowledge_path = cx.global::<AppDatabase>().knowledge_path.clone();
            // A task is looked up by its title and description. A question about the codebase is looked up by what
            // was asked.
            let (title, description) = if launch.whole_project {
                (latest_question(&launch.history), String::new())
            } else {
                (launch.title.clone(), launch.description.clone())
            };
            let retrieval = cx.background_executor().spawn(async move {
                retrieve_code(
                    &database_path,
                    &knowledge_path,
                    project,
                    &title,
                    &description,
                )
            });
            cx.spawn(async move |this, cx| {
                let (hunks, notice) = match retrieval.await {
                    Ok(hunks) => (hunks, None),
                    Err(message) => (Vec::new(), Some(message)),
                };
                let _ = this.update(cx, |view, cx| {
                    if let Some(notice) = notice {
                        view.set_chat_notice(notice, cx);
                    }
                    view.launch_reply(launch, hunks, cancel, cx)
                });
            })
            .detach();
        } else {
            self.launch_reply(launch, Vec::new(), cancel, cx);
        }
    }

    fn launch_reply(
        &mut self,
        launch: Launch,
        hunks: Vec<Hunk>,
        cancel: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        // The user may have stopped the reply while the code was being found.
        if cancel.is_cancelled() {
            return;
        }
        let brief = TaskBrief {
            title: &launch.title,
            description: &launch.description,
            status: &launch.status,
            whole_project: launch.whole_project,
        };
        let (request, _left_out) = build_chat_request(
            &launch.model,
            brief,
            &launch.history,
            &hunks,
            HISTORY_BUDGET_TOKENS,
        );
        let queue = match self
            .chat_dialog
            .as_ref()
            .and_then(|dialog| dialog.active.as_ref())
        {
            Some(active) if active.position == launch.position => Arc::clone(&active.queue),
            _ => return,
        };
        let provider = super::ai_settings::provider_for(cx, self.projects.active_id());
        spawn_reply(request, cancel, queue, provider);
        cx.notify();
    }

    /// Starts a reply from an agent, which searches and reads the repository itself instead of being handed code.
    fn launch_agent_reply(
        &mut self,
        launch: Launch,
        project: ProjectId,
        root: String,
        cancel: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        let brief = TaskBrief {
            title: &launch.title,
            description: &launch.description,
            status: &launch.status,
            whole_project: launch.whole_project,
        };
        let (request, _left_out) =
            build_agent_request(&launch.model, brief, &launch.history, HISTORY_BUDGET_TOKENS);
        let queue = match self
            .chat_dialog
            .as_ref()
            .and_then(|dialog| dialog.active.as_ref())
        {
            Some(active) if active.position == launch.position => Arc::clone(&active.queue),
            _ => return,
        };
        let provider = super::ai_settings::provider_for(cx, Some(project));
        let knowledge_path = cx.global::<AppDatabase>().knowledge_path.clone();
        let agent =
            super::agent::agent_for(provider, &knowledge_path, project, Path::new(&root), false);
        spawn_agent_reply(request, cancel, queue, agent);
        cx.notify();
    }

    /// Reads the updates the stream has sent since the last read. Returns whether the reader should keep going.
    fn read_chat_updates(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return false;
        };
        let Some(active) = dialog.active.as_mut() else {
            return false;
        };
        let updates = match active.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        for update in updates {
            match update {
                Update::Text(token) => active.reply.push(&token),
                Update::Step(step) => active.steps.push(step),
                Update::Source(source) => {
                    if !active.sources.contains(&source) {
                        active.sources.push(source);
                    }
                }
                Update::Finished(_) => active.reply.complete(),
                Update::Failed(err) => active.reply.fail(failure_text(&err)),
            }
        }
        if active.reply.state == ReplyState::Streaming {
            self.chat_scroll.scroll_to_bottom();
            cx.notify();
            return true;
        }
        let finished = (
            active.thread,
            active.position,
            active.reply.text.clone(),
            active.reply.state.clone(),
        );
        let (position, sources) = (active.position, std::mem::take(&mut active.sources));
        if !sources.is_empty() {
            dialog.sources.insert(position, sources);
        }
        dialog.active = None;
        self.save_reply(finished, cx);
        cx.notify();
        false
    }

    fn save_reply(
        &mut self,
        (thread, position, text, state): (ChatThreadId, u32, String, ReplyState),
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = store(cx).finish_message(thread, position, &text, &state) {
            self.chat_error(format!("Could not save the reply: {err}"), cx);
        }
        let sources = self
            .chat_dialog
            .as_ref()
            .and_then(|dialog| dialog.sources.get(&position))
            .map(|sources| sources_to_json(sources));
        if let Some(sources) = sources
            && let Err(err) = store(cx).set_message_sources(thread, position, &sources)
        {
            eprintln!("could not save the sources of a reply: {err}");
        }
        self.reload_chat(cx);
    }

    /// Stops the reply in progress. What has arrived is saved as a stopped reply, which keeps its text.
    pub(super) fn stop_chat_reply(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        let Some(mut active) = dialog.active.take() else {
            return;
        };
        active.cancel.cancel();
        active.reply.stop();
        let finished = (
            active.thread,
            active.position,
            active.reply.text.clone(),
            active.reply.state.clone(),
        );
        if !active.sources.is_empty()
            && let Some(dialog) = self.chat_dialog.as_mut()
        {
            dialog
                .sources
                .insert(active.position, std::mem::take(&mut active.sources));
        }
        self.save_reply(finished, cx);
        cx.notify();
    }

    /// Replaces a reply that failed or was stopped with a new one to the same message.
    pub(super) fn retry_chat_reply(&mut self, position: u32, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_ref() else {
            return;
        };
        if dialog.active.is_some() {
            return;
        }
        let Some(thread) = dialog.thread else {
            return;
        };
        if let Err(err) = store(cx).truncate_from(thread, position) {
            return self.chat_error(format!("Could not remove the failed reply: {err}"), cx);
        }
        self.reload_chat(cx);
        self.start_reply(thread, cx);
    }

    pub(super) fn set_chat_model(&mut self, model: &str, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        let task = dialog.task;
        // Only the next message uses the new model. A reply in progress keeps the one it started with.
        dialog.model = model.to_string();
        dialog.model_notice = None;
        dialog.model_menu = false;
        if let Err(err) = store(cx).set_setting(&model_key(task), model) {
            self.chat_error(format!("Could not remember the model: {err}"), cx);
        }
        cx.notify();
    }

    pub(super) fn toggle_chat_code(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        if !dialog.code_available {
            return;
        }
        dialog.use_code = !dialog.use_code;
        let value = if dialog.use_code { "on" } else { "off" };
        let task = dialog.task;
        if let Err(err) = store(cx).set_setting(&code_key(task), value) {
            self.chat_error(format!("Could not remember the setting: {err}"), cx);
        }
        cx.notify();
    }

    pub(super) fn copy_code(
        &mut self,
        position: u32,
        index: usize,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let Some(dialog) = self.chat_dialog.as_mut() else {
            return;
        };
        dialog.copied = Some((position, index));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |view, cx| {
                if let Some(dialog) = view.chat_dialog.as_mut()
                    && dialog.copied == Some((position, index))
                {
                    dialog.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_chat_notice(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.chat_dialog.as_mut() {
            dialog.model_notice = Some(message);
        }
        cx.notify();
    }

    fn chat_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.chat_dialog.as_mut() {
            dialog.error = Some(message);
        }
        cx.notify();
    }

    /// Keys while the chat is open. Enter sends, and Escape closes the model menu first, then the chat.
    pub(super) fn chat_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                if self
                    .chat_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.model_menu)
                {
                    if let Some(dialog) = self.chat_dialog.as_mut() {
                        dialog.model_menu = false;
                    }
                    cx.notify();
                } else {
                    self.close_chat(cx);
                }
            }
            "enter" => self.send_chat_message(cx),
            _ => {
                if let Some(dialog) = self.chat_dialog.as_mut()
                    && dialog.draft.handle_key(&event.keystroke)
                {
                    cx.notify();
                }
            }
        }
    }
}

/// A reply's sources as saved with it.
fn sources_to_json(sources: &[Source]) -> String {
    serde_json::Value::Array(
        sources
            .iter()
            .map(|source| {
                serde_json::json!({
                    "kind": source.kind,
                    "reference": source.reference,
                    "label": source.label,
                    "start_line": source.start_line,
                    "end_line": source.end_line,
                })
            })
            .collect(),
    )
    .to_string()
}

/// Sources as saved. Anything that can't be read is left out.
fn sources_from_json(text: &str) -> Vec<Source> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(Source {
                kind: item["kind"].as_str()?.to_string(),
                reference: item["reference"].as_str()?.to_string(),
                label: item["label"].as_str().unwrap_or_default().to_string(),
                start_line: item["start_line"].as_u64().unwrap_or(0) as u32,
                end_line: item["end_line"].as_u64().unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// Whether a saved model name is one the provider no longer lists.
fn choice_saved_missing(saved: Option<&str>) -> bool {
    matches!(saved, Some(model) if !MODELS.contains(&model))
}

/// What a failed reply says. Each error is a sentence the user can act on.
fn failure_text(err: &AiError) -> String {
    err.to_string()
}

/// The user's latest message in the conversation, which a question about the codebase is looked up by.
fn latest_question(history: &[AiMessage]) -> String {
    history
        .iter()
        .rev()
        .find(|message| message.role == looji_ai::ChatRole::User)
        .map(|message| message.content.clone())
        .unwrap_or_default()
}

/// The code blocks' hunks for a task, read on a separate connection to the database. The error is a notice for the
/// user, and the reply goes ahead without the code.
fn retrieve_code(
    database_path: &Path,
    knowledge_path: &Path,
    project: ProjectId,
    title: &str,
    description: &str,
) -> Result<Vec<Hunk>, String> {
    // The knowledge base is current and ranked. Until it has read the project, the repository is read directly.
    if let Some(hunks) =
        super::knowledge::search_hunks(knowledge_path, project, &format!("{title} {description}"))
    {
        return Ok(hunks);
    }
    let store = SqliteStore::open(database_path)
        .map_err(|err| format!("Repository code was not sent: {err}"))?;
    let root = store
        .load_path(project)
        .map_err(|err| format!("Repository code was not sent: {err}"))?
        .ok_or_else(|| "Repository code was not sent: no repository is connected.".to_string())?;
    let root = Path::new(&root);
    store
        .refresh_repo_index(project, root)
        .map_err(|err: IndexError| format!("Repository code was not sent: {err}"))?;
    store
        .retrieve_hunks(project, root, title, description, DEFAULT_TOKEN_BUDGET)
        .map_err(|err| format!("Repository code was not sent: {err}"))
}

/// Runs the request on the AI runtime and pushes what happens into the queue the reader drains. A debug build with
/// `LOOJI_FAKE_AI` set uses a scripted provider, so streaming and failures can be checked without a key.
fn spawn_reply(
    request: ChatRequest,
    cancel: CancellationToken,
    queue: Arc<Mutex<Vec<Update>>>,
    provider: looji_ai::Provider,
) {
    let future = async move {
        let sink = queue;
        let push = move |update: Update| {
            if let Ok(mut queue) = sink.lock() {
                queue.push(update);
            }
        };
        match provider_for(&request, provider) {
            Ok(provider) => run_with(provider.as_ref(), request, cancel, push).await,
            Err(err) => push(Update::Failed(err)),
        }
    };
    drop(looji_ai::runtime().spawn(future));
}

/// Runs the request through an agent. A failure to set the agent up is reported like any failed reply.
fn spawn_agent_reply(
    request: ChatRequest,
    cancel: CancellationToken,
    queue: Arc<Mutex<Vec<Update>>>,
    agent: Result<Arc<dyn ChatProvider>, AiError>,
) {
    let future = async move {
        let sink = queue;
        let push = move |update: Update| {
            if let Ok(mut queue) = sink.lock() {
                queue.push(update);
            }
        };
        match agent {
            Ok(agent) => run_with(agent.as_ref(), request, cancel, push).await,
            Err(err) => push(Update::Failed(err)),
        }
    };
    drop(looji_ai::runtime().spawn(future));
}

/// A debug build with `LOOJI_FAKE_AI` set answers from a script, so the agent is not used.
fn fake_ai() -> bool {
    cfg!(debug_assertions) && std::env::var_os("LOOJI_FAKE_AI").is_some()
}

#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn provider_for(
    request: &ChatRequest,
    choice: looji_ai::Provider,
) -> Result<Arc<dyn ChatProvider>, AiError> {
    #[cfg(debug_assertions)]
    if let Some(provider) = fake_provider(request) {
        return Ok(provider);
    }
    looji_ai::connect(choice)
}

/// A scripted reply for manual checks: "fail" in the message breaks the stream partway, "slow" makes the reply long,
/// and "markdown" sends a reply with every kind of block.
#[cfg(debug_assertions)]
fn fake_provider(request: &ChatRequest) -> Option<Arc<dyn ChatProvider>> {
    use looji_ai::{
        AiError,
        fake::{FakeProvider, Script, Step},
    };

    std::env::var_os("LOOJI_FAKE_AI")?;
    let last = request.messages.last()?.content.clone();
    let mut steps = Vec::new();
    if last.contains("markdown") {
        for chunk in MARKDOWN_SAMPLE.as_bytes().chunks(24) {
            steps.push(Step::Token(String::from_utf8_lossy(chunk).into_owned()));
            steps.push(Step::Pause(Duration::from_millis(30)));
        }
        steps.push(Step::End(None));
    } else if last.contains("fail") {
        for word in 0..6 {
            steps.push(Step::Token(format!("partial{word} ")));
            steps.push(Step::Pause(Duration::from_millis(80)));
        }
        steps.push(Step::Fail(AiError::Network("connection reset".to_string())));
    } else {
        let count = if last.contains("slow") { 200 } else { 12 };
        for word in 0..count {
            steps.push(Step::Token(format!("word{word} ")));
            steps.push(Step::Pause(Duration::from_millis(120)));
        }
        steps.push(Step::End(None));
    }
    let provider = FakeProvider::new();
    provider.push(Script::new(steps));
    Some(Arc::new(provider))
}

#[cfg(debug_assertions)]
const MARKDOWN_SAMPLE: &str = "## Plan\n\nThe **login** fix has *three* parts:\n\n- check `verify_password`\n- add a test\n- update the [notes](https://example.com)\n\n| File | Change |\n| --- | --- |\n| login.rs | refuse empty |\n| invoice.py | round totals |\n\n```rust\nif password.is_empty() {\n    return None;\n}\n```\n\n```\nno language here\n```\n\nDone.\n";

/// Questions the project chat offers before anything has been asked: a label and the question it sends. The agent
/// answers them with its tools, so what it says is cited from the commits and issues.
const PRESETS: [(&str, &str); 2] = [
    (
        "What changed this week",
        "What changed in this project in the last 7 days? Group the commits by the issues they touch, list the issues that \
moved between statuses or were created, and say what is still open and due soon. Cite the commits and issues.",
    ),
    (
        "What is at risk",
        "What is at risk in this project right now? Look for overdue or soon-due issues with no commits about them, \
started work that has not moved in a week, and issues that say they wait on others that have gone quiet. Cite the issues.",
    ),
];

/// The chat's panel, which the chat window fills. Nothing is drawn when the chat isn't open.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.chat_dialog.as_ref()?;
    let mut body = div().flex().flex_col().gap(theme.spacing.md);
    if dialog.messages.is_empty() && dialog.active.is_none() {
        body = body.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child(if dialog.whole_project {
                    "Ask about this project's codebase. Replies use the code that matches your question from the connected repository."
                } else {
                    "Ask about this task. Replies can use the task's details and, if you turn it on, the repository's code."
                }),
        );
    }
    for message in &dialog.messages {
        body = body.child(super::chat_render::message(
            dialog, message, window, theme, cx,
        ));
    }

    let busy = dialog.active.is_some();
    let model_row = super::chat_render::model_row(dialog, theme, cx);
    let composer = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.sm)
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(edit_box(&dialog.draft, "Message", true, theme)),
        )
        .child(if busy {
            button("chat-stop", "Stop", false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop_chat_reply(cx)))
                .into_any_element()
        } else {
            button("chat-send", "Send", true, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send_chat_message(cx)))
                .into_any_element()
        });

    let mut panel = div()
        .id("chat-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .size_full()
        .p(theme.spacing.lg)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(model_row)
        .child(
            div()
                .id("chat-messages")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&view.chat_scroll)
                .child(body),
        );
    if let Some(notice) = dialog.model_notice.clone() {
        panel = panel.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(notice),
        );
    }
    if let Some(error) = dialog.error.clone() {
        panel = panel.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(error),
        );
    }
    // Ready-made questions for the whole project, always within reach while nothing is being answered.
    if dialog.whole_project && !busy {
        let mut row = div().flex().flex_row().flex_wrap().gap(theme.spacing.sm);
        for (index, (label, question)) in PRESETS.iter().enumerate() {
            row = row.child(
                button(("chat-preset", index), label, false, theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_preset(question, cx)),
                ),
            );
        }
        panel = panel.child(row);
    }
    panel = panel.child(composer).child(
        div().flex().flex_row().justify_end().child(
            button("chat-done", "Done", false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_chat(cx))),
        ),
    );

    Some(panel.into_any_element())
}

#[cfg(test)]
mod source_tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{Source, sources_from_json, sources_to_json};

    #[test]
    fn sources_read_back_as_they_were_saved_and_junk_gives_none() {
        let sources = vec![Source {
            kind: "code".into(),
            reference: "src/auth.rs".into(),
            label: "fn verify".into(),
            start_line: 3,
            end_line: 9,
        }];
        assert_eq!(sources_from_json(&sources_to_json(&sources)), sources);
        assert!(sources_from_json("not json").is_empty());
    }
}
