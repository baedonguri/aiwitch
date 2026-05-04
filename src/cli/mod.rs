use crate::error::Result;
use clap::{Parser, Subcommand};

mod current;
mod env;
mod list;
mod shell;

#[derive(Parser, Debug)]
#[command(name = "aiwitch", version, about = "Switch between AI CLI accounts/profiles")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /** List profiles with email, plan, and expiry. */
    List,
    /** Print the active profile name. */
    Current,
    /** Print `export` lines for the given profile (intended for `eval`). */
    Env { profile: String },
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

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => list::run(),
        Command::Current => current::run(),
        Command::Env { profile } => env::run(&profile),
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
