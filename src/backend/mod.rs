use crate::error::Result;
use crate::profile::Profile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod codex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Codex,
}

#[derive(Debug, Default, Clone)]
pub struct ProfileMeta {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub subscription_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCommand {
    pub program: String,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

pub trait Backend {
    fn id(&self) -> BackendKind;

    /** Key/value pairs to be emitted as shell `export` lines. Caller handles escaping. */
    fn env_exports(&self, profile: &Profile) -> Result<Vec<(String, String)>>;

    /** Best-effort metadata. Callers must tolerate `Err` and render an empty row. */
    fn read_meta(&self, profile: &Profile) -> Result<ProfileMeta>;
}

pub enum AnyBackend {
    Codex(codex::CodexBackend),
}

impl AnyBackend {
    pub fn from_kind(kind: BackendKind) -> Self {
        match kind {
            BackendKind::Codex => AnyBackend::Codex(codex::CodexBackend),
        }
    }
}

impl Backend for AnyBackend {
    fn id(&self) -> BackendKind {
        match self {
            AnyBackend::Codex(b) => b.id(),
        }
    }
    fn env_exports(&self, profile: &Profile) -> Result<Vec<(String, String)>> {
        match self {
            AnyBackend::Codex(b) => b.env_exports(profile),
        }
    }
    fn read_meta(&self, profile: &Profile) -> Result<ProfileMeta> {
        match self {
            AnyBackend::Codex(b) => b.read_meta(profile),
        }
    }
}
