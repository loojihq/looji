//! An optional test run for a review. It runs the project's own code, so it only happens when a project turns it on,
//! and only for one of a short list of known commands. The result goes to the reviewer as evidence: whether the tests
//! passed, which failed, and the end of the output.

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// The commands a project can choose. A fixed list, so settings never hold a command that something else wrote.
pub const PRESETS: [&str; 5] = [
    "cargo test",
    "npm test",
    "pytest",
    "go test ./...",
    "dotnet test",
];

/// How long a run may take before it is stopped.
pub const TIMEOUT: Duration = Duration::from_secs(300);
/// The most output kept, from the end, since that is where the summary is.
const TAIL_CHARS: usize = 3_000;
/// The most failing test names listed.
const MAX_FAILING: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed { code: Option<i32> },
    TimedOut,
    CouldNotStart(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRun {
    pub command: String,
    pub outcome: Outcome,
    pub failing: Vec<String>,
    /// The end of the output.
    pub tail: String,
}

/// Whether `command` is one of the known commands.
pub fn is_preset(command: &str) -> bool {
    PRESETS.contains(&command)
}

/// Runs a known command in `root` and waits up to `timeout`. A command that isn't on the list is not run.
pub fn run(root: &Path, command: &str, timeout: Duration) -> TestRun {
    let result = |outcome: Outcome, output: &str| TestRun {
        command: command.to_string(),
        failing: failing_tests(output),
        tail: tail(output),
        outcome,
    };
    if !is_preset(command) {
        return result(
            Outcome::CouldNotStart("that is not a command Looji runs".to_string()),
            "",
        );
    }
    let mut process = if cfg!(windows) {
        let mut process = Command::new("cmd");
        process.args(["/C", command]);
        process
    } else {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    let spawned = process
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => return result(Outcome::CouldNotStart(err.to_string()), ""),
    };

    // Both streams are read on their own threads so a full pipe never stalls the process.
    let (sender, receiver) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let sender = sender.clone();
        readers.push(thread::spawn(move || {
            let mut text = String::new();
            let mut bytes = Vec::new();
            let _ = out.read_to_end(&mut bytes);
            text.push_str(&String::from_utf8_lossy(&bytes));
            let _ = sender.send(text);
        }));
    }
    if let Some(mut err) = child.stderr.take() {
        let sender = sender.clone();
        readers.push(thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = err.read_to_end(&mut bytes);
            let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
        }));
    }
    drop(sender);

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                stop(&mut child);
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(err) => return result(Outcome::CouldNotStart(err.to_string()), ""),
        }
    };
    // Readers finish when the process closes its pipes, which a stop does too.
    for reader in readers {
        let _ = reader.join();
    }
    let output: String = receiver.try_iter().collect::<Vec<_>>().join("\n");
    match status {
        Some(status) if status.success() => result(Outcome::Passed, &output),
        Some(status) => result(
            Outcome::Failed {
                code: status.code(),
            },
            &output,
        ),
        None => result(Outcome::TimedOut, &output),
    }
}

/// Stops a process and what it started. Killing only the shell would leave the tests running.
fn stop(child: &mut std::process::Child) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The names of tests that failed, read from the output of the known runners.
pub fn failing_tests(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let name = if let Some(rest) = line.strip_prefix("test ") {
            // cargo: `test module::name ... FAILED`
            rest.strip_suffix(" ... FAILED").map(str::to_string)
        } else if let Some(rest) = line.strip_prefix("FAILED ") {
            // pytest: `FAILED tests/test_x.py::test_y - message`
            Some(rest.split(" - ").next().unwrap_or(rest).to_string())
        } else if let Some(rest) = line.strip_prefix("--- FAIL: ") {
            // go: `--- FAIL: TestName (0.00s)`
            Some(rest.split(" (").next().unwrap_or(rest).to_string())
        } else {
            // dotnet: `Failed Namespace.Test [12 ms]`
            line.strip_prefix("Failed ")
                .map(|rest| rest.split(" [").next().unwrap_or(rest).to_string())
        };
        if let Some(name) = name.filter(|name| !name.is_empty() && !names.contains(name)) {
            names.push(name);
            if names.len() == MAX_FAILING {
                break;
            }
        }
    }
    names
}

fn tail(output: &str) -> String {
    let trimmed = output.trim_end();
    let count = trimmed.chars().count();
    if count <= TAIL_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - TAIL_CHARS).collect()
}

impl TestRun {
    /// What the run says in one line.
    pub fn headline(&self) -> String {
        match &self.outcome {
            Outcome::Passed => format!("`{}` passed.", self.command),
            Outcome::Failed { code: Some(code) } => {
                format!("`{}` failed (exit code {code}).", self.command)
            }
            Outcome::Failed { code: None } => format!("`{}` failed.", self.command),
            Outcome::TimedOut => format!(
                "`{}` was stopped after {} seconds.",
                self.command,
                TIMEOUT.as_secs()
            ),
            Outcome::CouldNotStart(reason) => {
                format!("`{}` could not be started: {reason}.", self.command)
            }
        }
    }

    /// The run as the reviewer reads it.
    pub fn report(&self) -> String {
        let mut text = self.headline();
        if !self.failing.is_empty() {
            text.push_str("\nFailing tests:\n");
            for name in &self.failing {
                text.push_str(&format!("- {name}\n"));
            }
        }
        if !self.tail.is_empty() && self.outcome != Outcome::Passed {
            text.push_str("\nEnd of the output:\n```\n");
            text.push_str(&self.tail);
            text.push_str("\n```");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_commands_run() {
        let run = run(Path::new("."), "rm -rf /", Duration::from_secs(1));
        assert!(matches!(run.outcome, Outcome::CouldNotStart(_)));
        assert!(!is_preset("cargo test && echo hi"));
        assert!(is_preset("cargo test"));
    }

    #[test]
    fn a_run_past_its_time_is_stopped() {
        let dir = std::env::temp_dir();
        let started = Instant::now();
        let run = run(&dir, "cargo test", Duration::ZERO);
        assert_eq!(run.outcome, Outcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(run.headline().contains("was stopped after"));
    }

    #[test]
    fn cargo_pytest_go_and_dotnet_failures_are_found() {
        let output = "running 2 tests\ntest a::b ... ok\ntest a::c ... FAILED\n\
FAILED tests/test_x.py::test_y - assert 1 == 2\n--- FAIL: TestLogin (0.01s)\n  Failed Shop.Cart.Totals [12 ms]\n";
        assert_eq!(
            failing_tests(output),
            vec![
                "a::c",
                "tests/test_x.py::test_y",
                "TestLogin",
                "Shop.Cart.Totals"
            ]
        );
    }

    #[test]
    fn the_same_failure_is_listed_once_and_the_list_is_capped() {
        assert_eq!(
            failing_tests("test a ... FAILED\ntest a ... FAILED\n"),
            vec!["a"]
        );
        let many: String = (0..40).map(|n| format!("test t{n} ... FAILED\n")).collect();
        assert_eq!(failing_tests(&many).len(), MAX_FAILING);
    }

    #[test]
    fn a_report_lists_failures_and_the_end_of_the_output() {
        let run = TestRun {
            command: "cargo test".into(),
            outcome: Outcome::Failed { code: Some(101) },
            failing: vec!["a::c".into()],
            tail: "test result: FAILED. 1 passed; 1 failed".into(),
        };
        let report = run.report();
        assert!(report.contains("failed (exit code 101)"));
        assert!(report.contains("- a::c"));
        assert!(report.contains("1 passed; 1 failed"));
        let passed = TestRun {
            outcome: Outcome::Passed,
            failing: Vec::new(),
            ..run
        };
        assert_eq!(passed.report(), "`cargo test` passed.");
    }

    #[test]
    fn a_long_output_keeps_its_end() {
        let output = format!("{}END", "x".repeat(10_000));
        let kept = tail(&output);
        assert_eq!(kept.chars().count(), TAIL_CHARS);
        assert!(kept.ends_with("END"));
    }
}
