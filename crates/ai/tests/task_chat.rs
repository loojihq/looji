//! What a task chat sends with each message: the task, the code, and the conversation so far, trimmed to a budget.

use looji_ai::{
    CancellationToken, ChatMessage,
    chat::{TaskBrief, build_chat_request},
    fake::{FakeProvider, Script},
    generation::complete_with,
};
use looji_domain::Hunk;

fn brief() -> TaskBrief<'static> {
    TaskBrief {
        title: "Fix password verification in login",
        description: "Empty passwords must be refused.",
        status: "In progress",
        whole_project: false,
    }
}

fn login_hunk() -> Hunk {
    Hunk {
        path: "src/auth/login.rs".to_string(),
        start_line: 5,
        end_line: 6,
        score: 4,
        text: "pub fn login(user: &str, password: &str) -> Option<Session> {".to_string(),
    }
}

#[tokio::test]
async fn a_follow_up_is_sent_with_the_earlier_reply_it_refers_to() {
    let history = vec![
        ChatMessage::user("Does login check the password?"),
        ChatMessage::assistant("Yes, verify_password runs before a session is made."),
        ChatMessage::user("And what does it do with an empty one?"),
    ];
    let (request, dropped) =
        build_chat_request("claude-sonnet-5-5", brief(), &history, &[], 10_000);
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["It refuses it."]));

    let reply = complete_with(&provider, request, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(reply, "It refuses it.");
    assert_eq!(dropped, 0);
    let sent = &provider.requests()[0];
    assert_eq!(sent.messages.len(), 3);
    assert_eq!(
        sent.messages[1].content, "Yes, verify_password runs before a session is made.",
        "the earlier reply is sent back, so the follow-up can refer to it"
    );
    assert_eq!(
        sent.messages[2].content,
        "And what does it do with an empty one?"
    );
}

#[test]
fn the_system_prompt_carries_the_task_and_the_code_when_it_is_on() {
    let (with_code, _) = build_chat_request(
        "claude-sonnet-5-5",
        brief(),
        &[ChatMessage::user("Why?")],
        &[login_hunk()],
        10_000,
    );
    let (without_code, _) = build_chat_request(
        "claude-sonnet-5-5",
        brief(),
        &[ChatMessage::user("Why?")],
        &[],
        10_000,
    );

    let system = with_code.system.unwrap();
    assert!(system.contains("Task: Fix password verification in login"));
    assert!(system.contains("Status: In progress"));
    assert!(system.contains("Empty passwords must be refused."));
    assert!(system.contains("### src/auth/login.rs:5-6"));
    let plain = without_code.system.unwrap();
    assert!(!plain.contains("Relevant code"), "{plain}");
}

#[test]
fn a_long_conversation_drops_the_oldest_messages_and_keeps_the_newest() {
    let long = "x".repeat(400); // about 100 tokens each
    let mut history = Vec::new();
    for turn in 0..10 {
        history.push(ChatMessage::user(format!("question {turn} {long}")));
        history.push(ChatMessage::assistant(format!("answer {turn} {long}")));
    }

    let (request, dropped) = build_chat_request("claude-sonnet-5-5", brief(), &history, &[], 350);

    assert!(dropped > 0, "some history is left out");
    assert_eq!(request.messages.len(), history.len() - dropped);
    assert_eq!(
        request.messages.last().unwrap().content,
        history.last().unwrap().content,
        "the newest message is kept"
    );
    assert_eq!(
        request.messages[0].content, history[dropped].content,
        "what is kept starts at the first message that fits, with nothing older than it"
    );
    let kept_cost: usize = request
        .messages
        .iter()
        .map(|message| looji_domain::approximate_tokens(&message.content))
        .sum();
    assert!(
        kept_cost <= 350,
        "{kept_cost} tokens kept over a budget of 350"
    );
}

#[test]
fn a_chat_about_the_whole_project_is_not_framed_as_one_task() {
    let brief = TaskBrief {
        title: "Website redesign",
        description: "The marketing site and docs.",
        status: "",
        whole_project: true,
    };
    let history = [ChatMessage::user("Where is the login handled?")];
    let (request, left_out) = build_chat_request("model", brief, &history, &[login_hunk()], 10_000);

    let system = request.system.expect("a system prompt");
    assert_eq!(left_out, 0);
    assert!(system.contains("whole project"));
    assert!(system.contains("Project: Website redesign"));
    assert!(!system.contains("Status:"));
    assert!(system.contains("src/auth/login.rs"));
}
