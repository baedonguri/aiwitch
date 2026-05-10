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
    // Reject provider/flag combos *before* any filesystem mutation or $HOME
    // read, so a mistyped invocation cannot leave half-provisioned state.
    validate_provider_args(provider, auth)?;

    let config_path = store::config_path()?;
    let home_base = store::expand_home_dir(Path::new("~"))?;
    let outcome = add_to_config_with_auth(&config_path, &home_base, provider, profile, home, auth)?;

    spawn_provider_login(&outcome, auth, print_env).with_context(|| {
        format!(
            "profile {:?} was added but login did not complete; retry with `aiwitch login {}`",
            outcome.profile, outcome.profile
        )
    })?;

    if print_env {
        let snippet = render_outcome_env(&outcome, shell)?;
        print!("{snippet}");
        return Ok(());
    }

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "added profile {}", outcome.profile)?;
    writeln!(stdout, "home_dir: {}", outcome.home_dir)?;
    let envs = AnyBackend::from_kind(outcome.provider).env_exports(&outcome_profile(&outcome))?;
    let (env_key, env_value) = envs
        .first()
        .ok_or_else(|| anyhow!("backend {:?} produced no env exports", outcome.provider))?;
    writeln!(
        stdout,
        "next: {env_key}=\"{env_value}\" {}",
        provider_program(outcome.provider)
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

/** Pure validator for `add` flag combinations. Today the only constraint is
 *  that `--auth` (a Codex-only concept) cannot be combined with `claude`,
 *  whose login flow is interactive-only. */
fn validate_provider_args(provider: BackendKind, auth: Option<CodexAuthMode>) -> Result<()> {
    if matches!(provider, BackendKind::Claude) && auth.is_some() {
        return Err(anyhow!(
            "`--auth` is not supported for claude (interactive `/login` only)\nhint: run `aiwitch add claude <profile>` and use `/login` inside the claude TUI"
        ));
    }
    Ok(())
}

/** Pure helper used by both `run` (for `--print-env`) and unit tests. Builds
 *  the env snippet from the backend's own `env_exports`, so each provider
 *  exports its native variable (`CODEX_HOME` / `CLAUDE_CONFIG_DIR`) without
 *  hardcoding a name in the CLI layer. */
fn render_outcome_env(outcome: &AddOutcome, shell: EnvFormat) -> Result<String> {
    let profile = outcome_profile(outcome);
    let mut pairs = AnyBackend::from_kind(outcome.provider).env_exports(&profile)?;
    pairs.push(("AIWITCH_CURRENT".to_string(), outcome.profile.clone()));
    render_env(shell, &pairs)
}

fn outcome_profile(outcome: &AddOutcome) -> Profile {
    Profile {
        name: outcome.profile.clone(),
        backend: outcome.provider,
        home_dir: outcome.expanded_home.clone(),
    }
}

fn provider_program(provider: BackendKind) -> &'static str {
    match provider {
        BackendKind::Codex => "codex",
        BackendKind::Claude => "claude",
    }
}

fn spawn_provider_login(
    outcome: &AddOutcome,
    auth: Option<CodexAuthMode>,
    print_env: bool,
) -> Result<()> {
    let backend = AnyBackend::from_kind(outcome.provider);
    let profile = outcome_profile(outcome);
    // Codex defaults to ChatGPT when `--auth` is omitted; Claude is
    // interactive-only and never reads stdin.
    let api_key_mode = matches!(auth, Some(CodexAuthMode::Api));
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
    // Defense-in-depth: `run` already validates, but a direct call from a
    // test or a future caller must also short-circuit before any disk work.
    validate_provider_args(provider, auth)?;

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
    store::write_atomic(config_path, &updated)?;

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
        BackendKind::Claude => "claude",
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
    const CLAUDE: BackendKind = BackendKind::Claude;

    #[test]
    fn default_home_for_profile_uses_codex_prefix() {
        assert_eq!(default_home(CODEX, "codex_lemon"), "~/.codex-codex_lemon");
    }

    #[test]
    fn default_home_for_claude_uses_claude_prefix() {
        assert_eq!(default_home(CLAUDE, "personal"), "~/.claude-personal");
    }

    #[test]
    fn validate_provider_args_rejects_claude_with_auth() {
        let err = validate_provider_args(CLAUDE, Some(CodexAuthMode::Chatgpt)).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not supported"));
        assert!(msg.contains("/login"));
    }

    #[test]
    fn validate_provider_args_allows_claude_without_auth() {
        assert!(validate_provider_args(CLAUDE, None).is_ok());
    }

    #[test]
    fn validate_provider_args_allows_codex_with_or_without_auth() {
        assert!(validate_provider_args(CODEX, None).is_ok());
        assert!(validate_provider_args(CODEX, Some(CodexAuthMode::Chatgpt)).is_ok());
        assert!(validate_provider_args(CODEX, Some(CodexAuthMode::Api)).is_ok());
    }

    #[test]
    fn add_claude_with_auth_flag_errors_before_any_filesystem_mutation() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");

        let err = add_to_config_with_auth(
            &config,
            tmp.path(),
            CLAUDE,
            "with-flag",
            None,
            Some(CodexAuthMode::Chatgpt),
        )
        .unwrap_err();

        assert!(format!("{err}").contains("not supported"));
        assert!(!config.exists(), "config file must not be created");
        assert!(
            !tmp.path().join(".claude-with-flag").exists(),
            "claude home dir must not be created"
        );
        assert!(
            !config.parent().unwrap().exists(),
            "config dir must not be created"
        );
    }

    #[test]
    fn add_to_empty_creates_single_claude_profile() {
        let got = add_profile_to_text_in(
            None,
            Path::new("/home"),
            CLAUDE,
            "personal",
            "~/.claude-personal",
        )
        .unwrap();

        assert_eq!(
            got,
            "[[profiles]]\nname = \"personal\"\nbackend = \"claude\"\nhome_dir = \"~/.claude-personal\"\n"
        );
    }

    #[test]
    fn add_to_config_creates_claude_profile_without_config_files() {
        let tmp = tempdir();
        let config = tmp.path().join(".config/aiwitch/profiles.toml");

        let outcome =
            add_to_config_with_auth(&config, tmp.path(), CLAUDE, "personal", None, None).unwrap();

        assert_eq!(outcome.profile, "personal");
        assert_eq!(outcome.provider, CLAUDE);
        assert_eq!(outcome.home_dir, "~/.claude-personal");
        assert_eq!(outcome.expanded_home, tmp.path().join(".claude-personal"));
        assert!(outcome.expanded_home.is_dir());
        // Claude provisions only the home dir; no config.toml or
        // .credentials.json is written by aiwitch.
        assert!(!outcome.expanded_home.join("config.toml").exists());
        assert!(!outcome.expanded_home.join(".credentials.json").exists());
        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "[[profiles]]\nname = \"personal\"\nbackend = \"claude\"\nhome_dir = \"~/.claude-personal\"\n"
        );
    }

    #[test]
    fn render_outcome_env_for_claude_emits_claude_config_dir() {
        let outcome = AddOutcome {
            profile: "personal".to_string(),
            provider: CLAUDE,
            home_dir: "~/.claude-personal".to_string(),
            expanded_home: PathBuf::from("/Users/x/.claude-personal"),
            auth: None,
        };

        let snippet = render_outcome_env(&outcome, EnvFormat::Posix).unwrap();

        assert!(snippet.contains("CLAUDE_CONFIG_DIR="));
        assert!(snippet.contains("/Users/x/.claude-personal"));
        assert!(!snippet.contains("CODEX_HOME"));
        assert!(snippet.contains("AIWITCH_CURRENT="));
        assert!(snippet.contains("personal"));
    }

    #[test]
    fn render_outcome_env_for_codex_emits_codex_home() {
        let outcome = AddOutcome {
            profile: "work".to_string(),
            provider: CODEX,
            home_dir: "~/.codex-work".to_string(),
            expanded_home: PathBuf::from("/Users/x/.codex-work"),
            auth: Some(CodexAuthMode::Chatgpt),
        };

        let snippet = render_outcome_env(&outcome, EnvFormat::Posix).unwrap();

        assert!(snippet.contains("CODEX_HOME="));
        assert!(!snippet.contains("CLAUDE_CONFIG_DIR"));
        assert!(snippet.contains("/Users/x/.codex-work"));
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
