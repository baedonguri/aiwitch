use crate::backend::BackendKind;
use crate::error::{Context, Result};
use crate::profile::{ProfilesFile, store};
use anyhow::anyhow;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(profile: &str, purge: bool) -> Result<()> {
    let config_path = store::config_path()?;
    let home_base = store::expand_home_dir(Path::new("~"))?;
    let env_current = std::env::var(super::current::AIWITCH_CURRENT_KEY).ok();

    let outcome = remove_profile(
        &config_path,
        &home_base,
        profile,
        purge,
        env_current.as_deref(),
    )?;

    if outcome.was_active {
        eprintln!(
            "warning: AIWITCH_CURRENT is still {:?} in this shell;\n         start a new shell or run `aiwitch use <other>`",
            outcome.name
        );
    }

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "removed profile {}", outcome.name)?;
    if let Some(path) = &outcome.purged {
        writeln!(stdout, "purged: {}", path.display())?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct RemoveOutcome {
    pub name: String,
    pub purged: Option<PathBuf>,
    pub was_active: bool,
}

#[derive(Debug)]
pub struct RemoveTextOutcome {
    pub updated_text: String,
    pub backend: BackendKind,
    pub raw_home: String,
}

pub fn remove_profile(
    config_path: &Path,
    home_base: &Path,
    name: &str,
    purge: bool,
    env_current: Option<&str>,
) -> Result<RemoveOutcome> {
    let existing_text = std::fs::read_to_string(config_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "no profiles file at {} (no profiles configured)",
                config_path.display()
            )
        } else {
            anyhow::Error::new(e).context(format!("failed to read {}", config_path.display()))
        }
    })?;

    let removal = remove_profile_from_text(&existing_text, name)?;
    store::write_atomic(config_path, &removal.updated_text)?;

    let purged = if purge {
        let resolved = store::expand_home_dir_in(Path::new(&removal.raw_home), home_base)?;
        Some(
            purge_home_dir(home_base, removal.backend, name, &resolved).with_context(|| {
                format!(
                    "profile {name:?} was removed from {}, but home_dir was not purged",
                    config_path.display()
                )
            })?,
        )
    } else {
        None
    };

    Ok(RemoveOutcome {
        name: name.to_string(),
        purged,
        was_active: env_current == Some(name),
    })
}

pub fn remove_profile_from_text(existing: &str, name: &str) -> Result<RemoveTextOutcome> {
    let parsed: ProfilesFile = if existing.trim().is_empty() {
        ProfilesFile::default()
    } else {
        toml::from_str(existing).context("failed to parse existing profiles TOML")?
    };

    let idx = parsed
        .profiles
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| {
            let available: Vec<&str> = parsed.profiles.iter().map(|p| p.name.as_str()).collect();
            if available.is_empty() {
                anyhow!("no profile named {name:?} (no profiles configured)")
            } else {
                anyhow!(
                    "no profile named {name:?}. available: {}",
                    available.join(", ")
                )
            }
        })?;

    let removed = &parsed.profiles[idx];
    let backend = removed.backend;
    let raw_home = removed
        .home_dir
        .to_str()
        .ok_or_else(|| anyhow!("home_dir is not valid UTF-8 for profile {name:?}"))?
        .to_string();

    let mut remaining = parsed;
    remaining.profiles.remove(idx);

    let updated_text = if remaining.profiles.is_empty() {
        String::new()
    } else {
        toml::to_string(&remaining)?
    };

    Ok(RemoveTextOutcome {
        updated_text,
        backend,
        raw_home,
    })
}

fn purge_home_dir(
    home_base: &Path,
    backend: BackendKind,
    name: &str,
    resolved: &Path,
) -> Result<PathBuf> {
    let prefix = match backend {
        BackendKind::Codex => "codex",
        BackendKind::Claude => "claude",
    };
    let default = home_base.join(format!(".{prefix}-{name}"));
    if resolved != default {
        return Err(anyhow!(
            "refusing to purge custom home_dir {}\nhint: run `rm -rf {}` manually if you want to delete it",
            resolved.display(),
            resolved.display(),
        ));
    }

    match std::fs::symlink_metadata(resolved) {
        Ok(m) if m.file_type().is_symlink() => Err(anyhow!(
            "refusing to follow symlink home_dir {}\nhint: run `rm -f {}` manually",
            resolved.display(),
            resolved.display(),
        )),
        Ok(_) => {
            std::fs::remove_dir_all(resolved)
                .with_context(|| format!("failed to remove {}", resolved.display()))?;
            Ok(resolved.to_path_buf())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(resolved.to_path_buf()),
        Err(e) => {
            Err(anyhow::Error::new(e).context(format!("failed to stat {}", resolved.display())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX: BackendKind = BackendKind::Codex;
    const CLAUDE: BackendKind = BackendKind::Claude;

    #[test]
    fn remove_text_drops_entry_and_keeps_siblings() {
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"~/.codex-work\"

[[profiles]]
name = \"personal\"
backend = \"codex\"
home_dir = \"~/.codex-personal\"
";
        let got = remove_profile_from_text(existing, "work").unwrap();
        assert_eq!(got.backend, CODEX);
        assert_eq!(got.raw_home, "~/.codex-work");
        assert_eq!(
            got.updated_text,
            "[[profiles]]\nname = \"personal\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-personal\"\n"
        );
    }

    #[test]
    fn remove_text_returns_empty_string_when_last_profile_removed() {
        let existing =
            "[[profiles]]\nname = \"only\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-only\"\n";
        let got = remove_profile_from_text(existing, "only").unwrap();
        assert_eq!(got.updated_text, "");
        assert_eq!(got.backend, CODEX);
        assert_eq!(got.raw_home, "~/.codex-only");
    }

    #[test]
    fn remove_text_preserves_raw_home_for_claude() {
        let existing =
            "[[profiles]]\nname = \"p\"\nbackend = \"claude\"\nhome_dir = \"~/.claude-p\"\n";
        let got = remove_profile_from_text(existing, "p").unwrap();
        assert_eq!(got.backend, CLAUDE);
        assert_eq!(got.raw_home, "~/.claude-p");
    }

    #[test]
    fn remove_text_missing_profile_lists_available() {
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"~/.codex-work\"
";
        let err = remove_profile_from_text(existing, "ghost").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ghost"));
        assert!(msg.contains("work"));
    }

    #[test]
    fn remove_text_missing_profile_in_empty_file_says_no_profiles() {
        let err = remove_profile_from_text("", "ghost").unwrap_err();
        assert!(format!("{err}").contains("no profiles configured"));
    }

    #[test]
    fn remove_profile_writes_updated_toml_and_skips_purge_by_default() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n\n[[profiles]]\nname = \"personal\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-personal\"\n",
        )
        .unwrap();
        let work_home = tmp.path().join(".codex-work");
        std::fs::create_dir_all(&work_home).unwrap();

        let outcome = remove_profile(&config, tmp.path(), "work", false, None).unwrap();

        assert_eq!(outcome.name, "work");
        assert!(outcome.purged.is_none());
        assert!(!outcome.was_active);
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "[[profiles]]\nname = \"personal\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-personal\"\n"
        );
        assert!(work_home.is_dir(), "home_dir must be preserved by default");
    }

    #[test]
    fn remove_profile_with_purge_deletes_default_home_dir() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"personal\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-personal\"\n",
        )
        .unwrap();
        let home = tmp.path().join(".codex-personal");
        std::fs::create_dir_all(home.join("nested")).unwrap();
        std::fs::write(home.join("nested/auth.json"), "{}").unwrap();

        let outcome = remove_profile(&config, tmp.path(), "personal", true, None).unwrap();

        assert_eq!(outcome.purged.as_deref(), Some(home.as_path()));
        assert!(!home.exists(), "purged dir must be gone");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
    }

    #[test]
    fn remove_profile_with_purge_idempotent_when_dir_already_missing() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"p\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-p\"\n",
        )
        .unwrap();
        // home_dir does not exist on disk.

        let outcome = remove_profile(&config, tmp.path(), "p", true, None).unwrap();
        let expected = tmp.path().join(".codex-p");
        assert_eq!(outcome.purged.as_deref(), Some(expected.as_path()));
    }

    #[test]
    fn remove_profile_with_purge_rejects_custom_home_dir() {
        let tmp = tempdir();
        let custom = tmp.path().join("custom-place");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("canary"), "still here").unwrap();

        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            format!(
                "[[profiles]]\nname = \"p\"\nbackend = \"codex\"\nhome_dir = \"{}\"\n",
                custom.display()
            ),
        )
        .unwrap();

        let err = remove_profile(&config, tmp.path(), "p", true, None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("rm -rf"), "must hint manual rm: {msg}");
        assert!(msg.contains("was removed from"), "must signal toml mutated");
        assert!(custom.is_dir(), "custom dir must be untouched");
        assert!(custom.join("canary").exists());
        // TOML rewrite must have happened (order invariant).
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
    }

    #[test]
    fn remove_profile_with_purge_rejects_dangerous_home_path() {
        // resolved == /tmp would never match default pattern, so it gets the
        // same custom-path rejection. Asserts the safety guard explicitly.
        let tmp = tempdir();
        let dangerous = tmp.path().join("danger");
        std::fs::create_dir_all(&dangerous).unwrap();

        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            format!(
                "[[profiles]]\nname = \"p\"\nbackend = \"codex\"\nhome_dir = \"{}\"\n",
                dangerous.display()
            ),
        )
        .unwrap();

        let err = remove_profile(&config, tmp.path(), "p", true, None).unwrap_err();
        assert!(format!("{err:#}").contains("rm -rf"));
        assert!(dangerous.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn remove_profile_with_purge_rejects_symlink_home_dir() {
        let tmp = tempdir();
        let real = tmp.path().join("real-target");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("canary"), "still here").unwrap();

        let link = tmp.path().join(".codex-p");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"p\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-p\"\n",
        )
        .unwrap();

        let err = remove_profile(&config, tmp.path(), "p", true, None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("symlink"), "must reject symlink: {msg}");
        // Symlink itself still exists (not followed/removed).
        assert!(link.symlink_metadata().is_ok());
        assert!(real.join("canary").exists(), "real target untouched");
    }

    #[test]
    fn remove_profile_marks_active_when_env_matches() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();

        let outcome = remove_profile(&config, tmp.path(), "work", false, Some("work")).unwrap();
        assert!(outcome.was_active);

        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();
        let outcome2 =
            remove_profile(&config, tmp.path(), "work", false, Some("personal")).unwrap();
        assert!(!outcome2.was_active);
    }

    #[test]
    fn remove_profile_missing_returns_available_list_error() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();

        let err = remove_profile(&config, tmp.path(), "ghost", false, None).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ghost"));
        assert!(msg.contains("work"));
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
        let dir = std::env::temp_dir().join(format!("aiwitch-remove-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
