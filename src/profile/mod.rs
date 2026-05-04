use crate::backend::BackendKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub backend: BackendKind,
    /** Backend state root (e.g. CODEX_HOME). Always absolute after loading. */
    pub home_dir: PathBuf,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProfilesFile {
    #[serde(default, rename = "profiles")]
    pub profiles: Vec<Profile>,
}
