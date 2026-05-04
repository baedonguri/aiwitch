use super::ProfilesFile;
use crate::error::Result;
use std::path::{Path, PathBuf};

/** Always `$HOME/.config/aiwitch/profiles.toml`, on every OS. */
pub fn config_path() -> Result<PathBuf> {
    todo!("read $HOME and join .config/aiwitch/profiles.toml")
}

/** Loads the config file, expands `~` in each `home_dir`, and returns the result. */
pub fn load() -> Result<ProfilesFile> {
    todo!("read file, parse TOML, expand_home_dir on each profile, helpful message on missing")
}

/** Expand a leading `~` or `~/...` against `$HOME`. Other forms are returned unchanged. */
pub fn expand_home_dir(_path: &Path) -> Result<PathBuf> {
    todo!("if starts with ~, replace with $HOME; otherwise return as-is")
}

#[allow(dead_code)]
pub fn sample_toml() -> &'static str {
    r#"# ~/.config/aiwitch/profiles.toml
[[profiles]]
name = "personal"
backend = "codex"
home_dir = "~/.codex-personal"

[[profiles]]
name = "work"
backend = "codex"
home_dir = "~/.codex-work"
"#
}
