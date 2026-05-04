use crate::backend::codex::{self, CodexLoginMode};
use crate::backend::{BackendKind, ProviderCommand};
use crate::error::{Context, Result};
use crate::profile::{ProfilesFile, store};
use crate::shell::validate_profile_name;
use anyhow::ensure;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn run(profile_name: &str, api_key: bool) -> Result<()> {
    validate_profile_name(profile_name)?;
    let profiles = store::load()?;
    let spec = command_spec(&profiles, profile_name, api_key)?;
    let api_key_input = if api_key {
        Some(read_openai_api_key_from_stdin()?)
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

fn command_spec(
    profiles: &ProfilesFile,
    profile_name: &str,
    api_key: bool,
) -> Result<ProviderCommand> {
    validate_profile_name(profile_name)?;
    let profile = profiles.find_by_name(profile_name)?;
    match profile.backend {
        BackendKind::Codex => {
            let mode = if api_key {
                CodexLoginMode::ApiKey
            } else {
                CodexLoginMode::Chatgpt
            };
            codex::login_command(profile, mode)
        }
    }
}

fn read_openai_api_key_from_stdin() -> Result<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read OpenAI API key from stdin")?;
    normalize_openai_api_key(&input)
}

fn normalize_openai_api_key(input: &str) -> Result<String> {
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
    Ok(key.to_string())
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
        let key = normalize_openai_api_key("  sk-proj-test1234567890\n").unwrap();

        assert_eq!(key, "sk-proj-test1234567890");
    }

    #[test]
    fn normalize_api_key_accepts_legacy_openai_key() {
        let key = normalize_openai_api_key("sk-test1234567890").unwrap();

        assert_eq!(key, "sk-test1234567890");
    }

    #[test]
    fn normalize_api_key_rejects_empty_input() {
        let err = normalize_openai_api_key(" \n").unwrap_err();

        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn normalize_api_key_rejects_anthropic_key() {
        let err = normalize_openai_api_key("sk-ant-api03-test").unwrap_err();

        assert!(format!("{err}").contains("Anthropic"));
    }

    #[test]
    fn normalize_api_key_rejects_shell_command_text() {
        let err = normalize_openai_api_key("printf 'n'").unwrap_err();

        assert!(format!("{err}").contains("OpenAI API key"));
    }

    #[test]
    fn normalize_api_key_rejects_internal_whitespace() {
        let err = normalize_openai_api_key("sk-proj-test value").unwrap_err();

        assert!(format!("{err}").contains("whitespace"));
    }
}
