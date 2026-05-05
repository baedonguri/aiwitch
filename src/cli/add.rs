use crate::backend::{AnyBackend, Backend, BackendKind, LoginMode, ProvisionOptions};
use crate::error::{Context, Result};
use crate::profile::{Profile, ProfilesFile, store};
use crate::shell::{EnvFormat, render_env, validate_profile_name};
use anyhow::{anyhow, ensure};
use clap::ValueEnum;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(
    provider: BackendKind,
    profile: &str,
    home: Option<&Path>,
    auth: Option<CodexAuthMode>,
    print_env: bool,
    shell: EnvFormat,
) -> Result<()> {
    let config_path = store::config_path()?;
    let home_base = store::expand_home_dir(Path::new("~"))?;
    let outcome = add_to_config_with_auth(&config_path, &home_base, provider, profile, home, auth)?;

    // Default to ChatGPT login when `--auth` is omitted, so `aiwitch add <name>`
    // registers the profile AND authenticates it in one step.
    let effective_auth = auth.unwrap_or(CodexAuthMode::Chatgpt);
    spawn_provider_login(&outcome, effective_auth, print_env).with_context(|| {
        format!(
            "profile {:?} was added but login did not complete; retry with `aiwitch login {}`",
            outcome.profile, outcome.profile
        )
    })?;

    if print_env {
        let snippet = render_env(
            shell,
            &[
                (
                    "CODEX_HOME".to_string(),
                    outcome.expanded_home.to_string_lossy().into_owned(),
                ),
                ("AIWITCH_CURRENT".to_string(), outcome.profile.clone()),
            ],
        )?;
        print!("{snippet}");
        return Ok(());
    }

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
    if let Some(auth) = outcome.auth {
        writeln!(stdout, "auth: {}", auth.config_value())?;
    }
    Ok(())
}

fn spawn_provider_login(outcome: &AddOutcome, auth: CodexAuthMode, print_env: bool) -> Result<()> {
    let backend = AnyBackend::from_kind(outcome.provider);
    let profile = Profile {
        name: outcome.profile.clone(),
        backend: outcome.provider,
        home_dir: outcome.expanded_home.clone(),
    };
    let api_key_mode = matches!(auth, CodexAuthMode::Api);
    let spec = super::login::command_spec_for_profile(&backend, &profile, api_key_mode)?;
    let api_key_input = if api_key_mode {
        Some(super::login::read_api_key_from_stdin(&backend)?)
    } else {
        None
    };
    super::login::run_command(
        spec,
        api_key_input,
        &super::login::RunOptions {
            redirect_stdout_to_stderr: print_env,
            profile_name: Some(outcome.profile.clone()),
        },
    )
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CodexAuthMode {
    Chatgpt,
    Api,
}

impl CodexAuthMode {
    fn config_value(self) -> &'static str {
        match self {
            CodexAuthMode::Chatgpt => "chatgpt",
            CodexAuthMode::Api => "api",
        }
    }
}

impl From<CodexAuthMode> for LoginMode {
    fn from(auth: CodexAuthMode) -> Self {
        match auth {
            CodexAuthMode::Chatgpt => LoginMode::Interactive,
            CodexAuthMode::Api => LoginMode::ApiKey,
        }
    }
}

#[derive(Debug)]
struct AddOutcome {
    profile: String,
    provider: BackendKind,
    home_dir: String,
    expanded_home: PathBuf,
    auth: Option<CodexAuthMode>,
}

fn add_to_config_with_auth(
    config_path: &Path,
    home_base: &Path,
    provider: BackendKind,
    profile: &str,
    home: Option<&Path>,
    auth: Option<CodexAuthMode>,
) -> Result<AddOutcome> {
    let home_dir = match home {
        Some(path) => path
            .to_str()
            .ok_or_else(|| anyhow!("home path is not valid UTF-8: {}", path.display()))?
            .to_string(),
        None => default_home(provider, profile),
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
    let updated =
        add_profile_to_text_in(existing.as_deref(), home_base, provider, profile, &home_dir)?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let expanded_home = store::expand_home_dir_in(Path::new(&home_dir), home_base)?;
    std::fs::create_dir_all(&expanded_home)
        .with_context(|| format!("failed to create {}", expanded_home.display()))?;
    let provision_profile = Profile {
        name: profile.to_string(),
        backend: provider,
        home_dir: expanded_home.clone(),
    };
    AnyBackend::from_kind(provider).provision(
        &provision_profile,
        ProvisionOptions {
            auth_mode: auth.map(Into::into),
        },
    )?;
    std::fs::write(config_path, updated)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(AddOutcome {
        profile: profile.to_string(),
        provider,
        home_dir,
        expanded_home,
        auth,
    })
}

fn default_home(provider: BackendKind, profile: &str) -> String {
    let prefix = provider_home_prefix(provider);
    format!("~/.{prefix}-{profile}")
}

fn provider_home_prefix(provider: BackendKind) -> &'static str {
    match provider {
        BackendKind::Codex => "codex",
    }
}

fn add_profile_to_text_in(
    existing: Option<&str>,
    home_base: &Path,
    provider: BackendKind,
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
            "duplicate profile name {:?}\nhint: pick a different name, or run `aiwitch list` to see existing profiles",
            profile.name
        );
    }
    ensure!(
        !parsed.profiles.iter().any(|p| p.name == name),
        "duplicate profile name {name:?}\nhint: pick a different name, or run `aiwitch list` to see existing profiles"
    );

    let block = toml::to_string(&ProfilesFile {
        profiles: vec![Profile {
            name: name.to_string(),
            backend: provider,
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

    const CODEX: BackendKind = BackendKind::Codex;

    #[test]
    fn default_home_for_profile_uses_codex_prefix() {
        assert_eq!(default_home(CODEX, "codex_lemon"), "~/.codex-codex_lemon");
    }

    #[test]
    fn add_to_empty_creates_single_codex_profile() {
        let got = add_profile_to_text_in(
            None,
            Path::new("/home"),
            CODEX,
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
            CODEX,
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
            CODEX,
            "codex_lemon",
            "~/.codex-other",
        )
        .unwrap_err();

        let msg = format!("{err}");
        assert!(msg.contains("duplicate"));
        assert!(msg.contains("hint:"));
    }

    #[test]
    fn add_rejects_existing_duplicate_profile_names() {
        let existing = "[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work\"\n\n[[profiles]]\nname = \"work\"\nbackend = \"codex\"\nhome_dir = \"~/.codex-work-2\"\n";

        let err = add_profile_to_text_in(
            Some(existing),
            Path::new("/home"),
            CODEX,
            "codex_lemon",
            "~/.codex-lemon",
        )
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate"));
    }

    #[test]
    fn add_rejects_invalid_profile_name() {
        assert!(
            add_profile_to_text_in(None, Path::new("/home"), CODEX, "bad.name", "~/.codex-bad")
                .is_err()
        );
    }

    #[test]
    fn add_rejects_relative_home_dir() {
        assert!(
            add_profile_to_text_in(None, Path::new("/home"), CODEX, "codex_lemon", "relative")
                .is_err()
        );
    }

    #[test]
    fn add_to_config_creates_config_and_home_dir() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");
        let outcome =
            add_to_config_with_auth(&config, tmp.path(), CODEX, "codex_lemon", None, None).unwrap();

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

        let outcome = add_to_config_with_auth(
            &config,
            tmp.path(),
            CODEX,
            "lemon",
            Some(Path::new("~/.codex-lemon")),
            None,
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

        let err = add_to_config_with_auth(
            &config,
            tmp.path(),
            CODEX,
            "lemon",
            Some(Path::new("~/.codex-other")),
            None,
        )
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate"));
        assert!(!tmp.path().join(".codex-other").exists());
    }

    #[test]
    fn add_to_config_with_api_auth_writes_codex_config() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");

        let outcome = add_to_config_with_auth(
            &config,
            tmp.path(),
            CODEX,
            "codex_api",
            None,
            Some(CodexAuthMode::Api),
        )
        .unwrap();

        assert_eq!(outcome.expanded_home, tmp.path().join(".codex-codex_api"));
        assert_eq!(
            std::fs::read_to_string(outcome.expanded_home.join("config.toml")).unwrap(),
            "forced_login_method = \"api\"\ncli_auth_credentials_store = \"file\"\n"
        );
    }

    #[test]
    fn add_to_config_with_chatgpt_auth_writes_codex_config() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");

        let outcome = add_to_config_with_auth(
            &config,
            tmp.path(),
            CODEX,
            "codex_chat",
            None,
            Some(CodexAuthMode::Chatgpt),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(outcome.expanded_home.join("config.toml")).unwrap(),
            "forced_login_method = \"chatgpt\"\ncli_auth_credentials_store = \"file\"\n"
        );
    }

    #[test]
    fn codex_auth_config_preserves_existing_keys() {
        let tmp = tempdir();
        let config = tmp.path().join("config.toml");
        std::fs::write(
            &config,
            "model = \"gpt-5.5\"\nforced_login_method = \"api\"\n",
        )
        .unwrap();

        add_to_config_with_auth(
            &tmp.path().join("profiles.toml"),
            tmp.path(),
            CODEX,
            "codex_existing",
            Some(tmp.path()),
            Some(CodexAuthMode::Chatgpt),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "model = \"gpt-5.5\"\nforced_login_method = \"api\"\ncli_auth_credentials_store = \"file\"\n"
        );
    }

    #[test]
    fn codex_auth_config_inserts_missing_root_keys_before_tables() {
        let tmp = tempdir();
        let config = tmp.path().join("config.toml");
        std::fs::write(&config, "[mcp_servers.foo]\ncommand = \"foo\"\n").unwrap();

        add_to_config_with_auth(
            &tmp.path().join("profiles.toml"),
            tmp.path(),
            CODEX,
            "codex_existing",
            Some(tmp.path()),
            Some(CodexAuthMode::Api),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "forced_login_method = \"api\"\ncli_auth_credentials_store = \"file\"\n\n[mcp_servers.foo]\ncommand = \"foo\"\n"
        );
    }

    #[test]
    fn codex_auth_config_ignores_same_named_keys_inside_tables() {
        let tmp = tempdir();
        let config = tmp.path().join("config.toml");
        std::fs::write(
            &config,
            "[mcp_servers.foo]\nforced_login_method = \"chatgpt\"\n",
        )
        .unwrap();

        add_to_config_with_auth(
            &tmp.path().join("profiles.toml"),
            tmp.path(),
            CODEX,
            "codex_existing",
            Some(tmp.path()),
            Some(CodexAuthMode::Api),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "forced_login_method = \"api\"\ncli_auth_credentials_store = \"file\"\n\n[mcp_servers.foo]\nforced_login_method = \"chatgpt\"\n"
        );
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
