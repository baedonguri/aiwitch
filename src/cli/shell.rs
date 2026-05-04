use super::ShellKind;
use crate::error::Result;
use crate::shell;

pub fn run_init(kind: ShellKind) -> Result<()> {
    let snippet = shell::init_snippet(kind.into());
    print!("{snippet}");
    Ok(())
}
