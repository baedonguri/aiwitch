use super::{Backend, BackendKind, ProfileMeta, ProviderCommand};
use crate::error::{Context, Result};
use crate::profile::Profile;
use anyhow::ensure;

pub mod auth;
pub mod jwt;

pub struct CodexBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexLoginMode {
    Chatgpt,
    ApiKey,
}

pub fn login_command(profile: &Profile, mode: CodexLoginMode) -> Result<ProviderCommand> {
    let mut args = vec![
        "-c".to_string(),
        "cli_auth_credentials_store=\"file\"".to_string(),
        "login".to_string(),
    ];
    if mode == CodexLoginMode::ApiKey {
        args.push("--with-api-key".to_string());
    }

    Ok(ProviderCommand {
        program: "codex".to_string(),
        args,
        envs: CodexBackend.env_exports(profile)?,
    })
}

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

    fn read_meta(&self, profile: &Profile) -> Result<ProfileMeta> {
        ensure!(
            profile.backend == BackendKind::Codex,
            "CodexBackend received non-codex profile {:?}",
            profile.name
        );
        let auth_file = auth::read(&profile.home_dir)?;
        if auth_file.has_api_key() {
            return Ok(ProfileMeta {
                plan: Some("api-key".to_string()),
                ..ProfileMeta::default()
            });
        }
        let Some(tokens) = auth_file.tokens else {
            return Ok(ProfileMeta::default());
        };
        let payload = jwt::decode_payload(&tokens.id_token)?;
        let (plan, subscription_until) = payload
            .openai_auth
            .map(|a| (a.chatgpt_plan_type, a.chatgpt_subscription_active_until))
            .unwrap_or((None, None));
        Ok(ProfileMeta {
            email: payload.email,
            plan,
            subscription_until,
        })
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

    #[test]
    fn read_meta_chatgpt_tokens_none_returns_default_meta() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join("auth.json"),
            r#"{"auth_mode":"ChatGPT","tokens":null}"#,
        )
        .unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = CodexBackend.read_meta(&p).unwrap();

        assert!(meta.email.is_none());
        assert!(meta.plan.is_none());
        assert!(meta.subscription_until.is_none());
    }

    #[test]
    fn read_meta_apikey_mode_returns_api_key_plan() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join("auth.json"),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
        )
        .unwrap();
        let p = profile("p", tmp.path().to_str().unwrap());

        let meta = CodexBackend.read_meta(&p).unwrap();

        assert!(meta.email.is_none());
        assert_eq!(meta.plan.as_deref(), Some("api-key"));
        assert!(meta.subscription_until.is_none());
    }

    #[test]
    fn login_command_uses_codex_home_for_chatgpt_login() {
        let p = profile("p", "/abs/codex");

        let command = login_command(&p, CodexLoginMode::Chatgpt).unwrap();

        assert_eq!(command.program, "codex");
        assert_eq!(
            command.args,
            vec![
                "-c".to_string(),
                "cli_auth_credentials_store=\"file\"".to_string(),
                "login".to_string()
            ]
        );
        assert_eq!(
            command.envs,
            vec![("CODEX_HOME".to_string(), "/abs/codex".to_string())]
        );
    }

    #[test]
    fn login_command_adds_api_key_flag() {
        let p = profile("p", "/abs/codex");

        let command = login_command(&p, CodexLoginMode::ApiKey).unwrap();

        assert_eq!(
            command.args,
            vec![
                "-c".to_string(),
                "cli_auth_credentials_store=\"file\"".to_string(),
                "login".to_string(),
                "--with-api-key".to_string()
            ]
        );
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &std::path::Path {
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
        let dir = std::env::temp_dir().join(format!("aiwitch-codex-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
