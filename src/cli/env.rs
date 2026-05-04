use crate::backend::{AnyBackend, Backend};
use crate::cli::current::AIWITCH_CURRENT_KEY;
use crate::error::Result;
use crate::profile::{ProfilesFile, store};
use crate::shell::{EnvFormat, render_env, validate_profile_name};
use std::io::Write;

pub fn run(profile_name: &str, format: EnvFormat) -> Result<()> {
    validate_profile_name(profile_name)?;
    let profiles = store::load()?;
    let rendered = render(&profiles, profile_name, format)?;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(rendered.as_bytes())?;
    Ok(())
}

/** Pure renderer used by tests and by `run`. Returns the full snippet as a String;
 *  no I/O. On any error (unknown name, bad key/value) returns Err and the caller
 *  must not write anything to stdout.
 *
 *  Re-validates the profile name even though `run` already does so — defensive in
 *  case future callers (e.g. an `import` command) reach this directly. */
pub fn render(profiles: &ProfilesFile, name: &str, format: EnvFormat) -> Result<String> {
    validate_profile_name(name)?;
    let profile = profiles.find_by_name(name)?;
    let backend = AnyBackend::from_kind(profile.backend);
    let mut pairs = backend.env_exports(profile)?;
    pairs.push((AIWITCH_CURRENT_KEY.to_string(), profile.name.clone()));
    render_env(format, &pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::profile::Profile;
    use std::path::PathBuf;

    fn pf() -> ProfilesFile {
        ProfilesFile {
            profiles: vec![
                Profile {
                    name: "personal".to_string(),
                    backend: BackendKind::Codex,
                    home_dir: PathBuf::from("/Users/x/.codex-personal"),
                },
                Profile {
                    name: "work".to_string(),
                    backend: BackendKind::Codex,
                    home_dir: PathBuf::from("/Users/x/with space/.codex-work"),
                },
            ],
        }
    }

    #[test]
    fn render_posix_includes_codex_home_and_aiwitch_current() {
        let got = render(&pf(), "personal", EnvFormat::Posix).unwrap();
        assert_eq!(
            got,
            "export CODEX_HOME='/Users/x/.codex-personal'\n\
             export AIWITCH_CURRENT='personal'\n"
        );
    }

    #[test]
    fn render_fish_uses_set_gx() {
        let got = render(&pf(), "personal", EnvFormat::Fish).unwrap();
        assert_eq!(
            got,
            "set -gx CODEX_HOME '/Users/x/.codex-personal'\n\
             set -gx AIWITCH_CURRENT 'personal'\n"
        );
    }

    #[test]
    fn render_preserves_spaces_in_path() {
        let got = render(&pf(), "work", EnvFormat::Posix).unwrap();
        assert!(got.contains("'/Users/x/with space/.codex-work'"));
    }

    #[test]
    fn render_unknown_name_errors() {
        assert!(render(&pf(), "ghost", EnvFormat::Posix).is_err());
    }

    #[test]
    fn render_unknown_name_lists_available() {
        let err = render(&pf(), "ghost", EnvFormat::Posix).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("personal"));
        assert!(msg.contains("work"));
    }

    #[test]
    fn aiwitch_current_is_last_pair() {
        let got = render(&pf(), "personal", EnvFormat::Posix).unwrap();
        let lines: Vec<&str> = got.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("export CODEX_HOME="));
        assert!(lines[1].starts_with("export AIWITCH_CURRENT="));
    }

    #[test]
    fn render_rejects_invalid_profile_name_defensively() {
        assert!(render(&pf(), "with.dot", EnvFormat::Posix).is_err());
        assert!(render(&pf(), "-leading", EnvFormat::Posix).is_err());
        assert!(render(&pf(), "", EnvFormat::Posix).is_err());
    }
}
