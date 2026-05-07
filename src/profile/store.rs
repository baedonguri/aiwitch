use super::ProfilesFile;
use crate::error::{Context, Result};
use crate::shell::validate_profile_name;
use anyhow::{anyhow, ensure};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/** Always `$HOME/.config/aiwitch/profiles.toml`, on every OS. */
pub fn config_path() -> Result<PathBuf> {
    let home = read_home()?;
    Ok(config_path_for(&home))
}

/** Pure variant of `config_path` used by tests; takes `home` as input. */
pub fn config_path_for(home: &Path) -> PathBuf {
    home.join(".config").join("aiwitch").join("profiles.toml")
}

/** Expand a leading `~` or `~/...` against `$HOME`. Other forms are returned unchanged. */
pub fn expand_home_dir(path: &Path) -> Result<PathBuf> {
    let home = read_home()?;
    expand_home_dir_in(path, &home)
}

/** Pure variant of `expand_home_dir`. Accepts only `~`, `~/...`, or absolute paths;
 *  relative paths and `~user/...` are rejected so the resolved value is always absolute. */
pub fn expand_home_dir_in(path: &Path, home: &Path) -> Result<PathBuf> {
    let s = path
        .to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))?;

    if s == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    ensure!(
        !s.starts_with('~'),
        "~user/ style paths are not supported: {s:?}"
    );
    let pb = PathBuf::from(s);
    ensure!(
        pb.is_absolute(),
        "home_dir must be absolute or start with `~`: {s:?}"
    );
    Ok(pb)
}

/** Loads the config file, expands `~` in each `home_dir`, and returns the result. */
pub fn load() -> Result<ProfilesFile> {
    let home = read_home()?;
    let path = config_path_for(&home);
    load_from(&path, &home)
}

/** Pure variant of `load`. Reads the given file and resolves `~` against the given home. */
pub fn load_from(path: &Path, home: &Path) -> Result<ProfilesFile> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "no profiles file at {}\n\ncreate one with this template:\n\n{}",
                path.display(),
                sample_toml()
            )
        } else {
            anyhow::Error::new(e).context(format!("failed to read {}", path.display()))
        }
    })?;

    let mut parsed: ProfilesFile = toml::from_str(&text)
        .with_context(|| format!("failed to parse TOML at {}", path.display()))?;

    let mut seen: HashSet<&str> = HashSet::new();
    for p in &parsed.profiles {
        validate_profile_name(&p.name)
            .with_context(|| format!("invalid profile name in {}", path.display()))?;
        if !seen.insert(p.name.as_str()) {
            return Err(anyhow!(
                "duplicate profile name {:?} in {}",
                p.name,
                path.display()
            ));
        }
    }

    for p in &mut parsed.profiles {
        p.home_dir = expand_home_dir_in(&p.home_dir, home)
            .with_context(|| format!("invalid home_dir for profile {:?}", p.name))?;
    }

    Ok(parsed)
}

fn read_home() -> Result<PathBuf> {
    let h = std::env::var("HOME").map_err(|_| anyhow!("$HOME is not set"))?;
    ensure!(!h.is_empty(), "$HOME is empty");
    Ok(PathBuf::from(h))
}

#[allow(dead_code)]
pub fn sample_toml() -> &'static str {
    r#"# ~/.config/aiwitch/profiles.toml
[[profiles]]
name = "personal"
backend = "codex"
home_dir = "~/.codex-personal"

[[profiles]]
name = "work"
backend = "codex"
home_dir = "~/.codex-work"

[[profiles]]
name = "claude-personal"
backend = "claude"
home_dir = "~/.claude-personal"
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;

    #[test]
    fn config_path_joins_under_home() {
        let home = PathBuf::from("/Users/x");
        assert_eq!(
            config_path_for(&home),
            PathBuf::from("/Users/x/.config/aiwitch/profiles.toml")
        );
    }

    #[test]
    fn config_path_with_space_in_home() {
        let home = PathBuf::from("/Users/with space");
        assert_eq!(
            config_path_for(&home),
            PathBuf::from("/Users/with space/.config/aiwitch/profiles.toml")
        );
    }

    #[test]
    fn expand_tilde_alone() {
        let home = PathBuf::from("/Users/x");
        let got = expand_home_dir_in(Path::new("~"), &home).unwrap();
        assert_eq!(got, PathBuf::from("/Users/x"));
    }

    #[test]
    fn expand_tilde_slash() {
        let home = PathBuf::from("/Users/x");
        let got = expand_home_dir_in(Path::new("~/.codex-personal"), &home).unwrap();
        assert_eq!(got, PathBuf::from("/Users/x/.codex-personal"));
    }

    #[test]
    fn expand_absolute_unchanged() {
        let home = PathBuf::from("/Users/x");
        let got = expand_home_dir_in(Path::new("/etc/hosts"), &home).unwrap();
        assert_eq!(got, PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn expand_rejects_relative() {
        let home = PathBuf::from("/Users/x");
        assert!(expand_home_dir_in(Path::new("rel/path"), &home).is_err());
        assert!(expand_home_dir_in(Path::new("./foo"), &home).is_err());
        assert!(expand_home_dir_in(Path::new(""), &home).is_err());
    }

    #[test]
    fn expand_rejects_tilde_user() {
        let home = PathBuf::from("/Users/x");
        assert!(expand_home_dir_in(Path::new("~root/foo"), &home).is_err());
        assert!(expand_home_dir_in(Path::new("~alice"), &home).is_err());
    }

    #[test]
    fn expand_does_not_touch_internal_tilde() {
        let home = PathBuf::from("/Users/x");
        let got = expand_home_dir_in(Path::new("/tmp/~backup"), &home).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/~backup"));
    }

    #[test]
    fn load_from_missing_file_has_helpful_message() {
        let tmp = tempdir();
        let missing = tmp.path().join("does-not-exist.toml");
        let err = load_from(&missing, Path::new("/Users/x")).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("no profiles file at"));
        assert!(msg.contains("[[profiles]]"));
    }

    #[test]
    fn load_from_parses_valid_file_and_expands_tildes() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "personal"
backend = "codex"
home_dir = "~/.codex-personal"

[[profiles]]
name = "work"
backend = "codex"
home_dir = "/abs/codex-work"
"#,
        )
        .unwrap();

        let pf = load_from(&path, Path::new("/Users/x")).unwrap();
        assert_eq!(pf.profiles.len(), 2);
        assert_eq!(pf.profiles[0].name, "personal");
        assert_eq!(pf.profiles[0].backend, BackendKind::Codex);
        assert_eq!(
            pf.profiles[0].home_dir,
            PathBuf::from("/Users/x/.codex-personal")
        );
        assert_eq!(pf.profiles[1].home_dir, PathBuf::from("/abs/codex-work"));
    }

    #[test]
    fn load_from_empty_file_is_ok() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(&path, "").unwrap();
        let pf = load_from(&path, Path::new("/Users/x")).unwrap();
        assert!(pf.profiles.is_empty());
    }

    #[test]
    fn load_from_rejects_invalid_toml() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(&path, "this is = not = valid toml [[[").unwrap();
        let err = load_from(&path, Path::new("/Users/x")).unwrap_err();
        assert!(format!("{err}").contains("failed to parse TOML"));
    }

    #[test]
    fn load_from_rejects_unknown_backend() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "x"
backend = "gemini"
home_dir = "~/.gemini"
"#,
        )
        .unwrap();
        assert!(load_from(&path, Path::new("/Users/x")).is_err());
    }

    #[test]
    fn load_from_rejects_missing_required_field() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "x"
backend = "codex"
"#,
        )
        .unwrap();
        assert!(load_from(&path, Path::new("/Users/x")).is_err());
    }

    #[test]
    fn load_from_propagates_tilde_user_error_with_profile_name() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "bad"
backend = "codex"
home_dir = "~root/x"
"#,
        )
        .unwrap();
        let err = load_from(&path, Path::new("/Users/x")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("bad"));
    }

    #[test]
    fn load_from_rejects_relative_home_dir() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "rel"
backend = "codex"
home_dir = "relative/dir"
"#,
        )
        .unwrap();
        let err = load_from(&path, Path::new("/Users/x")).unwrap_err();
        assert!(format!("{err:#}").contains("rel"));
    }

    #[test]
    fn load_from_rejects_invalid_profile_name() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "with.dot"
backend = "codex"
home_dir = "~/x"
"#,
        )
        .unwrap();
        assert!(load_from(&path, Path::new("/Users/x")).is_err());
    }

    #[test]
    fn load_from_rejects_leading_dash_profile_name() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "-foo"
backend = "codex"
home_dir = "~/x"
"#,
        )
        .unwrap();
        assert!(load_from(&path, Path::new("/Users/x")).is_err());
    }

    #[test]
    fn load_from_rejects_duplicate_profile_names() {
        let tmp = tempdir();
        let path = tmp.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profiles]]
name = "dup"
backend = "codex"
home_dir = "~/a"

[[profiles]]
name = "dup"
backend = "codex"
home_dir = "~/b"
"#,
        )
        .unwrap();
        let err = load_from(&path, Path::new("/Users/x")).unwrap_err();
        assert!(format!("{err}").contains("duplicate"));
    }

    /** RAII temp dir to avoid pulling in the `tempfile` crate; removes itself on drop. */
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
        let dir = std::env::temp_dir().join(format!("aiwitch-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
