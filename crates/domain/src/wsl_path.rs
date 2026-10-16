//! Recognises paths into a WSL distribution, such as `\\wsl.localhost\Ubuntu\home\dana\repo`, and splits them into
//! the distribution's name and the path inside it.
//!
//! Only the text is handled here. Whether the distribution exists or is running is asked of WSL in persistence.

/// A path inside a WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslPath {
    /// The distribution's name as written in the path. Names are matched without regard to case.
    pub distro: String,
    /// The path inside the distribution, as Linux sees it: `/home/dana/repo`. `/` for the distribution's root.
    pub linux_path: String,
}

/// Both spellings Windows uses for WSL shares. `wsl$` is the older one.
const PREFIXES: [&str; 2] = [r"\\wsl.localhost\", r"\\wsl$\"];

/// Parses a WSL path. Forward slashes are accepted too, and the prefix is matched without regard to case. Returns
/// `None` for any other path, including a WSL prefix with no distribution name after it.
pub fn parse_wsl_path(text: &str) -> Option<WslPath> {
    let normalised = text.trim().replace('/', r"\");
    let rest = PREFIXES
        .iter()
        .find_map(|prefix| strip_prefix_ignore_case(&normalised, prefix))?;

    let mut parts = rest.split('\\').filter(|part| !part.is_empty());
    let distro = parts.next()?.to_string();

    let mut linux_path = String::new();
    for part in parts {
        linux_path.push('/');
        linux_path.push_str(part);
    }
    if linux_path.is_empty() {
        linux_path.push('/');
    }
    Some(WslPath { distro, linux_path })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::wsl_path::{WslPath, parse_wsl_path};

    fn wsl(distro: &str, linux_path: &str) -> Option<WslPath> {
        Some(WslPath {
            distro: distro.to_string(),
            linux_path: linux_path.to_string(),
        })
    }

    #[test]
    fn a_wsl_localhost_path_splits_into_distro_and_linux_path() {
        assert_eq!(
            parse_wsl_path(r"\\wsl.localhost\Ubuntu\home\dana\repo"),
            wsl("Ubuntu", "/home/dana/repo")
        );
    }

    #[test]
    fn the_older_wsl_dollar_prefix_is_accepted() {
        assert_eq!(
            parse_wsl_path(r"\\wsl$\openSUSE-Tumbleweed\tmp\repo"),
            wsl("openSUSE-Tumbleweed", "/tmp/repo")
        );
    }

    #[test]
    fn forward_slashes_and_the_prefix_case_do_not_matter() {
        assert_eq!(
            parse_wsl_path("//WSL.LOCALHOST/Ubuntu/home/dana/"),
            wsl("Ubuntu", "/home/dana")
        );
    }

    #[test]
    fn trailing_and_doubled_separators_are_dropped() {
        assert_eq!(
            parse_wsl_path(r"  \\wsl.localhost\Ubuntu\home\\dana\repo\  "),
            wsl("Ubuntu", "/home/dana/repo")
        );
    }

    #[test]
    fn the_distribution_root_is_the_linux_root() {
        assert_eq!(
            parse_wsl_path(r"\\wsl.localhost\Ubuntu"),
            wsl("Ubuntu", "/")
        );
    }

    #[test]
    fn a_prefix_with_no_distribution_is_not_a_wsl_path() {
        assert_eq!(parse_wsl_path(r"\\wsl.localhost\"), None);
    }

    #[test]
    fn windows_and_other_network_paths_are_not_wsl_paths() {
        assert_eq!(parse_wsl_path(r"C:\dev\looji"), None);
        assert_eq!(parse_wsl_path(r"\\server\share\repo"), None);
        assert_eq!(parse_wsl_path(r"\\wsl.localhost.evil\Ubuntu\repo"), None);
        assert_eq!(parse_wsl_path(""), None);
    }

    #[test]
    fn text_that_is_not_ascii_does_not_panic() {
        assert_eq!(parse_wsl_path(r"\\wsl.localho€st\x"), None);
    }
}
