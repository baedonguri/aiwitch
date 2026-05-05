<div align="center">
  <h1>aiwitch</h1>
  <p>
    Switch between multiple AI CLI accounts and profiles in a single shell.
    Today: <a href="https://github.com/openai/codex">Codex</a>. Built so more providers slot in without rewriting the CLI surface.
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

# Add profiles (one per account)
aiwitch add codex-personal
aiwitch add codex-work

# Switch the current shell, then run Codex with that profile's account
aiwitch use codex-personal && codex
aiwitch use codex-work     && codex
```

> First run prompts Codex to log in. For API-key flows, see [Login](#login).

## Why

Running multiple Codex accounts — personal, work, an API-key-only sandbox — means juggling `CODEX_HOME` and shuffling `auth.json` files by hand. `aiwitch` keeps each account in its own home directory and switches the active one in your current shell with a single command.

Inspired by `nvm`/`pyenv`-style version switchers, applied to AI CLI accounts.

## Requirements

- macOS with [Homebrew](https://brew.sh), or any OS with Rust 1.85+
- [Codex CLI](https://github.com/openai/codex) installed and on `PATH`
- For API-key login: an OpenAI API key on stdin (`$OPENAI_API_KEY` or piped)

## Key features

- **[Per-profile `CODEX_HOME`](#how-it-works)** — every profile has an isolated directory; switching is a single env-var swap.
- **[In-shell switching](#shell-setup)** — `aiwitch use <profile>` and `aiwitch add <profile>` mutate the current shell via a small snippet for zsh, bash, and fish.
- **Two Codex auth modes** — ChatGPT login and API-key login; the latter reads the key from stdin and pipes it straight to `codex` without `aiwitch` persisting it.
- **TOML config** — a single `~/.config/aiwitch/profiles.toml` with strict name validation and duplicate detection.
- **Backend trait** — adding a new provider is a `Backend` impl, not a CLI rewrite.

## Install

> Homebrew install is enabled once `v0.1.0` is tagged and the tap is published. Until then, use the [from-source](#from-source-any-platform) path.

### macOS — Homebrew

```sh
brew install baedonguri/tap/aiwitch
```

After tapping once:

```sh
brew tap baedonguri/tap
brew install aiwitch
```

### From source (any platform)

Requires Rust 1.85+.

```sh
cargo install --git https://github.com/baedonguri/aiwitch --tag v0.1.0
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
| `aiwitch add <profile>` | Add a profile **and** activate it in the current shell. |

Without `shell init` you can still drive everything manually via `eval "$(aiwitch env <profile>)"`.

## How it works

Codex picks up its account from the `CODEX_HOME` directory (`auth.json` lives inside). `aiwitch` keeps one such directory **per profile** and exposes them as named entries in `~/.config/aiwitch/profiles.toml`.

```
~/.codex-work/         ← CODEX_HOME for "work"
  └── auth.json
~/.codex-personal/     ← CODEX_HOME for "personal"
  └── auth.json
```

`aiwitch use <profile>` evaluates to `export CODEX_HOME=...` and `export AIWITCH_CURRENT=...` (or `set -gx` on fish). The next `codex` invocation picks up the right account; `aiwitch current` reads `AIWITCH_CURRENT`. No background daemon, no symlink swapping, no global state to corrupt.

## Usage

### Add a profile

```sh
aiwitch add work --home ~/.codex-work --auth api
aiwitch add personal --home ~/.codex-personal --auth chatgpt
```

| Flag | Meaning |
|---|---|
| `--home <PATH>` | Directory used as `CODEX_HOME`. Optional; defaults to `~/.codex-<profile>`. |
| `--auth <chatgpt\|api>` | Codex login mode. Optional. |

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
# ChatGPT login (delegates to codex)
aiwitch login work

# API key login: aiwitch reads the key from stdin and pipes it to codex.
# aiwitch itself never persists the key.
echo "$OPENAI_API_KEY" | aiwitch login work --api-key
```

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
```

The file is validated on every load: names must be `[A-Za-z0-9_-]+` (no leading dash), and duplicate names are rejected.

## Roadmap

- More providers (Claude Code, Gemini, …) behind the existing `Backend` trait.
- Prebuilt Homebrew bottles to skip the Rust build step.
- Optional encrypted secret storage (today `aiwitch` does not store secrets — they live in the provider's own home dir).

## License

[MIT](LICENSE)
