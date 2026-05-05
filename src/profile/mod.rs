use crate::backend::BackendKind;
use crate::error::Result;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub backend: BackendKind,
    /** Backend state root (e.g. `CODEX_HOME`). Always absolute after loading. */
    pub home_dir: PathBuf,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProfilesFile {
    #[serde(default, rename = "profiles")]
    pub profiles: Vec<Profile>,
}

impl ProfilesFile {
    /** Exact-match lookup. Error message lists available names so users can fix typos.
     *  Duplicate names are already rejected at load time. */
    pub fn find_by_name(&self, name: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| {
                let available: Vec<&str> = self.profiles.iter().map(|p| p.name.as_str()).collect();
                if available.is_empty() {
                    anyhow!("no profile named {name:?} (no profiles configured)")
                } else {
                    anyhow!(
                        "no profile named {name:?}. available: {}",
                        available.join(", ")
                    )
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf(names: &[&str]) -> ProfilesFile {
        ProfilesFile {
            profiles: names
                .iter()
                .map(|n| Profile {
                    name: n.to_string(),
                    backend: BackendKind::Codex,
                    home_dir: PathBuf::from(format!("/abs/{n}")),
                })
                .collect(),
        }
    }

    #[test]
    fn find_by_name_hit() {
        let f = pf(&["personal", "work"]);
        assert_eq!(f.find_by_name("work").unwrap().name, "work");
    }

    #[test]
    fn find_by_name_miss_lists_available() {
        let f = pf(&["personal", "work"]);
        let err = f.find_by_name("ghost").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ghost"));
        assert!(msg.contains("personal"));
        assert!(msg.contains("work"));
    }

    #[test]
    fn find_by_name_miss_on_empty_file() {
        let f = pf(&[]);
        let err = f.find_by_name("anything").unwrap_err();
        assert!(format!("{err}").contains("no profiles configured"));
    }

    #[test]
    fn find_by_name_is_case_sensitive() {
        let f = pf(&["personal"]);
        assert!(f.find_by_name("Personal").is_err());
    }
}
