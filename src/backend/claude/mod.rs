use super::{Backend, BackendKind, LoginMode, ProfileMeta, ProviderCommand, ProvisionOptions};
use crate::error::{Context, Result};
use crate::profile::Profile;
use anyhow::{anyhow, bail, ensure};
use chrono::{DateTime, Utc};

pub mod auth;
pub mod keychain;

pub struct ClaudeBackend;

/** Threshold above which an `expiresAt` integer is interpreted as Unix epoch
 *  milliseconds. Below this, it is treated as seconds. 10^12 gives us a wide
 *  safety margin: timestamps in seconds stay below ~10^10 well into year 2286,
 *  while millisecond timestamps for any plausible date are above 10^12. The
 *  absolute value keeps negative test fixtures unit-symmetric around zero. */
const MS_THRESHOLD: i64 = 1_000_000_000_000;

impl Backend for ClaudeBackend {
    fn id(&self) -> BackendKind {
        BackendKind::Claude
    }

    fn env_exports(&self, profile: &Profile) -> Result<Vec<(String, String)>> {
        ensure!(
            profile.backend == BackendKind::Claude,
            "ClaudeBackend received non-claude profile {:?}",
            profile.name
        );
        let home = profile
            .home_dir
            .to_str()
            .with_context(|| {
                format!(
                    "profile {:?} home_dir is not valid UTF-8: {}",
                    profile.name,
                    profile.home_dir.display()
                )
            })?
            .to_string();
        ensure!(
            profile.home_dir.is_absolute(),
            "profile {:?} home_dir must be absolute: {}",
            profile.name,
            profile.home_dir.display()
        );
        Ok(vec![("CLAUDE_CONFIG_DIR".to_string(), home)])
    }

    fn read_meta(&self, profile: &Profile) -> Result<ProfileMeta> {
        ensure!(
            profile.backend == BackendKind::Claude,
            "ClaudeBackend received non-claude profile {:?}",
            profile.name
        );
        // Best-effort: on macOS the credentials live in Keychain, so the file
        // is usually absent. Treat any read/parse failure as "no metadata"
        // rather than surfacing an error to the list-table renderer. The
        // parser itself preserves errors for direct callers and unit tests.
        let Ok(creds) = auth::read(&profile.home_dir) else {
            return Ok(ProfileMeta::default());
        };

        let oauth = creds.claude_ai_oauth.unwrap_or_default();
        let plan = oauth.subscription_type;
        let subscription_until = oauth.expires_at.and_then(timestamp_to_datetime);

        Ok(ProfileMeta {
            email: oauth.email,
            plan,
            subscription_until,
        })
    }

    fn login_command(&self, profile: &Profile, mode: LoginMode) -> Result<ProviderCommand> {
        match mode {
            LoginMode::Interactive => Ok(ProviderCommand {
                program: "claude".to_string(),
                args: vec![],
                envs: self.env_exports(profile)?,
            }),
            LoginMode::ApiKey => Err(anyhow!(
                "Claude API-key login is not supported.\nhint: run `aiwitch add claude {}` and use `/login` inside the claude TUI",
                profile.name
            )),
        }
    }

    fn normalize_api_key<'a>(&self, _input: &'a str) -> Result<&'a str> {
        bail!("Claude API-key login is not supported (interactive `/login` only)")
    }

    fn provision(&self, profile: &Profile, options: ProvisionOptions) -> Result<()> {
        ensure!(
            profile.backend == BackendKind::Claude,
            "ClaudeBackend received non-claude profile {:?}",
            profile.name
        );
        // Reject `--auth` for Claude *before* any filesystem mutation so a
        // mistyped invocation cannot leave a half-provisioned home dir behind.
        ensure!(
            options.auth_mode.is_none(),
            "Claude does not support `--auth` (interactive `/login` only)"
        );
        prepare_home_dir(&profile.home_dir)
    }
}

pub(crate) fn timestamp_to_datetime(value: i64) -> Option<DateTime<Utc>> {
    if value.abs() >= MS_THRESHOLD {
        DateTime::<Utc>::from_timestamp_millis(value)
    } else {
        DateTime::<Utc>::from_timestamp(value, 0)
    }
}

fn prepare_home_dir(path: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    restrict_dir_permissions(path)
}

#[cfg(unix)]
fn restrict_dir_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to restrict permissions for {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use std::path::{Path, PathBuf};

    fn profile(name: &str, home: &str) -> Profile {
        Profile {
            name: name.to_string(),
            backend: BackendKind::Claude,
            home_dir: PathBuf::from(home),
        }
    }

    #[test]
    fn env_exports_returns_claude_config_dir() {
        let p = profile("personal", "/Users/x/.claude-personal");
        let exports = ClaudeBackend.env_exports(&p).unwrap();
        assert_eq!(
            exports,
            vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/Users/x/.claude-personal".to_string()
            )]
        );
    }

    #[test]
    fn env_exports_preserves_spaces_in_path() {
        let p = profile("p", "/Users/x/with space/.claude");
        let exports = ClaudeBackend.env_exports(&p).unwrap();
        assert_eq!(exports[0].1, "/Users/x/with space/.claude");
    }

    #[test]
    fn env_exports_rejects_relative_home_dir() {
        let p = profile("p", "rel/claude");
        assert!(ClaudeBackend.env_exports(&p).is_err());
    }

    #[test]
    fn env_exports_rejects_non_claude_profile() {
        let p = Profile {
            name: "p".to_string(),
            backend: BackendKind::Codex,
            home_dir: PathBuf::from("/abs/codex"),
        };
        assert!(ClaudeBackend.env_exports(&p).is_err());
    }

    #[test]
    fn login_command_uses_claude_config_dir_for_interactive() {
        let p = profile("p", "/abs/.claude-p");

        let command = ClaudeBackend
            .login_command(&p, LoginMode::Interactive)
            .unwrap();

        assert_eq!(command.program, "claude");
        assert!(command.args.is_empty());
        assert_eq!(
            command.envs,
            vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/abs/.claude-p".to_string()
            )]
        );
    }

    #[test]
    fn login_command_rejects_api_key_mode() {
        let p = profile("p", "/abs/.claude-p");

        let err = ClaudeBackend
            .login_command(&p, LoginMode::ApiKey)
            .unwrap_err();

        let msg = format!("{err}");
        assert!(msg.contains("not supported"));
        assert!(msg.contains("/login"));
    }

    #[test]
    fn normalize_api_key_rejects_unsupported_api_key_login() {
        let err = ClaudeBackend
            .normalize_api_key("sk-ant-api03-test1234567890")
            .unwrap_err();

        assert!(format!("{err}").contains("not supported"));
    }

    #[test]
    fn provision_creates_dir_with_0700_and_no_config_files() {
        let tmp = tempdir();
        let p = profile("p", tmp.path().to_str().unwrap());

        ClaudeBackend
            .provision(&p, ProvisionOptions { auth_mode: None })
            .unwrap();

        assert!(tmp.path().is_dir());
        // Claude Code manages its own config files; aiwitch must not create any.
        assert!(!tmp.path().join("config.toml").exists());
        assert!(!tmp.path().join(".credentials.json").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn provision_with_auth_mode_set_returns_error_before_dir_creation() {
        let tmp_root = tempdir();
        let target = tmp_root.path().join("not-yet-created");
        let p = profile("p", target.to_str().unwrap());

        let err = ClaudeBackend
            .provision(
                &p,
                ProvisionOptions {
                    auth_mode: Some(LoginMode::Interactive),
                },
            )
            .unwrap_err();

        let msg = format!("{err}");
        assert!(msg.contains("does not support"));
        assert!(
            !target.exists(),
            "directory must not be created on rejection"
        );
    }

    #[test]
    fn read_meta_on_missing_credentials_returns_default_meta() {
        let tmp = tempdir();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = ClaudeBackend.read_meta(&p).unwrap();

        assert!(meta.email.is_none());
        assert!(meta.plan.is_none());
        assert!(meta.subscription_until.is_none());
    }

    #[test]
    fn read_meta_maps_subscription_type_to_plan() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"subscriptionType":"max","email":"u@e.com"}}"#,
        )
        .unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = ClaudeBackend.read_meta(&p).unwrap();

        assert_eq!(meta.email.as_deref(), Some("u@e.com"));
        assert_eq!(meta.plan.as_deref(), Some("max"));
        assert!(meta.subscription_until.is_none());
    }

    #[test]
    fn read_meta_handles_expires_at_in_milliseconds() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"expiresAt":1735689600000}}"#,
        )
        .unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = ClaudeBackend.read_meta(&p).unwrap();

        let dt = meta.subscription_until.expect("expected datetime");
        // 1735689600000 ms = 2025-01-01T00:00:00Z
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2025-01-01");
    }

    #[test]
    fn read_meta_handles_expires_at_in_seconds() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"expiresAt":1735689600}}"#,
        )
        .unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = ClaudeBackend.read_meta(&p).unwrap();

        let dt = meta.subscription_until.expect("expected datetime");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2025-01-01");
    }

    #[test]
    fn timestamp_to_datetime_handles_unit_boundaries() {
        assert_eq!(timestamp_to_datetime(0).unwrap().timestamp_millis(), 0);
        assert_eq!(
            timestamp_to_datetime(MS_THRESHOLD - 1).unwrap().timestamp(),
            MS_THRESHOLD - 1
        );
        assert_eq!(
            timestamp_to_datetime(MS_THRESHOLD)
                .unwrap()
                .timestamp_millis(),
            MS_THRESHOLD
        );
        assert_eq!(
            timestamp_to_datetime(-(MS_THRESHOLD - 1))
                .unwrap()
                .timestamp(),
            -(MS_THRESHOLD - 1)
        );
        assert_eq!(
            timestamp_to_datetime(-MS_THRESHOLD)
                .unwrap()
                .timestamp_millis(),
            -MS_THRESHOLD
        );
    }

    #[test]
    fn read_meta_swallows_malformed_json() {
        let tmp = tempdir();
        std::fs::write(tmp.path().join(".credentials.json"), "{ not json").unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        // read_meta is best-effort; the list-table renderer should not break
        // on a corrupt credentials file. Direct callers of `auth::parse`
        // still see the error.
        let meta = ClaudeBackend.read_meta(&p).unwrap();
        assert!(meta.email.is_none() && meta.plan.is_none());
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
        let dir = std::env::temp_dir().join(format!("aiwitch-claude-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
