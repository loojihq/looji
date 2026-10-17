# looji

**A desktop board for planning and tracking work, with AI planning, task chat, repository awareness, and PDF reports.**

[![CI](https://github.com/loojihq/looji/actions/workflows/ci.yml/badge.svg)](https://github.com/loojihq/looji/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/loojihq/looji?display_name=tag&sort=semver)](https://github.com/loojihq/looji/releases/latest)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078D4)
![Rust](https://img.shields.io/badge/Rust-2024%20edition-DEA584?logo=rust&logoColor=white)
![Storage](https://img.shields.io/badge/storage-SQLite-003B57?logo=sqlite&logoColor=white)

looji keeps your workspaces, projects, and tasks on a kanban board. It can plan a project with AI, chat about a single task
with the code it touches, and export a project as a PDF report. It runs on your machine and keeps your data in one folder.

- **Boards that stay fast.** Drag cards between columns and reorder them. A board with 1,000 tasks still draws smoothly.
- **Planning with AI.** Turn a goal into a spec, user stories, and a backlog that you review before publishing.
- **Task chat with repository context.** Link a project to a git repository, and chat can use the code that matches a task.
- **Reports.** Export a project as a PDF with a summary and every task grouped by status.
- **Safe updates.** New versions are downloaded, checked against a published SHA-256 checksum, and installed in one click.
  Your database is backed up before any upgrade changes it.

**New here?** Read the [user guide](docs/user-guide.md). It starts with a first task you can finish in a few minutes.

## Install

Download the installer, `looji-<version>-windows-x64-setup.exe`, from the [latest release](https://github.com/loojihq/looji/releases/latest)
and run it. It installs for your own account and doesn't need administrator rights.

Releases are not yet code-signed, so Windows may show an "unknown publisher" warning. Choose **More info**, then **Run
anyway**. Each release lists a `SHA256SUMS.txt` so you can check your download.

## Development

Requirements: Windows 10 or 11, and a recent stable Rust toolchain.

```
cargo run --bin looji                              # run a debug build, with a console
cargo test --workspace                             # run the test suite
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Every pull request runs the last three in [CI](.github/workflows/ci.yml).

### Architecture

looji is a Cargo workspace. The rules live apart from the interface, so they can be tested without a window or a database.

| Crate | Responsibility |
| --- | --- |
| [`looji-domain`](crates/domain) | Types and rules. No GPUI or SQLite. |
| [`looji-persistence`](crates/persistence) | The SQLite store, migrations, pre-upgrade backups, and the repository index cache. |
| [`looji-symbols`](crates/symbols) | Reads functions, types, and modules from source files with tree-sitter. |
| [`looji-ai`](crates/ai) | A provider-agnostic chat client, planning, and task check and review notes. |
| [`looji-report`](crates/report) | Lays out a project report and writes it as a PDF. |
| [`looji-updates`](crates/updates) | Checks for a new release, downloads it, verifies it, and replaces the program. |
| [`looji-ui`](crates/ui) | The GPUI interface and the `looji` program. |

### Development switches

Debug builds read these environment variables, so features can be checked without a real account or a real release.

| Variable | Effect |
| --- | --- |
| `LOOJI_DATA_DIR` | Use this folder for the database and icons instead of `%APPDATA%\looji`. |
| `LOOJI_FAKE_AI=1` | Answer chat with a scripted provider. A message containing `slow`, `fail`, or `markdown` gets a long, broken, or markdown reply. |
| `LOOJI_UPDATE_URL` | Ask this address for the latest release instead of GitHub. |
| `LOOJI_UPDATE_NOW=1` | Skip the once-a-day schedule for the update check. |
| `LOOJI_FPS=1` | Draw continuously and log frames per second, in any build. |

## Releasing

1. Set the version in `Cargo.toml` under `[workspace.package]`.
2. Tag it and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.

[`release.yml`](.github/workflows/release.yml) builds the program and the installer, signs them if a certificate is
configured, writes `SHA256SUMS.txt`, and publishes the release. Signing uses two repository secrets:
`WINDOWS_SIGNING_CERT_BASE64` (a base64-encoded `.pfx`) and `WINDOWS_SIGNING_CERT_PASSWORD`. Without them the release is
published unsigned, and its run summary says so. To rehearse without publishing, run the workflow by hand with **dry run**
turned on.

## Documentation

- [User guide](docs/user-guide.md): the board, planning, chat, repositories, reports, updates, and troubleshooting.
