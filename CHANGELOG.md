# Changelog

All notable changes to this project will be documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.1.0] — 2026-05-05

Initial public release.

### Added
- **Codex backend** with multi-profile switching backed by `~/.config/aiwitch/profiles.toml`.
- **`add`** — register a Codex profile, optionally provisioning its `CODEX_HOME` and login mode.
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

[v0.1.0]: https://github.com/baedonguri/aiwitch/releases/tag/v0.1.0
