use crate::error::Result;
use clap::{Parser, Subcommand};

mod add;
mod current;
mod env;
mod list;
mod login;
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

/** CLI-facing provider name. Today only `codex` is supported; clap enforces that. */
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ProviderArg {
    Codex,
}

impl From<ProviderArg> for crate::backend::BackendKind {
    fn from(p: ProviderArg) -> Self {
        match p {
            ProviderArg::Codex => crate::backend::BackendKind::Codex,
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
        Command::Env { profile, shell } => env::run(&profile, shell.into()),
        Command::Login { profile, api_key } => login::run(&profile, api_key),
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
