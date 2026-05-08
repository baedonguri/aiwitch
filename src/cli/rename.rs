use crate::backend::BackendKind;
use crate::error::{Context, Result};
use crate::profile::{ProfilesFile, store};
use crate::shell::validate_profile_name;
use anyhow::anyhow;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(old: &str, new: &str) -> Result<()> {
    let config_path = store::config_path()?;
    let home_base = store::expand_home_dir(Path::new("~"))?;
    let env_current = std::env::var(super::current::AIWITCH_CURRENT_KEY).ok();

    let outcome = rename_profile(&config_path, &home_base, old, new, env_current.as_deref())?;

    if outcome.old_was_symlink {
        eprintln!(
            "warning: old home_dir was a symlink; the symlink itself was moved, not its target"
        );
    }
    if outcome.was_active {
        eprintln!(
            "warning: AIWITCH_CURRENT is still {:?} in this shell;\n         run `aiwitch use {}` or start a new shell",
            outcome.old, outcome.new
        );
    }

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "renamed {} -> {}", outcome.old, outcome.new)?;
    if let Some(home) = &outcome.new_home_str {
        writeln!(stdout, "home_dir: {home}")?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct RenameOutcome {
    pub old: String,
    pub new: String,
    /** Some when the profile used the default `~/.<provider>-<name>` pattern
     *  and the `home_dir` was renamed both in TOML and on disk. */
    pub new_home_str: Option<String>,
    pub was_active: bool,
    /** True when the moved entry's `home_dir` was a symlink — `fs::rename`
     *  moves the link itself, not the target. The CLI surfaces this as a
     *  warning so users notice when their credentials directory is unaffected. */
    pub old_was_symlink: bool,
}

#[derive(Debug)]
pub struct RenameTextOutcome {
    pub updated_text: String,
    pub backend: BackendKind,
    pub new_home_str: String,
    /** True when `<old>`'s raw `home_dir` matched `default_home(provider, old)`,
     *  so the `home_dir` string was rewritten and the directory should also be
     *  renamed on disk. */
    pub renamed_default_home: bool,
}

pub fn rename_profile(
    config_path: &Path,
    home_base: &Path,
    old: &str,
    new: &str,
    env_current: Option<&str>,
) -> Result<RenameOutcome> {
    /* Read the raw TOML text directly. `store::load_from` would expand `~` to
     * an absolute path, which destroys the default-pattern detection that
     * `rename_profile_in_text` relies on (`raw_home == "~/.codex-<old>"`). */
    let existing = std::fs::read_to_string(config_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "no profiles file at {} (no profiles configured)",
                config_path.display()
            )
        } else {
            anyhow::Error::new(e).context(format!("failed to read {}", config_path.display()))
        }
    })?;

    let text_outcome = rename_profile_in_text(&existing, old, new)?;

    let mut old_was_symlink = false;
    if text_outcome.renamed_default_home {
        let old_resolved = store::expand_home_dir_in(
            Path::new(&default_home(text_outcome.backend, old)),
            home_base,
        )?;
        let new_resolved =
            store::expand_home_dir_in(Path::new(&text_outcome.new_home_str), home_base)?;

        // `symlink_metadata` so a dangling symlink at the target also blocks
        // the rename instead of being silently overwritten by `fs::rename`.
        if new_resolved.symlink_metadata().is_ok() {
            return Err(anyhow!(
                "target home_dir already exists: {}\nhint: remove or rename it before renaming the profile",
                new_resolved.display()
            ));
        }
        let old_meta = old_resolved.symlink_metadata().ok();
        old_was_symlink = old_meta
            .as_ref()
            .is_some_and(|m| m.file_type().is_symlink());
        if old_meta.is_some() {
            if let Err(e) = std::fs::rename(&old_resolved, &new_resolved) {
                // EXDEV (cross-filesystem rename) is the most actionable
                // failure mode: explain it instead of leaking the raw libc msg.
                let cross_device = e.kind() == std::io::ErrorKind::CrossesDevices;
                let ctx = if cross_device {
                    format!(
                        "cannot rename home_dir across filesystems: {} -> {}\nhint: move the directory to the same filesystem (or onto the same volume as $HOME) and re-run, or use a custom `home_dir` and rename manually",
                        old_resolved.display(),
                        new_resolved.display()
                    )
                } else {
                    format!(
                        "failed to rename home_dir {} -> {}",
                        old_resolved.display(),
                        new_resolved.display()
                    )
                };
                return Err(anyhow::Error::new(e).context(ctx));
            }
        }
        store::write_atomic(config_path, &text_outcome.updated_text).with_context(|| {
            format!(
                "home_dir was renamed to {} but {} could not be updated; manually edit it to set name = {:?} and home_dir = {:?}",
                new_resolved.display(),
                config_path.display(),
                new,
                text_outcome.new_home_str
            )
        })?;
    } else {
        store::write_atomic(config_path, &text_outcome.updated_text)?;
    }

    Ok(RenameOutcome {
        old: old.to_string(),
        new: new.to_string(),
        new_home_str: if text_outcome.renamed_default_home {
            Some(text_outcome.new_home_str)
        } else {
            None
        },
        was_active: env_current == Some(old),
        old_was_symlink,
    })
}

pub fn rename_profile_in_text(existing: &str, old: &str, new: &str) -> Result<RenameTextOutcome> {
    validate_profile_name(new)?;
    if old == new {
        return Err(anyhow!("old and new profile names are identical: {old:?}"));
    }

    let mut parsed: ProfilesFile = if existing.trim().is_empty() {
        ProfilesFile::default()
    } else {
        toml::from_str(existing).context("failed to parse existing profiles TOML")?
    };

    let mut found_idx: Option<usize> = None;
    for (i, p) in parsed.profiles.iter().enumerate() {
        if p.name == new {
            return Err(anyhow!(
                "profile {new:?} already exists\nhint: pick a different name, or remove it first with `aiwitch remove {new}`"
            ));
        }
        if p.name == old {
            found_idx = Some(i);
        }
    }
    let idx = found_idx.ok_or_else(|| {
        let available: Vec<&str> = parsed.profiles.iter().map(|p| p.name.as_str()).collect();
        if available.is_empty() {
            anyhow!("no profile named {old:?} (no profiles configured)")
        } else {
            anyhow!(
                "no profile named {old:?}. available: {}",
                available.join(", ")
            )
        }
    })?;

    let backend = parsed.profiles[idx].backend;
    let raw_home = parsed.profiles[idx]
        .home_dir
        .to_str()
        .ok_or_else(|| anyhow!("home_dir is not valid UTF-8 for profile {old:?}"))?
        .to_string();

    let renamed_default_home = raw_home == default_home(backend, old);
    let new_home_str = if renamed_default_home {
        default_home(backend, new)
    } else {
        raw_home
    };

    parsed.profiles[idx].name = new.to_string();
    parsed.profiles[idx].home_dir = PathBuf::from(&new_home_str);

    let updated_text = toml::to_string(&parsed)?;

    Ok(RenameTextOutcome {
        updated_text,
        backend,
        new_home_str,
        renamed_default_home,
    })
}

fn default_home(backend: BackendKind, name: &str) -> String {
    let prefix = match backend {
        BackendKind::Codex => "codex",
        BackendKind::Claude => "claude",
    };
    format!("~/.{prefix}-{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX: BackendKind = BackendKind::Codex;
    const CLAUDE: BackendKind = BackendKind::Claude;

    #[test]
    fn rename_text_default_pattern_rewrites_home_dir() {
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"~/.codex-work\"
";
        let got = rename_profile_in_text(existing, "work", "office").unwrap();
        assert_eq!(got.backend, CODEX);
        assert!(got.renamed_default_home);
        assert_eq!(got.new_home_str, "~/.codex-office");
        assert_eq!(
            got.updated_text,
            "[[profiles]]\nname = \"office\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-office\"\n"
        );
    }

    #[test]
    fn rename_text_claude_default_pattern_rewrites() {
        let existing =
            "[[profiles]]\nname = \"p\"\nbackend = \"claude\"\nhome_dir = \"~/.claude-p\"\n";
        let got = rename_profile_in_text(existing, "p", "q").unwrap();
        assert_eq!(got.backend, CLAUDE);
        assert!(got.renamed_default_home);
        assert_eq!(got.new_home_str, "~/.claude-q");
    }

    #[test]
    fn rename_text_custom_path_keeps_home_dir() {
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"/abs/codex-secret\"
";
        let got = rename_profile_in_text(existing, "work", "office").unwrap();
        assert!(!got.renamed_default_home);
        assert_eq!(got.new_home_str, "/abs/codex-secret");
        assert_eq!(
            got.updated_text,
            "[[profiles]]\nname = \"office\"\nbackend = \"codex\"\nhome_dir = \"/abs/codex-secret\"\n"
        );
    }

    #[test]
    fn rename_text_default_pattern_uses_raw_string_not_expanded_path() {
        // raw `~/.codex-work` must match the default pattern even though
        // load_from would expand it to an absolute path.
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"~/.codex-work\"
";
        let got = rename_profile_in_text(existing, "work", "office").unwrap();
        assert!(got.renamed_default_home);
    }

    #[test]
    fn rename_text_rejects_old_eq_new() {
        let existing =
            "[[profiles]]\nname = \"x\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-x\"\n";
        let err = rename_profile_in_text(existing, "x", "x").unwrap_err();
        assert!(format!("{err}").contains("identical"));
    }

    #[test]
    fn rename_text_rejects_invalid_new_name() {
        let existing =
            "[[profiles]]\nname = \"x\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-x\"\n";
        assert!(rename_profile_in_text(existing, "x", "with.dot").is_err());
        assert!(rename_profile_in_text(existing, "x", "-foo").is_err());
        assert!(rename_profile_in_text(existing, "x", "").is_err());
    }

    #[test]
    fn rename_text_rejects_collision_with_existing_profile() {
        let existing = "\
[[profiles]]
name = \"work\"
backend = \"codex\"
home_dir = \"~/.codex-work\"

[[profiles]]
name = \"office\"
backend = \"codex\"
home_dir = \"~/.codex-office\"
";
        let err = rename_profile_in_text(existing, "work", "office").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("already exists"));
        assert!(msg.contains("office"));
    }

    #[test]
    fn rename_text_rejects_missing_old_with_available_list() {
        let existing =
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n";
        let err = rename_profile_in_text(existing, "ghost", "new").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ghost"));
        assert!(msg.contains("work"));
    }

    #[test]
    fn rename_text_missing_old_in_empty_file() {
        let err = rename_profile_in_text("", "ghost", "new").unwrap_err();
        assert!(format!("{err}").contains("no profiles configured"));
    }

    #[test]
    fn rename_profile_with_default_pattern_moves_dir_and_updates_toml() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();
        let old_home = tmp.path().join(".codex-work");
        std::fs::create_dir_all(old_home.join("nested")).unwrap();
        std::fs::write(old_home.join("nested/auth.json"), "secret").unwrap();

        let outcome = rename_profile(&config, tmp.path(), "work", "office", None).unwrap();

        assert_eq!(outcome.new_home_str.as_deref(), Some("~/.codex-office"));
        let new_home = tmp.path().join(".codex-office");
        assert!(!old_home.exists(), "old home_dir must be moved away");
        assert!(new_home.exists());
        assert_eq!(
            std::fs::read_to_string(new_home.join("nested/auth.json")).unwrap(),
            "secret"
        );
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "[[profiles]]\nname = \"office\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-office\"\n"
        );
    }

    #[test]
    fn rename_profile_with_custom_home_dir_only_updates_toml() {
        let tmp = tempdir();
        let custom = tmp.path().join("custom-home");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("canary"), "still here").unwrap();

        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            format!(
                "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"{}\"\n",
                custom.display()
            ),
        )
        .unwrap();

        let outcome = rename_profile(&config, tmp.path(), "work", "office", None).unwrap();
        assert!(outcome.new_home_str.is_none());
        assert!(custom.is_dir(), "custom dir untouched");
        assert!(custom.join("canary").exists());
        let toml_after = std::fs::read_to_string(&config).unwrap();
        assert!(toml_after.contains("name = \"office\""));
        assert!(toml_after.contains(&format!("home_dir = \"{}\"", custom.display())));
    }

    #[test]
    fn rename_profile_default_pattern_rejects_when_target_dir_exists() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();
        let old_home = tmp.path().join(".codex-work");
        std::fs::create_dir_all(&old_home).unwrap();
        let new_home = tmp.path().join(".codex-office");
        std::fs::create_dir_all(&new_home).unwrap();
        std::fs::write(new_home.join("preexisting"), "do not clobber").unwrap();

        let err = rename_profile(&config, tmp.path(), "work", "office", None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already exists"));
        // Nothing must have moved.
        assert!(old_home.exists());
        assert_eq!(
            std::fs::read_to_string(new_home.join("preexisting")).unwrap(),
            "do not clobber"
        );
        // TOML must be unchanged (rename is "dir-first; if rejected, no toml write").
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("name = \"work\"")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_profile_default_pattern_moves_symlink_and_flags_outcome() {
        // When `home_dir` is itself a symlink, `fs::rename` moves the link;
        // the credentials at the link target stay put. The outcome must flag
        // this so the CLI can warn the user.
        let tmp = tempdir();
        let real_target = tmp.path().join("creds-stash");
        std::fs::create_dir_all(&real_target).unwrap();
        std::fs::write(real_target.join("auth"), "secret").unwrap();

        let old_home = tmp.path().join(".codex-work");
        std::os::unix::fs::symlink(&real_target, &old_home).unwrap();

        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();

        let outcome = rename_profile(&config, tmp.path(), "work", "office", None).unwrap();

        assert!(outcome.old_was_symlink, "must flag symlink in outcome");
        assert_eq!(outcome.new_home_str.as_deref(), Some("~/.codex-office"));
        let new_home = tmp.path().join(".codex-office");
        assert!(
            new_home
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(real_target.join("auth")).unwrap(),
            "secret",
            "real target must be untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_profile_default_pattern_rejects_when_target_is_dangling_symlink() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();
        let old_home = tmp.path().join(".codex-work");
        std::fs::create_dir_all(&old_home).unwrap();
        let dangling = tmp.path().join(".codex-office");
        std::os::unix::fs::symlink(tmp.path().join("does-not-exist"), &dangling).unwrap();

        let err = rename_profile(&config, tmp.path(), "work", "office", None).unwrap_err();
        assert!(format!("{err:#}").contains("already exists"));
        // Old dir untouched, dangling symlink still present, TOML unchanged.
        assert!(old_home.exists());
        assert!(dangling.symlink_metadata().is_ok());
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("name = \"work\"")
        );
    }

    #[test]
    fn rename_profile_default_pattern_works_when_old_dir_missing() {
        // User manually deleted the home_dir, but TOML still has it. Rename
        // should still update the TOML.
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n",
        )
        .unwrap();

        let outcome = rename_profile(&config, tmp.path(), "work", "office", None).unwrap();
        assert_eq!(outcome.new_home_str.as_deref(), Some("~/.codex-office"));
        assert!(std::fs::read_to_string(&config).unwrap().contains("office"));
    }

    #[test]
    fn rename_profile_marks_active_when_env_matches_old() {
        let tmp = tempdir();
        let config = tmp.path().join("profiles.toml");
        std::fs::write(
            &config,
            "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"/abs/work\"\n",
        )
        .unwrap();

        let outcome = rename_profile(&config, tmp.path(), "work", "office", Some("work")).unwrap();
        assert!(outcome.was_active);

        std::fs::write(
            &config,
            "[[profiles]]\nname = \"office\"\nbackend = \"codex\"\nhome_dir = \"/abs/work\"\n",
        )
        .unwrap();
        let outcome2 =
            rename_profile(&config, tmp.path(), "office", "another", Some("personal")).unwrap();
        assert!(!outcome2.was_active);
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
        let dir = std::env::temp_dir().join(format!("aiwitch-rename-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
