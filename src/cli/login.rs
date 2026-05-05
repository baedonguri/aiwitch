use crate::backend::{AnyBackend, Backend, LoginMode, ProviderCommand};
use crate::error::{Context, Result};
#[cfg(test)]
use crate::profile::ProfilesFile;
use crate::profile::{Profile, store};
use crate::shell::validate_profile_name;
use anyhow::ensure;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn run(profile_name: &str, api_key: bool) -> Result<()> {
    validate_profile_name(profile_name)?;
    let profiles = store::load()?;
    let profile = profiles.find_by_name(profile_name)?;
    let backend = AnyBackend::from_kind(profile.backend);
    let spec = command_spec_for_profile(&backend, profile, api_key)?;
    let api_key_input = if api_key {
        Some(read_api_key_from_stdin(&backend)?)
    } else {
        None
    };
    run_command(spec, api_key_input)
}

fn run_command(spec: ProviderCommand, api_key_input: Option<String>) -> Result<()> {
    let ProviderCommand {
        program,
        args,
        envs,
    } = spec;
    let mut command = Command::new(&program);
    command
        .args(&args)
        .envs(envs)
        .stdin(if api_key_input.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run {program}"))?;
    if let Some(key) = api_key_input {
        let mut stdin = child
            .stdin
            .take()
            .with_context(|| format!("failed to open {program} stdin"))?;
        writeln!(stdin, "{key}").with_context(|| format!("failed to write {program} stdin"))?;
    }
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {program}"))?;

    ensure!(status.success(), "{program} login exited with {status}");
    Ok(())
}

#[cfg(test)]
fn command_spec(
    profiles: &ProfilesFile,
    profile_name: &str,
    api_key: bool,
) -> Result<ProviderCommand> {
    validate_profile_name(profile_name)?;
    let profile = profiles.find_by_name(profile_name)?;
    let backend = AnyBackend::from_kind(profile.backend);
    command_spec_for_profile(&backend, profile, api_key)
}

fn command_spec_for_profile(
    backend: &AnyBackend,
    profile: &Profile,
    api_key: bool,
) -> Result<ProviderCommand> {
    let mode = if api_key {
        LoginMode::ApiKey
    } else {
        LoginMode::Interactive
    };
    backend.login_command(profile, mode)
}

fn read_api_key_from_stdin(backend: &AnyBackend) -> Result<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read API key from stdin")?;
    normalize_api_key(backend, &input)
}

fn normalize_api_key(backend: &AnyBackend, input: &str) -> Result<String> {
    backend.normalize_api_key(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::profile::{Profile, ProfilesFile};
    use std::path::PathBuf;

    fn profiles() -> ProfilesFile {
        ProfilesFile {
            profiles: vec![Profile {
                name: "codex_api".to_string(),
                backend: BackendKind::Codex,
                home_dir: PathBuf::from("/abs/codex-api"),
            }],
        }
    }

    fn backend() -> AnyBackend {
        AnyBackend::from_kind(BackendKind::Codex)
    }

    #[test]
    fn command_spec_uses_codex_home_for_chatgpt_login() {
        let spec = command_spec(&profiles(), "codex_api", false).unwrap();

        assert_eq!(spec.program, "codex");
        assert_eq!(
            spec.args,
            vec![
                "-c".to_string(),
                "cli_auth_credentials_store=\"file\"".to_string(),
                "login".to_string()
            ]
        );
        assert_eq!(
            spec.envs,
            vec![("CODEX_HOME".to_string(), "/abs/codex-api".to_string())]
        );
    }

    #[test]
    fn command_spec_adds_with_api_key_flag() {
        let spec = command_spec(&profiles(), "codex_api", true).unwrap();

        assert_eq!(
            spec.args,
            vec![
                "-c".to_string(),
                "cli_auth_credentials_store=\"file\"".to_string(),
                "login".to_string(),
                "--with-api-key".to_string()
            ]
        );
    }

    #[test]
    fn normalize_api_key_accepts_openai_project_key() {
        let key = normalize_api_key(&backend(), "  sk-proj-test1234567890\n").unwrap();

        assert_eq!(key, "sk-proj-test1234567890");
    }

    #[test]
    fn normalize_api_key_accepts_legacy_openai_key() {
        let key = normalize_api_key(&backend(), "sk-test1234567890").unwrap();

        assert_eq!(key, "sk-test1234567890");
    }

    #[test]
    fn normalize_api_key_rejects_empty_input() {
        let err = normalize_api_key(&backend(), " \n").unwrap_err();

        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn normalize_api_key_rejects_anthropic_key() {
        let err = normalize_api_key(&backend(), "sk-ant-api03-test").unwrap_err();

        assert!(format!("{err}").contains("Anthropic"));
    }

    #[test]
    fn normalize_api_key_rejects_shell_command_text() {
        let err = normalize_api_key(&backend(), "printf 'n'").unwrap_err();

        assert!(format!("{err}").contains("OpenAI API key"));
    }

    #[test]
    fn normalize_api_key_rejects_internal_whitespace() {
        let err = normalize_api_key(&backend(), "sk-proj-test value").unwrap_err();

        assert!(format!("{err}").contains("whitespace"));
    }
}
