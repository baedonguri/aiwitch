use anyhow::Result;

mod backend;
mod cli;
mod error;
mod profile;
mod shell;

fn main() -> Result<()> {
    cli::run()
}
