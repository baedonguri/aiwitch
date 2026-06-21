# Changelog

All notable changes to this project will be documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **macOS Keychain support for Claude profiles.** `aiwitch doctor` now reads the per-profile Keychain entry (`Claude Code-credentials-<sha256(CLAUDE_CONFIG_DIR)[:8]>`) to report **plan and token expiry** on macOS, distinguishing "not logged in" (no entry) from "keychain access denied". `aiwitch remove <p> --purge` also deletes that Keychain entry, after a read-back verifies it parses as a Claude OAuth blob. All Keychain naming/coupling is isolated in `backend::claude::keychain` and every access is best-effort.

### Changed
- `aiwitch list` is unchanged on macOS (stays Keychain-free for speed; Claude rows still show `-`); enrichment is `doctor`-only.

### Security
- The default `~/.claude` (main-account) Keychain entry is never read or deleted: a path-equality guard (`resolved == $HOME/.claude`) skips it, and `--purge` performs a read-before-delete identity check so a hash collision or a changed naming scheme can never delete an unrelated entry.

## [v0.3.0] — 2026-05-10

### Added
- **`aiwitch doctor`** — diagnose profile health. Walks each profile and checks home dir existence, login state, and token expiry; globally checks provider CLIs on `PATH` and `AIWITCH_CURRENT` validity. Exits non-zero on `[err]` so it can gate CI.
- **`aiwitch remove <profile> [--purge]`** — drop a profile entry from `profiles.toml`. Without `--purge` the per-profile home directory is preserved so credentials stay recoverable. With `--purge` the directory is deleted **only** when it matches the default `~/.codex-<name>` / `~/.claude-<name>` pattern; custom paths and symlinks are rejected with an `rm -rf` hint so the tool never deletes a directory it didn't create. Emits a stderr warning when `AIWITCH_CURRENT` still points at the removed name.
- **`aiwitch rename <old> <new>`** — rename a profile entry. When `home_dir` uses the default pattern, the directory on disk is also moved to `~/.codex-<new>` / `~/.claude-<new>`; custom paths are left untouched. Cross-filesystem renames (`EXDEV`) are rejected with an actionable hint, and a symlinked `home_dir` triggers a stderr warning since `fs::rename` moves the link rather than its target. If the target directory already exists, the command aborts before any change.

### Changed
- All writes to `profiles.toml` now go through a shared `store::write_atomic` helper (sibling tmpfile + atomic rename in the same parent directory). `aiwitch add` was migrated onto the same helper for consistency.

## [v0.2.0] — 2026-05-08

### Added
- **Claude Code backend.** `aiwitch add claude <profile>` provisions an isolated `CLAUDE_CONFIG_DIR` (defaults to `~/.claude-<profile>`, mode `0700`) and spawns the `claude` TUI for interactive `/login`. `aiwitch list` shows `provider = claude`.
- **Best-effort Claude metadata.** `aiwitch list` reads `<CLAUDE_CONFIG_DIR>/.credentials.json` when present (Linux/Windows) and surfaces `email`, `subscription_type`, and `expires_at` (ms or seconds). On macOS, Claude Code stores credentials in the system Keychain, so these columns will typically render as `-` even after a successful login — that's expected, not a bug.

### Changed
- `aiwitch add` now derives the printed `next:` hint and `--print-env` snippet from each backend's `env_exports`, so Claude profiles emit `CLAUDE_CONFIG_DIR=...` instead of `CODEX_HOME=...`.
- `--auth` is rejected for `claude` (and `aiwitch login <claude-profile> --api-key` errors before stdin is read), since Claude does not have an API-key login flow yet.

## [v0.1.0] — 2026-05-05

Initial public release.

### Added
- **Codex backend** with multi-profile switching backed by `~/.config/aiwitch/profiles.toml`.
- **`add <provider> <profile>`** — register a profile, provision its provider home directory, and run the provider's login flow (defaults to ChatGPT login; pass `--auth api` to log in with an API key from stdin). Provider name is required; today only `codex` is supported.
- Actionable error hints on duplicate profile name, missing/uninstalled provider CLI, login exit failure, and missing piped API key.
- **`list`** — show profiles with email, plan, and token expiry; marks the active profile per backend.
- **`current`** — print the currently active profile name.
- **`env`** — emit `export` (POSIX) or `set -gx` (fish) statements for `eval`/`source` integration.
- **`login`** — ChatGPT login (delegates to `codex`) or API key login that reads the key from stdin, validates and size-limits it, pipes it to `codex` without persisting, and zeros the in-memory buffer (volatile write) on drop.
- **`shell init {zsh|bash|fish}`** — emit a shell function exposing `aiwitch use` and an env-aware `aiwitch add`.
- TOML profile store with `~`-expansion, duplicate-name detection, and strict profile-name validation.
- Lenient JWT timestamp deserializer for Codex `auth.json` parsing.

### Project
- Rust 1.85 / edition 2024 baseline.
- Lint baseline: `rustfmt`, `clippy` with `pedantic` warn + curated allows, `unsafe_code = "deny"` with one localized `allow` for volatile secret zeroing.
- CI on GitHub Actions: `fmt`, `clippy`, matrix `test` (ubuntu, macos), and `doc` jobs. `clippy`/`test`/`doc`/release build run `--locked`; `clippy`/`test`/`doc`/release build run `--all-features`; `doc` enforces `RUSTDOCFLAGS=-D warnings`.

[v0.3.0]: https://github.com/baedonguri/aiwitch/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/baedonguri/aiwitch/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/baedonguri/aiwitch/releases/tag/v0.1.0
