use super::{Backend, BackendKind, ProfileMeta};
use crate::error::{Result, Context};
use crate::profile::Profile;
use anyhow::ensure;

pub mod auth;
pub mod jwt;

pub struct CodexBackend;

impl Backend for CodexBackend {
    fn id(&self) -> BackendKind {
        BackendKind::Codex
    }

    fn env_exports(&self, profile: &Profile) -> Result<Vec<(String, String)>> {
        ensure!(
            profile.backend == BackendKind::Codex,
            "CodexBackend received non-codex profile {:?}",
            profile.name
        );
        let home = profile
            .home_dir
            .to_str()
            .with_context(|| {
                format!(
                    "profile {:?} home_dir is not valid UTF-8: {:?}",
                    profile.name, profile.home_dir
                )
            })?
            .to_string();
        ensure!(
            profile.home_dir.is_absolute(),
            "profile {:?} home_dir must be absolute: {:?}",
            profile.name,
            profile.home_dir
        );
        Ok(vec![("CODEX_HOME".to_string(), home)])
    }

    fn read_meta(&self, _profile: &Profile) -> Result<ProfileMeta> {
        todo!("parse auth.json and decode id_token")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use std::path::PathBuf;

    fn profile(name: &str, home: &str) -> Profile {
        Profile {
            name: name.to_string(),
            backend: BackendKind::Codex,
            home_dir: PathBuf::from(home),
        }
    }

    #[test]
    fn env_exports_returns_codex_home() {
        let p = profile("personal", "/Users/x/.codex-personal");
        let exports = CodexBackend.env_exports(&p).unwrap();
        assert_eq!(
            exports,
            vec![(
                "CODEX_HOME".to_string(),
                "/Users/x/.codex-personal".to_string()
            )]
        );
    }

    #[test]
    fn env_exports_preserves_spaces_in_path() {
        let p = profile("p", "/Users/x/with space/.codex");
        let exports = CodexBackend.env_exports(&p).unwrap();
        assert_eq!(exports[0].1, "/Users/x/with space/.codex");
    }

    #[test]
    fn env_exports_rejects_relative_home_dir() {
        let p = profile("p", "rel/codex");
        assert!(CodexBackend.env_exports(&p).is_err());
    }

    #[test]
    fn env_exports_emits_single_pair() {
        let p = profile("p", "/abs/codex");
        let exports = CodexBackend.env_exports(&p).unwrap();
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].0, "CODEX_HOME");
    }
}
