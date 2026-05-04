use crate::backend::BackendKind;
use crate::error::{Context, Result};
use crate::profile::{Profile, ProfilesFile, store};
use crate::shell::validate_profile_name;
use anyhow::{anyhow, ensure};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(profile: &str, home: Option<&Path>) -> Result<()> {
    let config_path = store::config_path()?;
    let home_base = store::expand_home_dir(Path::new("~"))?;
    let outcome = add_to_config(&config_path, &home_base, profile, home)?;

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "added profile {}", outcome.profile)?;
    writeln!(stdout, "home_dir: {}", outcome.home_dir)?;
    writeln!(
        stdout,
        "next: CODEX_HOME=\"{}\" codex",
        outcome.expanded_home.display()
    )?;
    writeln!(
        stdout,
        "switch: eval \"$(aiwitch env {})\"",
        outcome.profile
    )?;
    Ok(())
}

#[derive(Debug)]
struct AddOutcome {
    profile: String,
    home_dir: String,
    expanded_home: PathBuf,
}

fn add_to_config(
    config_path: &Path,
    home_base: &Path,
    profile: &str,
    home: Option<&Path>,
) -> Result<AddOutcome> {
    let home_dir = match home {
        Some(path) => path
            .to_str()
            .ok_or_else(|| anyhow!("home path is not valid UTF-8: {path:?}"))?
            .to_string(),
        None => default_home(profile),
    };

    let existing = match std::fs::read_to_string(config_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(
                anyhow::Error::new(e).context(format!("failed to read {}", config_path.display()))
            );
        }
    };
    let updated = add_profile_to_text_in(existing.as_deref(), home_base, profile, &home_dir)?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let expanded_home = store::expand_home_dir_in(Path::new(&home_dir), home_base)?;
    std::fs::create_dir_all(&expanded_home)
        .with_context(|| format!("failed to create {}", expanded_home.display()))?;
    std::fs::write(config_path, updated)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(AddOutcome {
        profile: profile.to_string(),
        home_dir,
        expanded_home,
    })
}

fn default_home(profile: &str) -> String {
    format!("~/.codex-{profile}")
}

fn add_profile_to_text_in(
    existing: Option<&str>,
    home_base: &Path,
    name: &str,
    home_dir: &str,
) -> Result<String> {
    validate_profile_name(name)?;
    store::expand_home_dir_in(Path::new(home_dir), home_base)?;

    let existing = existing.unwrap_or("");
    let parsed: ProfilesFile = if existing.trim().is_empty() {
        ProfilesFile::default()
    } else {
        toml::from_str(existing).context("failed to parse existing profiles TOML")?
    };
    let mut seen = HashSet::new();
    for profile in &parsed.profiles {
        validate_profile_name(&profile.name).context("invalid existing profile name")?;
        store::expand_home_dir_in(&profile.home_dir, home_base)
            .context("invalid existing profile home_dir")?;
        ensure!(
            seen.insert(profile.name.as_str()),
            "duplicate profile name {:?}",
            profile.name
        );
    }
    ensure!(
        !parsed.profiles.iter().any(|p| p.name == name),
        "duplicate profile name {name:?}"
    );

    let block = toml::to_string(&ProfilesFile {
        profiles: vec![Profile {
            name: name.to_string(),
            backend: BackendKind::Codex,
            home_dir: Path::new(home_dir).to_path_buf(),
        }],
    })?;
    if existing.trim().is_empty() {
        return Ok(block);
    }

    let mut out = existing.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&block);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn default_home_for_profile_uses_codex_prefix() {
        assert_eq!(default_home("codex_lemon"), "~/.codex-codex_lemon");
    }

    #[test]
    fn add_to_empty_creates_single_codex_profile() {
        let got = add_profile_to_text_in(
            None,
            Path::new("/home"),
            "codex_lemon",
            "~/.codex-codex_lemon",
        )
        .unwrap();

        assert_eq!(
            got,
            "[[profiles]]\nname = \"codex_lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-codex_lemon\"\n"
        );
    }

    #[test]
    fn add_to_existing_appends_profile() {
        let existing =
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n";

        let got = add_profile_to_text_in(
            Some(existing),
            Path::new("/home"),
            "codex_lemon",
            "~/.codex-lemon",
        )
        .unwrap();

        assert_eq!(
            got,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n\n[[profiles]]\nname = \"codex_lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-lemon\"\n"
        );
    }

    #[test]
    fn add_rejects_duplicate_profile_name() {
        let existing = "[[profiles]]\nname = \"codex_lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-lemon\"\n";

        let err = add_profile_to_text_in(
            Some(existing),
            Path::new("/home"),
            "codex_lemon",
            "~/.codex-other",
        )
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate"));
    }

    #[test]
    fn add_rejects_existing_duplicate_profile_names() {
        let existing = "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n\n[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work-2\"\n";

        let err = add_profile_to_text_in(
            Some(existing),
            Path::new("/home"),
            "codex_lemon",
            "~/.codex-lemon",
        )
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate"));
    }

    #[test]
    fn add_rejects_invalid_profile_name() {
        assert!(
            add_profile_to_text_in(None, Path::new("/home"), "bad.name", "~/.codex-bad").is_err()
        );
    }

    #[test]
    fn add_rejects_relative_home_dir() {
        assert!(
            add_profile_to_text_in(None, Path::new("/home"), "codex_lemon", "relative").is_err()
        );
    }

    #[test]
    fn add_to_config_creates_config_and_home_dir() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        let outcome = add_to_config(&config, tmp.path(), "codex_lemon", None).unwrap();

        assert_eq!(outcome.profile, "codex_lemon");
        assert_eq!(outcome.home_dir, "~/.codex-codex_lemon");
        assert_eq!(outcome.expanded_home, tmp.path().join(".codex-codex_lemon"));
        assert!(outcome.expanded_home.is_dir());
        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "[[profiles]]\nname = \"codex_lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-codex_lemon\"\n"
        );
    }

    #[test]
    fn add_to_config_appends_to_existing_config() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();

        let outcome = add_to_config(
            &config,
            tmp.path(),
            "lemon",
            Some(Path::new("~/.codex-lemon")),
        )
        .unwrap();

        assert_eq!(outcome.expanded_home, tmp.path().join(".codex-lemon"));
        assert!(outcome.expanded_home.is_dir());
        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n\n[[profiles]]\nname = \"lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-lemon\"\n"
        );
    }

    #[test]
    fn add_to_config_duplicate_does_not_create_new_home_dir() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"lemon\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-lemon\"\n",
        )
        .unwrap();

        let err = add_to_config(
            &config,
            tmp.path(),
            "lemon",
            Some(Path::new("~/.codex-other")),
        )
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate"));
        assert!(!tmp.path().join(".codex-other").exists());
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("aiwitch-add-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
