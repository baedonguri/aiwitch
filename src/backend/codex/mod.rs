use super::{Backend, BackendKind, ProfileMeta};
use crate::error::Result;
use crate::profile::Profile;

pub mod auth;
pub mod jwt;

pub struct CodexBackend;

impl Backend for CodexBackend {
    fn id(&self) -> BackendKind {
        BackendKind::Codex
    }

    fn env_exports(&self, _profile: &Profile) -> Result<Vec<(String, String)>> {
        todo!("export CODEX_HOME")
    }

    fn read_meta(&self, _profile: &Profile) -> Result<ProfileMeta> {
        todo!("parse auth.json and decode id_token")
    }
}
