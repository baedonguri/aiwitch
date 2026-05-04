use crate::error::Result;
use clap::{Parser, Subcommand};

mod add;
mod current;
mod env;
mod list;
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
    /** Add a Codex profile. */
    Add {
        profile: String,
        #[arg(long = "home")]
        home: Option<std::path::PathBuf>,
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
        Command::Add { profile, home } => add::run(&profile, home.as_deref()),
        Command::List => list::run(),
        Command::Current => current::run(),
        Command::Env { profile, shell } => env::run(&profile, shell.into()),
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
