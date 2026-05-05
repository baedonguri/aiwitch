use super::{Backend, BackendKind, LoginMode, ProfileMeta, ProviderCommand, ProvisionOptions};
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

    fn login_command(&self, profile: &Profile, mode: LoginMode) -> Result<ProviderCommand> {
        let mode = match mode {
            LoginMode::Interactive => CodexLoginMode::Chatgpt,
            LoginMode::ApiKey => CodexLoginMode::ApiKey,
        };
        login_command(profile, mode)
    }

    fn normalize_api_key<'a>(&self, input: &'a str) -> Result<&'a str> {
        normalize_api_key(input)
    }

    fn provision(&self, profile: &Profile, options: ProvisionOptions) -> Result<()> {
        ensure!(
            profile.backend == BackendKind::Codex,
            "CodexBackend received non-codex profile {:?}",
            profile.name
        );
        prepare_home_dir(&profile.home_dir)?;
        if let Some(auth_mode) = options.auth_mode {
            write_auth_config(&profile.home_dir, auth_mode)?;
        }
        Ok(())
    }
}

pub fn normalize_api_key(input: &str) -> Result<&str> {
    let key = input.trim();
    ensure!(!key.is_empty(), "OpenAI API key is empty");
    ensure!(
        !key.chars().any(char::is_whitespace),
        "OpenAI API key must not contain whitespace"
    );
    ensure!(
        !key.starts_with("sk-ant-"),
        "Codex API key login requires an OpenAI API key, not an Anthropic key"
    );
    ensure!(
        key.starts_with("sk-"),
        "Codex API key login requires an OpenAI API key (expected sk-... or sk-proj-...)"
    );
    Ok(key)
}

fn write_auth_config(codex_home: &std::path::Path, auth: LoginMode) -> Result<()> {
    prepare_home_dir(codex_home)?;
    let path = codex_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("failed to read {}", path.display())));
        }
    };

    let mut prefix = String::new();
    if !has_root_toml_key(&existing, "forced_login_method") {
        prefix.push_str(&format!(
            "forced_login_method = \"{}\"\n",
            auth_config_value(auth)
        ));
    }
    if !has_root_toml_key(&existing, "cli_auth_credentials_store") {
        prefix.push_str("cli_auth_credentials_store = \"file\"\n");
    }

    let updated = insert_root_toml_keys(prefix, existing);
    std::fs::write(&path, updated).with_context(|| format!("failed to write {}", path.display()))
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

fn auth_config_value(auth: LoginMode) -> &'static str {
    match auth {
        LoginMode::Interactive => "chatgpt",
        LoginMode::ApiKey => "api",
    }
}

fn has_root_toml_key(text: &str, key: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            return false;
        }
        if !trimmed.starts_with('#')
            && trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            return true;
        }
    }
    false
}

fn insert_root_toml_keys(prefix: String, existing: String) -> String {
    if prefix.is_empty() {
        return existing;
    }
    if existing.is_empty() {
        return prefix;
    }
    let Some(table_start) = first_toml_table_start(&existing) else {
        let separator = if existing.ends_with('\n') { "" } else { "\n" };
        return format!("{existing}{separator}{prefix}");
    };

    let (root, tables) = existing.split_at(table_start);
    if root.is_empty() {
        format!("{prefix}\n{tables}")
    } else if root.ends_with('\n') {
        format!("{root}{prefix}\n{tables}")
    } else {
        format!("{root}\n{prefix}\n{tables}")
    }
}

fn first_toml_table_start(text: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            return Some(offset);
        }
        offset += line.len();
    }
    None
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

    #[test]
    fn backend_login_command_maps_generic_api_key_mode() {
        let p = profile("p", "/abs/codex");

        let command = CodexBackend.login_command(&p, LoginMode::ApiKey).unwrap();

        assert!(command.args.contains(&"--with-api-key".to_string()));
    }

    #[test]
    fn backend_login_command_maps_generic_interactive_mode() {
        let p = profile("p", "/abs/codex");

        let command = CodexBackend
            .login_command(&p, LoginMode::Interactive)
            .unwrap();

        assert!(!command.args.contains(&"--with-api-key".to_string()));
    }

    #[test]
    fn normalize_api_key_rejects_anthropic_key() {
        let err = CodexBackend
            .normalize_api_key("sk-ant-api03-test")
            .unwrap_err();

        assert!(format!("{err}").contains("Anthropic"));
    }

    #[test]
    fn provision_writes_api_auth_config() {
        let tmp = tempdir();
        let p = profile("p", tmp.path().to_str().unwrap());

        CodexBackend
            .provision(
                &p,
                ProvisionOptions {
                    auth_mode: Some(LoginMode::ApiKey),
                },
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("config.toml")).unwrap(),
            "forced_login_method = \"api\"\ncli_auth_credentials_store = \"file\"\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn provision_writes_interactive_auth_config() {
        let tmp = tempdir();
        let p = profile("p", tmp.path().to_str().unwrap());

        CodexBackend
            .provision(
                &p,
                ProvisionOptions {
                    auth_mode: Some(LoginMode::Interactive),
                },
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("config.toml")).unwrap(),
            "forced_login_method = \"chatgpt\"\ncli_auth_credentials_store = \"file\"\n"
        );
    }

    #[test]
    fn provision_without_auth_mode_is_noop() {
        let tmp = tempdir();
        let p = profile("p", tmp.path().to_str().unwrap());

        CodexBackend
            .provision(&p, ProvisionOptions { auth_mode: None })
            .unwrap();

        assert!(!tmp.path().join("config.toml").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
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
