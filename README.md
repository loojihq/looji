# looji

A desktop board for planning and tracking work, with AI planning, task chat, repository awareness, and PDF reports. It is
a Rust workspace with a GPUI interface and a SQLite database.

**Using looji:** read the [user guide](docs/user-guide.md).

## Building

```
cargo run --bin looji          # a debug build, with a console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Windows is the supported platform. CI runs the last three on every pull request (`.github/workflows/ci.yml`).

## The crates

| Crate | What it holds |
| --- | --- |
| `looji-domain` | Plain types and rules. No GPUI or SQLite. |
| `looji-persistence` | The SQLite store, migrations, backups before an upgrade, and the repository index cache. |
| `looji-symbols` | Reads functions, types, and modules out of source files with tree-sitter. |
| `looji-ai` | A provider-agnostic chat client, planning, and the task check and review notes. |
| `looji-report` | Lays out a project report and writes it as a PDF. |
| `looji-updates` | Checks GitHub for a new release, downloads it, verifies it, and swaps it in. |
| `looji-ui` | The GPUI interface and the `looji` program. |

## Switches for development

Debug builds read these environment variables. They exist so features can be checked without a real account or a real
release.

| Variable | Effect |
| --- | --- |
| `LOOJI_DATA_DIR` | Use this folder for the database and icons, instead of `%APPDATA%\looji`. |
| `LOOJI_FAKE_AI=1` | Answer chat with a scripted provider. A message containing `slow`, `fail`, or `markdown` gets a long, a broken, or a markdown reply. |
| `LOOJI_UPDATE_URL` | Ask this address for the latest release instead of GitHub. |
| `LOOJI_UPDATE_NOW=1` | Skip the once-a-day schedule for the update check. |
| `LOOJI_FPS=1` | Draw continuously and log frames per second, in any build. |

## Releasing

1. Set the version in `Cargo.toml` under `[workspace.package]`.
2. Tag it: `git tag v0.2.0 && git push origin v0.2.0`.

`.github/workflows/release.yml` builds the program and the installer, signs both, writes `SHA256SUMS.txt`, and publishes the
release. It needs two repository secrets, `WINDOWS_SIGNING_CERT_BASE64` (a base64 `.pfx`) and `WINDOWS_SIGNING_CERT_PASSWORD`.
Without them it stops at its first step. It never publishes an unsigned release. To rehearse without publishing, run the
workflow by hand with **dry run** on.
