//! WSL repository folders: `\\wsl.localhost\<distro>\…` paths, asked about through `wsl.exe`.
//!
//! Windows reaches a running distribution through its UNC path, so that's read directly. Touching the path of a
//! stopped distribution starts it without asking, so the state is asked first, and a stopped distribution is only
//! reported. Starting it is the user's choice ([`start_distro`]). When a direct read fails, the folder is probed
//! through the distribution with `wsl --exec`, which answers as Linux sees the path.
//!
//! `wsl.exe` writes its list output as UTF-16. Everything else it prints is UTF-8, so both are decoded here.

use std::{
    io,
    path::Path,
    process::{Command, Output},
};

use looji_domain::{PathCheck, WslPath};

use crate::repo_folder::check_local_folder;

/// Stops a console window from flashing up when `wsl.exe` runs from the GUI.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Prints `git`, `dir`, `file`, or `missing` for the path in `$1`, as Linux sees it. `.git` may be a folder or a file.
const PROBE: &str = r#"if [ -d "$1/.git" ] || [ -f "$1/.git" ]; then echo git; elif [ -d "$1" ]; then echo dir; elif [ -e "$1" ]; then echo file; else echo missing; fi"#;

/// Checks a folder inside a WSL distribution. A stopped or missing distribution is reported before the folder is
/// touched.
pub(crate) fn check_folder(path: &Path, wsl_path: &WslPath) -> Result<(), PathCheck> {
    let distro = resolve_running(&wsl_path.distro)?;
    match check_local_folder(path) {
        Err(PathCheck::Unreadable(_)) => probe_through_distro(&distro, &wsl_path.linux_path),
        other => other,
    }
}

/// The distribution's name as WSL spells it, if it's installed and running. Stopped and missing distributions are
/// errors, and nothing is started.
pub(crate) fn resolve_running(distro: &str) -> Result<String, PathCheck> {
    let installed = list_names(&["--list", "--quiet"])?;
    let Some(name) = find_name(&installed, distro) else {
        return Err(PathCheck::DistroNotFound {
            distro: distro.to_string(),
        });
    };
    let running = list_names(&["--list", "--running", "--quiet"])?;
    if find_name(&running, name).is_none() {
        return Err(PathCheck::DistroStopped {
            distro: name.clone(),
        });
    }
    Ok(name.clone())
}

/// Reads a file through a running distribution, for when Windows can't read it directly.
pub fn read_through_distro(distro: &str, linux_path: &str) -> io::Result<Vec<u8>> {
    let distro = resolve_running(distro).map_err(|reason| io::Error::other(reason.to_string()))?;
    let output = wsl(&[
        "--distribution",
        distro.as_str(),
        "--exec",
        "cat",
        linux_path,
    ])?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(io::Error::other(message(&output.stderr, &output.stdout)))
    }
}

/// Starts a distribution the user asked to start. On failure, the message is WSL's own.
pub fn start_distro(distro: &str) -> Result<(), String> {
    let output = wsl(&["--distribution", distro, "--exec", "sh", "-c", "exit 0"])
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(message(&output.stderr, &output.stdout))
    }
}

/// Probes a Linux path inside the distribution, which must be running. Public so tests can use it directly.
pub fn probe_through_distro(distro: &str, linux_path: &str) -> Result<(), PathCheck> {
    let output = wsl(&[
        "--distribution",
        distro,
        "--exec",
        "sh",
        "-c",
        PROBE,
        "sh",
        linux_path,
    ])
    .map_err(|err| PathCheck::Unreadable(err.to_string()))?;
    if !output.status.success() {
        return Err(PathCheck::Unreadable(message(
            &output.stderr,
            &output.stdout,
        )));
    }
    probe_result(&String::from_utf8_lossy(&output.stdout))
}

/// The check for the word [`PROBE`] printed.
fn probe_result(word: &str) -> Result<(), PathCheck> {
    match word.trim() {
        "git" => Ok(()),
        "dir" => Err(PathCheck::NotAGitRepository),
        "file" => Err(PathCheck::NotADirectory),
        "missing" => Err(PathCheck::Missing),
        other => Err(PathCheck::Unreadable(format!(
            "the distribution answered {other:?}"
        ))),
    }
}

fn list_names(args: &[&str]) -> Result<Vec<String>, PathCheck> {
    let output = wsl(args).map_err(|err| PathCheck::WslUnavailable(err.to_string()))?;
    if !output.status.success() {
        return Err(PathCheck::WslUnavailable(message(
            &output.stderr,
            &output.stdout,
        )));
    }
    Ok(parse_names(&decode_wsl_text(&output.stdout)))
}

fn wsl(args: &[&str]) -> io::Result<Output> {
    let mut command = Command::new("wsl.exe");
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

/// Decodes text from `wsl.exe`. Its list output is UTF-16LE, with or without a byte-order mark, and anything else
/// is taken as UTF-8.
fn decode_wsl_text(bytes: &[u8]) -> String {
    let utf16 = bytes.starts_with(&[0xFF, 0xFE]) || bytes.get(1) == Some(&0);
    if !utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let body = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    let (pairs, _) = body.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

/// One distribution name per line. Lines with spaces are WSL's messages, such as "no distributions installed",
/// and not names. Distribution names don't contain spaces.
fn parse_names(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
        .map(str::to_owned)
        .collect()
}

/// Names are matched without regard to case, as WSL does.
fn find_name<'a>(names: &'a [String], distro: &str) -> Option<&'a String> {
    names.iter().find(|name| name.eq_ignore_ascii_case(distro))
}

/// What WSL said on failure, or a fallback when it said nothing.
fn message(stderr: &[u8], stdout: &[u8]) -> String {
    let text = decode_wsl_text(stderr);
    let text = if text.trim().is_empty() {
        decode_wsl_text(stdout)
    } else {
        text
    };
    let text = text.trim();
    if text.is_empty() {
        "WSL didn't give a reason".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::PathCheck;

    use crate::wsl::{decode_wsl_text, find_name, parse_names, probe_result};

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut bytes = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn list_output_is_utf16_with_or_without_a_byte_order_mark() {
        assert_eq!(decode_wsl_text(&utf16le("Ubuntu\r\n", true)), "Ubuntu\r\n");
        assert_eq!(decode_wsl_text(&utf16le("Ubuntu\r\n", false)), "Ubuntu\r\n");
    }

    #[test]
    fn other_output_is_utf8() {
        assert_eq!(decode_wsl_text("sh: café\n".as_bytes()), "sh: café\n");
    }

    #[test]
    fn names_are_read_one_per_line_and_wsl_messages_are_skipped() {
        let text = "Ubuntu\r\nopenSUSE-Tumbleweed\r\n\r\nWindows Subsystem for Linux has no installed distributions.\r\n";
        assert_eq!(
            parse_names(text),
            vec!["Ubuntu".to_string(), "openSUSE-Tumbleweed".to_string()]
        );
    }

    #[test]
    fn a_distro_is_found_without_regard_to_case() {
        let names = vec!["openSUSE-Tumbleweed".to_string()];
        assert_eq!(
            find_name(&names, "OPENSUSE-tumbleweed").map(String::as_str),
            Some("openSUSE-Tumbleweed")
        );
        assert_eq!(find_name(&names, "Ubuntu"), None);
    }

    #[test]
    fn the_probe_answers_map_to_checks() {
        assert_eq!(probe_result("git\n"), Ok(()));
        assert_eq!(probe_result("dir"), Err(PathCheck::NotAGitRepository));
        assert_eq!(probe_result("file"), Err(PathCheck::NotADirectory));
        assert_eq!(probe_result("missing"), Err(PathCheck::Missing));
        assert!(matches!(
            probe_result("something else"),
            Err(PathCheck::Unreadable(_))
        ));
    }
}
