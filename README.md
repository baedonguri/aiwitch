<div align="center">
  <h1>aiwitch</h1>
  <p>
    Switch between multiple AI CLI accounts and profiles in a single shell.
    Today: <a href="https://github.com/openai/codex">Codex</a> and <a href="https://docs.claude.com/en/docs/claude-code">Claude Code</a>. Built so more providers slot in without rewriting the CLI surface.
  </p>
</div>

<div align="center">

[![CI](https://github.com/baedonguri/aiwitch/actions/workflows/ci.yml/badge.svg)](https://github.com/baedonguri/aiwitch/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Homebrew](https://img.shields.io/badge/install-brew-FBB040?logo=homebrew&logoColor=000)](#install)

</div>

______________________________________________________________________

## Table of Contents

- [Quick start](#quick-start)
- [Why](#why)
- [Requirements](#requirements)
- [Key features](#key-features)
- [Install](#install)
- [Shell setup](#shell-setup)
- [How it works](#how-it-works)
- [Usage](#usage)
- [Configuration](#configuration)
- [Roadmap](#roadmap)
- [License](#license)

______________________________________________________________________

## Quick start

```sh
# Install via Homebrew (macOS)
brew install baedonguri/tap/aiwitch

# Enable in-shell switching (add to ~/.zshrc; bash/fish similar — see Shell setup)
eval "$(aiwitch shell init zsh)"

# Add profiles (registers and runs the provider's login flow)
aiwitch add codex personal       # ChatGPT login by default
aiwitch add codex work
aiwitch add claude personal      # spawns `claude`; run `/login` inside the REPL

# Switch the current shell to a profile
aiwitch use personal

# Then run the provider's CLI with that profile's account
codex     # or `claude`
```

> `aiwitch add <provider> <profile>` triggers the provider's login flow for the new profile. Codex supports `--auth api` for API-key login (see [Login](#login)); Claude is interactive-only — log in via `/login` inside the spawned `claude` TUI.

## Why

Running multiple AI CLI accounts — personal, work, an API-key-only sandbox — means juggling `CODEX_HOME` / `CLAUDE_CONFIG_DIR` and shuffling auth files by hand. `aiwitch` keeps each account in its own home directory and switches the active one in your current shell with a single command.

Inspired by `nvm`/`pyenv`-style version switchers, applied to AI CLI accounts.

## Requirements

- macOS or Linux with Rust 1.85+ (Windows is not supported in v0.1.0)
- [Homebrew](https://brew.sh) on macOS, if you want the `brew install` path
- [Codex CLI](https://github.com/openai/codex) and/or [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and on `PATH` (only the providers you intend to use)
- For Codex API-key login: an OpenAI API key on stdin (`$OPENAI_API_KEY` or piped)

## Key features

- **Per-profile provider home** — every profile has an isolated directory (`CODEX_HOME` for Codex, `CLAUDE_CONFIG_DIR` for Claude); switching is a single env-var swap. See [How it works](#how-it-works).
- **[In-shell switching](#shell-setup)** — `aiwitch use <profile>` and `aiwitch add <provider> <profile>` mutate the current shell via a small snippet for zsh, bash, and fish.
- **Two Codex auth modes** — ChatGPT login and API-key login; the latter reads the key from stdin and pipes it straight to `codex` without `aiwitch` persisting it.
- **Claude interactive login** — `aiwitch add claude <profile>` provisions an isolated `CLAUDE_CONFIG_DIR` and spawns the `claude` TUI; users log in via `/login` inside the REPL.
- **TOML config** — a single `~/.config/aiwitch/profiles.toml` with strict name validation and duplicate detection.
- **Backend trait** — adding a new provider is a `Backend` impl, not a CLI rewrite.

## Install

### macOS — Homebrew

```sh
brew install baedonguri/tap/aiwitch
```

After tapping once:

```sh
brew tap baedonguri/tap
brew install aiwitch
```

### From source (macOS / Linux)

Requires Rust 1.85+.

```sh
cargo install --git https://github.com/baedonguri/aiwitch --tag v0.2.0
```

## Shell setup

`aiwitch` switches profiles by exporting environment variables in the **current shell**, so it needs a small shell function.

```sh
# zsh — add to ~/.zshrc
eval "$(aiwitch shell init zsh)"

# bash — add to ~/.bashrc
eval "$(aiwitch shell init bash)"

# fish — add to ~/.config/fish/config.fish
aiwitch shell init fish | source
```

This wires up two extra subcommands:

| Command | What it does |
|---|---|
| `aiwitch use <profile>` | Switch the current shell to an existing profile. |
| `aiwitch add <provider> <profile>` | Add a profile, run its login flow, **and** activate it in the current shell. |

Without `shell init` you can still drive everything manually via `eval "$(aiwitch env <profile>)"`.

## How it works

Each provider picks up its account from a directory pointed to by an environment variable — `CODEX_HOME` for Codex, `CLAUDE_CONFIG_DIR` for Claude Code. `aiwitch` keeps one such directory **per profile** and exposes them as named entries in `~/.config/aiwitch/profiles.toml`.

```
~/.codex-work/             ← CODEX_HOME for codex/"work"
  └── auth.json
~/.codex-personal/         ← CODEX_HOME for codex/"personal"
  └── auth.json
~/.claude-personal/        ← CLAUDE_CONFIG_DIR for claude/"personal"
  └── (managed by Claude Code; on Linux a `.credentials.json` lives here)
```

`aiwitch use <profile>` evaluates to `export <PROVIDER_VAR>=...` and `export AIWITCH_CURRENT=...` (or `set -gx` on fish). The next `codex` / `claude` invocation picks up the right account; `aiwitch current` reads `AIWITCH_CURRENT`. No background daemon, no symlink swapping, no global state to corrupt.

> **macOS note for Claude profiles**: Claude Code stores OAuth credentials in the system Keychain on macOS, not inside `CLAUDE_CONFIG_DIR`. `aiwitch list` will show `email=-`, `plan=-`, `expires=-` for Claude profiles even after a successful login — that's expected, not a bug. On Linux, `CLAUDE_CONFIG_DIR/.credentials.json` is parsed best-effort.

## Usage

### Add a profile

`aiwitch add <provider> <profile>` registers the profile and immediately runs the provider's login flow. The provider is required; supported values are `codex` and `claude`.

```sh
aiwitch add codex personal                                    # Codex ChatGPT login
echo "$OPENAI_API_KEY" | aiwitch add codex work --auth api    # Codex API-key login (stdin)
aiwitch add claude personal                                   # Claude — spawns `claude` TUI; run `/login`
```

| Flag | Meaning |
|---|---|
| `--home <PATH>` | Directory used as the provider home. Optional; defaults to `~/.codex-<profile>` (codex) or `~/.claude-<profile>` (claude). |
| `--auth <chatgpt\|api>` | **Codex only.** Login mode; defaults to `chatgpt`. Rejected with an explicit error for `claude` (Claude is interactive-only). |

> If login fails or is canceled, the profile is still registered. Retry with `aiwitch login <profile>` (add `--api-key` for API-key mode).

### List profiles

```sh
aiwitch list
```

Shows each profile's name, provider, email, plan, and token expiry. The active profile (per provider) is marked.

### Switch profiles

Requires [shell setup](#shell-setup) — `aiwitch use` is a shell function, not a binary subcommand.

```sh
aiwitch use work
aiwitch current   # → work
```

### Login

```sh
# Codex — ChatGPT login (delegates to codex)
aiwitch login work

# Codex — API key login: aiwitch reads the key from stdin and pipes it to codex.
# aiwitch itself never persists the key.
echo "$OPENAI_API_KEY" | aiwitch login work --api-key

# Claude — interactive only: spawns `claude`; log in via `/login` inside the REPL.
aiwitch login claude-personal
```

> `aiwitch login <claude-profile> --api-key` is rejected with an explicit error before stdin is read; Claude does not yet have an API-key login flow.

### Print env without sourcing

```sh
aiwitch env work               # POSIX: export K='v'
aiwitch env work --shell fish  # fish:   set -gx K 'v'
```

## Configuration

Profiles are stored in `~/.config/aiwitch/profiles.toml`:

```toml
[[profiles]]
name = "work"
backend = "codex"
home_dir = "~/.codex-work"

[[profiles]]
name = "personal"
backend = "codex"
home_dir = "~/.codex-personal"

[[profiles]]
name = "claude-personal"
backend = "claude"
home_dir = "~/.claude-personal"
```

The file is validated on every load: names must be `[A-Za-z0-9_-]+` (no leading dash), and duplicate names are rejected.

## Roadmap

- More providers (Gemini, …) behind the existing `Backend` trait.
- Prebuilt Homebrew bottles to skip the Rust build step.
- Optional encrypted secret storage (today `aiwitch` does not store secrets — they live in the provider's own home dir).

## License

[MIT](LICENSE)
