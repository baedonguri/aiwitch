use crate::backend::codex::{self, CodexLoginMode};
use crate::backend::{BackendKind, ProviderCommand};
use crate::error::{Context, Result};
use crate::profile::{ProfilesFile, store};
use crate::shell::validate_profile_name;
use anyhow::ensure;
use std::process::{Command, Stdio};

pub fn run(profile_name: &str, api_key: bool) -> Result<()> {
    validate_profile_name(profile_name)?;
    let profiles = store::load()?;
    let spec = command_spec(&profiles, profile_name, api_key)?;
    let ProviderCommand {
        program,
        args,
        envs,
    } = spec;
    let status = Command::new(&program)
        .args(&args)
        .envs(envs)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run {program}"))?;

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
}
