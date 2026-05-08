use crate::error::Result;
use clap::{Parser, Subcommand};

mod add;
mod current;
mod doctor;
mod env;
mod list;
mod login;
mod remove;
mod rename;
mod run;
mod shell;

#[derive(Parser, Debug)]
#[command(
    name = "aiwitch",
    version,
    about = "Switch between AI CLI accounts/profiles"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /** Add a profile for the given provider and run its login flow (e.g. `aiwitch add codex personal`). */
    Add {
        provider: ProviderArg,
        profile: String,
        #[arg(long = "home")]
        home: Option<std::path::PathBuf>,
        #[arg(long = "auth", value_enum)]
        auth: Option<add::CodexAuthMode>,
        #[arg(long = "print-env", hide = true)]
        print_env: bool,
        #[arg(long = "shell", value_enum, default_value_t = EnvShell::Posix, hide = true)]
        shell: EnvShell,
    },
    /** List profiles with email, plan, and expiry. */
    List,
    /** Print the active profile name. */
    Current,
    /** Diagnose profile health: provider CLI, home dirs, auth state, expiry, env. */
    Doctor,
    /** Print shell statements for the given profile (intended for `eval` / `source`).
     *  Default output is POSIX (`export K='v'`); pass `--shell=fish` for fish syntax. */
    Env {
        profile: String,
        #[arg(long = "shell", value_enum, default_value_t = EnvShell::Posix)]
        shell: EnvShell,
    },
    /** Login to the provider for the given profile. */
    Login {
        profile: String,
        #[arg(long = "api-key")]
        api_key: bool,
    },
    /** Remove a profile from `~/.config/aiwitch/profiles.toml`. Pass `--purge` to also delete the profile's default home directory. */
    Remove {
        profile: String,
        #[arg(long)]
        purge: bool,
    },
    /** Rename a profile. If the profile uses the default home directory pattern (`~/.codex-<name>` or `~/.claude-<name>`), the directory on disk is also renamed; custom `home_dir` paths are left untouched. */
    Rename { old: String, new: String },
    /** Run a command under the given profile without mutating the current shell. */
    #[command(long_about = "\
Run a command under the given profile without mutating the current shell.

Example: `aiwitch run personal -- codex exec \"hi\"`. Use `--` so flags after \
the profile name are passed to the child instead of consumed by aiwitch.

Inherits the parent environment and overlays the profile's provider env vars \
(CODEX_HOME / CLAUDE_CONFIG_DIR) plus AIWITCH_CURRENT. Note: other variables \
are *not* stripped — if OPENAI_API_KEY or ANTHROPIC_API_KEY are exported in \
the parent shell, the provider CLI may use them instead of the profile's \
stored credentials. To run with a cleaner environment, use `env -i` or \
`env -u` from the shell.")]
    Run {
        profile: String,
        #[arg(trailing_var_arg = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /** Emit a shell snippet that wires up the `use` alias. */
    Shell {
        #[command(subcommand)]
        sub: ShellCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ShellCmd {
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ShellKind {
    Zsh,
    Bash,
    Fish,
}

/** CLI-facing provider name. Clap enforces the allowed set. */
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ProviderArg {
    Codex,
    Claude,
}

impl From<ProviderArg> for crate::backend::BackendKind {
    fn from(p: ProviderArg) -> Self {
        match p {
            ProviderArg::Codex => crate::backend::BackendKind::Codex,
            ProviderArg::Claude => crate::backend::BackendKind::Claude,
        }
    }
}

/** Output flavor for `aiwitch env`. Separate from `ShellKind` because zsh and bash
 *  share POSIX output, so a 3-way enum here would expose meaningless variants. */
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum EnvShell {
    Posix,
    Fish,
}

impl From<EnvShell> for crate::shell::EnvFormat {
    fn from(s: EnvShell) -> Self {
        match s {
            EnvShell::Posix => crate::shell::EnvFormat::Posix,
            EnvShell::Fish => crate::shell::EnvFormat::Fish,
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Add {
            provider,
            profile,
            home,
            auth,
            print_env,
            shell,
        } => add::run(
            provider.into(),
            &profile,
            home.as_deref(),
            auth,
            print_env,
            shell.into(),
        ),
        Command::List => list::run(),
        Command::Current => current::run(),
        Command::Doctor => doctor::run(),
        Command::Env { profile, shell } => env::run(&profile, shell.into()),
        Command::Login { profile, api_key } => login::run(&profile, api_key),
        Command::Remove { profile, purge } => remove::run(&profile, purge),
        Command::Rename { old, new } => rename::run(&old, &new),
        Command::Run { profile, cmd } => run::run(&profile, &cmd),
        Command::Shell { sub } => match sub {
            ShellCmd::Init { shell } => shell::run_init(shell),
        },
    }
}

impl From<ShellKind> for crate::shell::Shell {
    fn from(s: ShellKind) -> Self {
        match s {
            ShellKind::Zsh => crate::shell::Shell::Zsh,
            ShellKind::Bash => crate::shell::Shell::Bash,
            ShellKind::Fish => crate::shell::Shell::Fish,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("aiwitch").chain(args.iter().copied()))
    }

    fn assert_run(cli: Cli, expected_profile: &str, expected_cmd: &[&str]) {
        match cli.command {
            Command::Run { profile, cmd } => {
                assert_eq!(profile, expected_profile);
                assert_eq!(cmd, expected_cmd);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn run_parses_with_double_dash_separator() {
        let cli = parse(&["run", "personal", "--", "codex", "--version"]).unwrap();
        assert_run(cli, "personal", &["codex", "--version"]);
    }

    #[test]
    fn run_parses_without_double_dash_separator() {
        let cli = parse(&["run", "personal", "codex", "--version"]).unwrap();
        assert_run(cli, "personal", &["codex", "--version"]);
    }

    #[test]
    fn run_passes_through_complex_child_args() {
        let cli = parse(&["run", "work", "--", "codex", "exec", "hello world"]).unwrap();
        assert_run(cli, "work", &["codex", "exec", "hello world"]);
    }

    #[test]
    fn run_rejects_missing_cmd() {
        let err = parse(&["run", "personal"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn run_rejects_missing_profile_and_cmd() {
        assert!(parse(&["run"]).is_err());
    }

    #[test]
    fn run_help_flag_works_before_profile() {
        let err = parse(&["run", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }
}
