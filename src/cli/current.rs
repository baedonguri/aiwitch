use crate::error::Result;

pub fn run() -> Result<()> {
    todo!(
        "1) prefer AIWITCH_CURRENT env var; \
         2) otherwise, for each profile compare backend.env_exports() pairs against the current process env; \
         3) print '(unmanaged)' if nothing matches"
    )
}
