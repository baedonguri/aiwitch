# aiwitch

[![CI](https://github.com/baedonguri/aiwitch/actions/workflows/ci.yml/badge.svg)](https://github.com/baedonguri/aiwitch/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Switch between AI CLI accounts and profiles. Today: [Codex](https://github.com/openai/codex). Designed to add more providers without rewriting the CLI surface.

## Why

Running multiple Codex accounts (personal / work / API key) means juggling `CODEX_HOME` and shuffling `auth.json` files by hand. `aiwitch` keeps each account in its own home directory and switches the active one in your shell with a single command.

## Install

### macOS (Homebrew)

```sh
brew install baedonguri/tap/aiwitch
```

After tapping once:

```sh
brew tap baedonguri/tap
brew install aiwitch
```

### From source

Requires Rust 1.85+.

```sh
cargo install --git https://github.com/baedonguri/aiwitch --tag v0.1.0
```

## Shell setup

`aiwitch` switches profiles by exporting environment variables in the **current shell**, so it needs a tiny shell function. Add this to your `~/.zshrc` (or `~/.bashrc`, or `~/.config/fish/config.fish`):

```sh
eval "$(aiwitch shell init zsh)"   # or: bash | fish
```

This wires up two extra subcommands:

- `aiwitch add <profile>` — adds a profile **and** activates it.
- `aiwitch use <profile>` — switches to an existing profile.

Without `shell init`, you can still use the underlying commands directly via `eval "$(aiwitch env <profile>)"`.

## Usage

### Add a profile

```sh
aiwitch add work --home ~/.codex-work --auth api
aiwitch add personal --home ~/.codex-personal --auth chatgpt
```

- `--home <PATH>` — directory used as `CODEX_HOME` for this profile. Optional; defaults to `~/.codex-<profile>`.
- `--auth <chatgpt|api>` — which Codex login mode to use. Optional.

### List profiles

```sh
aiwitch list
```

Shows each profile's name, provider, email, plan, and token expiry.

### Switch profiles

After `shell init` is loaded:

```sh
aiwitch use work
aiwitch current   # prints: work
```

### Login

```sh
aiwitch login work             # ChatGPT login
aiwitch login work --api-key   # API key login (prompts securely)
```

### Print env without sourcing

```sh
aiwitch env work               # POSIX: export K='v'
aiwitch env work --shell fish  # fish:   set -gx K 'v'
```

## Config

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

You can edit it by hand; `aiwitch` validates the file on load.

## Roadmap

- More providers (Claude Code, Gemini, etc.) behind the same `Backend` trait.
- Prebuilt Homebrew bottles to skip the Rust build step.
- Optional encrypted secret storage.

## License

[MIT](LICENSE)
