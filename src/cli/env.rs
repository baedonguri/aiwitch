use crate::error::Result;

pub fn run(_profile_name: &str) -> Result<()> {
    todo!(
        "1) validate_profile_name; \
         2) load the profile by name; \
         3) call backend.env_exports, validate_env_value + sh_quote each value; \
         4) print `export KEY='value'` lines plus AIWITCH_CURRENT"
    )
}
