use crate::backend::{AnyBackend, Backend};
use crate::error::Result;
use crate::profile::{ProfilesFile, store};
use crate::shell::validate_profile_name;
use std::collections::HashMap;

const AIWITCH_CURRENT_KEY: &str = "AIWITCH_CURRENT";
const UNMANAGED: &str = "(unmanaged)";

/** Read-only view over the process environment so tests can inject a fake. */
pub trait EnvLookup {
    fn get(&self, key: &str) -> Option<String>;
}

struct SystemEnv;
impl EnvLookup for SystemEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl EnvLookup for HashMap<String, String> {
    fn get(&self, key: &str) -> Option<String> {
        HashMap::get(self, key).cloned()
    }
}

#[derive(Debug)]
pub struct CurrentOutcome {
    /** Goes to stdout (always exactly one line). */
    pub display: String,
    /** Optional one-line message for stderr (e.g. invalid AIWITCH_CURRENT). */
    pub warning: Option<String>,
}

pub fn run() -> Result<()> {
    let outcome = decide(&SystemEnv, store::load)?;
    if let Some(w) = &outcome.warning {
        eprintln!("warning: {w}");
    }
    println!("{}", outcome.display);
    Ok(())
}

/** Pure decision function. AIWITCH_CURRENT (when valid) is the fast path and
 *  skips loading `profiles.toml` entirely so first-run UX errors don't surface
 *  for users who already have a sentinel set. */
pub fn decide<E: EnvLookup, F: FnOnce() -> Result<ProfilesFile>>(
    env: &E,
    load_profiles: F,
) -> Result<CurrentOutcome> {
    if let Some(raw) = env.get(AIWITCH_CURRENT_KEY) {
        if !raw.is_empty() {
            return Ok(if validate_profile_name(&raw).is_ok() {
                CurrentOutcome { display: raw, warning: None }
            } else {
                CurrentOutcome {
                    display: UNMANAGED.to_string(),
                    warning: Some(format!(
                        "{AIWITCH_CURRENT_KEY} is set to invalid value {raw:?}"
                    )),
                }
            });
        }
    }

    let profiles = load_profiles()?;
    for p in &profiles.profiles {
        let backend = AnyBackend::from_kind(p.backend);
        let exports = backend.env_exports(p)?;
        if exports.is_empty() {
            continue;
        }
        let all_match = exports
            .iter()
            .all(|(k, v)| env.get(k).as_deref() == Some(v.as_str()));
        if all_match {
            return Ok(CurrentOutcome { display: p.name.clone(), warning: None });
        }
    }

    Ok(CurrentOutcome { display: UNMANAGED.to_string(), warning: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::profile::Profile;
    use std::path::PathBuf;

    fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn pf(specs: &[(&str, &str)]) -> ProfilesFile {
        ProfilesFile {
            profiles: specs
                .iter()
                .map(|(name, home)| Profile {
                    name: name.to_string(),
                    backend: BackendKind::Codex,
                    home_dir: PathBuf::from(home),
                })
                .collect(),
        }
    }

    fn unreachable_loader() -> Result<ProfilesFile> {
        panic!("profiles loader must not be called when AIWITCH_CURRENT is valid");
    }

    #[test]
    fn aiwitch_current_valid_is_fast_path_and_skips_load() {
        let env = env_map(&[("AIWITCH_CURRENT", "personal")]);
        let outcome = decide(&env, unreachable_loader).unwrap();
        assert_eq!(outcome.display, "personal");
        assert!(outcome.warning.is_none());
    }

    #[test]
    fn aiwitch_current_invalid_renders_unmanaged_with_warning() {
        let env = env_map(&[("AIWITCH_CURRENT", "with.dot")]);
        let outcome = decide(&env, unreachable_loader).unwrap();
        assert_eq!(outcome.display, "(unmanaged)");
        let w = outcome.warning.unwrap();
        assert!(w.contains("AIWITCH_CURRENT"));
        assert!(w.contains("with.dot"));
    }

    #[test]
    fn aiwitch_current_invalid_leading_dash_renders_unmanaged() {
        let env = env_map(&[("AIWITCH_CURRENT", "-foo")]);
        let outcome = decide(&env, unreachable_loader).unwrap();
        assert_eq!(outcome.display, "(unmanaged)");
        assert!(outcome.warning.is_some());
    }

    #[test]
    fn aiwitch_current_empty_string_treated_as_unset_and_falls_back() {
        let env = env_map(&[("AIWITCH_CURRENT", "")]);
        let profiles = pf(&[("personal", "/Users/x/.codex-personal")]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        // No CODEX_HOME in env → no match → unmanaged
        assert_eq!(outcome.display, "(unmanaged)");
        assert!(outcome.warning.is_none());
    }

    #[test]
    fn fallback_matches_when_codex_home_matches_a_profile() {
        let env = env_map(&[("CODEX_HOME", "/Users/x/.codex-work")]);
        let profiles = pf(&[
            ("personal", "/Users/x/.codex-personal"),
            ("work", "/Users/x/.codex-work"),
        ]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        assert_eq!(outcome.display, "work");
    }

    #[test]
    fn fallback_first_match_wins() {
        let env = env_map(&[("CODEX_HOME", "/shared")]);
        let profiles = pf(&[("a", "/shared"), ("b", "/shared")]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        assert_eq!(outcome.display, "a");
    }

    #[test]
    fn fallback_unmanaged_when_no_profile_matches() {
        let env = env_map(&[("CODEX_HOME", "/some/other/path")]);
        let profiles = pf(&[("personal", "/Users/x/.codex-personal")]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        assert_eq!(outcome.display, "(unmanaged)");
    }

    #[test]
    fn fallback_unmanaged_when_codex_home_is_unset() {
        let env: HashMap<String, String> = HashMap::new();
        let profiles = pf(&[("personal", "/Users/x/.codex-personal")]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        assert_eq!(outcome.display, "(unmanaged)");
    }

    #[test]
    fn fallback_unmanaged_when_no_profiles_configured() {
        let env: HashMap<String, String> = HashMap::new();
        let profiles = pf(&[]);
        let outcome = decide(&env, || Ok(profiles)).unwrap();
        assert_eq!(outcome.display, "(unmanaged)");
    }

    #[test]
    fn fallback_propagates_load_error() {
        let env: HashMap<String, String> = HashMap::new();
        let result = decide(&env, || {
            Err(anyhow::anyhow!("simulated profiles.toml missing"))
        });
        assert!(result.is_err());
    }

    #[test]
    fn aiwitch_current_skips_load_even_when_load_would_fail() {
        let env = env_map(&[("AIWITCH_CURRENT", "personal")]);
        let outcome = decide(&env, || -> Result<ProfilesFile> {
            Err(anyhow::anyhow!("should not be called"))
        })
        .unwrap();
        assert_eq!(outcome.display, "personal");
    }
}
