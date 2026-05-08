use super::ProfilesFile;
use crate::error::{Context, Result};
use crate::shell::validate_profile_name;
use anyhow::{anyhow, ensure};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/** Write `contents` to `path` atomically by writing to a sibling temp file in
 *  the same parent directory, then `rename`-ing onto the target. Same-directory
 *  rename keeps the operation on a single filesystem so it stays atomic.
 *
 *  Does **not** call `fsync` on the temp file or its parent directory. After an
 *  OS-level crash between write and rename, the target may still hold the old
 *  contents or end up empty. This is an explicit trade-off: `profiles.toml` is
 *  small and re-runnable, so the cost of fsync isn't worth the durability gain.
 *  Callers writing real credentials should not reuse this helper. */
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow!(
            "cannot write to path without a parent directory: {}",
            path.display()
        )
    })?;
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let file_name = path.file_name().ok_or_else(|| {
        anyhow!(
            "cannot write to path without a file name: {}",
            path.display()
        )
    })?;
    let pid = std::process::id();
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".{pid}.{n}.tmp"));
    let tmp_path = parent.join(&tmp_name);

    if let Err(e) = std::fs::write(&tmp_path, contents) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(
            anyhow::Error::new(e).context(format!("failed to write {}", tmp_path.display()))
        );
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(anyhow::Error::new(e).context(format!(
            "failed to rename {} -> {}",
            tmp_path.display(),
            path.display()
        )));
    }
    Ok(())
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

    #[test]
    fn write_atomic_creates_target_file() {
        let tmp = tempdir();
        let target = tmp.path().join("profiles.toml");
        write_atomic(&target, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");
    }

    #[test]
    fn write_atomic_replaces_existing_file() {
        let tmp = tempdir();
        let target = tmp.path().join("profiles.toml");
        std::fs::write(&target, "old contents").unwrap();
        write_atomic(&target, "new contents").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new contents");
    }

    #[test]
    fn write_atomic_creates_missing_parent_dir() {
        let tmp = tempdir();
        let target = tmp.path().join("nested/dir/profiles.toml");
        write_atomic(&target, "hi").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");
    }

    #[test]
    fn write_atomic_does_not_leave_temp_files_behind_on_success() {
        let tmp = tempdir();
        let target = tmp.path().join("profiles.toml");
        write_atomic(&target, "ok").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "only the target file should remain");
        assert_eq!(leftovers[0], "profiles.toml");
    }

    #[test]
    fn write_atomic_failed_rename_does_not_corrupt_existing_target() {
        // Simulate a rename failure by making `target` a non-empty directory:
        // std::fs::rename(file, dir) fails on Unix, so the original directory
        // (and any sibling content) must be left intact.
        let tmp = tempdir();
        let target = tmp.path().join("profiles.toml");
        std::fs::create_dir(&target).unwrap();
        let canary = target.join("canary.txt");
        std::fs::write(&canary, "still here").unwrap();

        let err = write_atomic(&target, "new").unwrap_err();
        assert!(
            format!("{err:#}").contains("failed to rename"),
            "expected rename error, got {err:#}"
        );
        assert_eq!(std::fs::read_to_string(&canary).unwrap(), "still here");
        let siblings: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            siblings.len(),
            1,
            "no temp file should be left behind on failure"
        );
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
