use crate::backend::{AnyBackend, Backend, ProviderCommand};
use crate::cli::current::AIWITCH_CURRENT_KEY;
use crate::error::{Context, Result};
use crate::profile::{ProfilesFile, store};
use crate::shell::{validate_env_key, validate_env_value, validate_profile_name};
use anyhow::ensure;
use std::process::{Command, ExitStatus, Stdio};

pub fn run(profile_name: &str, cmd: &[String]) -> Result<()> {
    let code = run_inner(profile_name, cmd)?;
    // `process::exit` does not unwind: it skips destructors and `BufWriter` flushes.
    // We currently only inherit stdio, but if a future caller adds parent-side
    // output before this point, flush it explicitly before returning here.
    std::process::exit(code);
}

fn run_inner(profile_name: &str, cmd: &[String]) -> Result<i32> {
    validate_profile_name(profile_name)?;
    let profiles = store::load()?;
    let spec = build_command(&profiles, profile_name, cmd)?;
    spawn_and_wait(spec)
}

/** Pure command builder. Mirrors `cli::env::render` env semantics so that
 *  `aiwitch use` and `aiwitch run` produce identical environments. */
pub fn build_command(
    profiles: &ProfilesFile,
    name: &str,
    cmd: &[String],
) -> Result<ProviderCommand> {
    validate_profile_name(name)?;
    ensure!(
        !cmd.is_empty(),
        "expected a command after the profile name (e.g. `aiwitch run {name} -- codex`)"
    );
    let profile = profiles.find_by_name(name)?;
    let backend = AnyBackend::from_kind(profile.backend);
    let mut envs = backend.env_exports(profile)?;
    envs.push((AIWITCH_CURRENT_KEY.to_string(), profile.name.clone()));
    for (k, v) in &envs {
        validate_env_key(k)?;
        validate_env_value(v)?;
    }
    let (program, args) = cmd
        .split_first()
        .expect("non-empty checked above by ensure!");
    Ok(ProviderCommand {
        program: program.clone(),
        args: args.to_vec(),
        envs,
    })
}

fn spawn_and_wait(spec: ProviderCommand) -> Result<i32> {
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
        .with_context(|| {
            format!(
                "failed to run {program}\nhint: install the {program} CLI and ensure it is on PATH"
            )
        })?;
    Ok(exit_code(status))
}

#[cfg(unix)]
fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    if let Some(c) = status.code() {
        return c;
    }
    if let Some(sig) = status.signal() {
        return 128 + sig;
    }
    1
}

#[cfg(not(unix))]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::profile::Profile;
    use std::path::PathBuf;

    fn pf() -> ProfilesFile {
        ProfilesFile {
            profiles: vec![
                Profile {
                    name: "personal".to_string(),
                    backend: BackendKind::Codex,
                    home_dir: PathBuf::from("/Users/x/.codex-personal"),
                },
                Profile {
                    name: "claude-main".to_string(),
                    backend: BackendKind::Claude,
                    home_dir: PathBuf::from("/Users/x/.claude-main"),
                },
            ],
        }
    }

    #[test]
    fn build_command_includes_provider_env_and_aiwitch_current() {
        let cmd = vec!["codex".to_string()];
        let spec = build_command(&pf(), "personal", &cmd).unwrap();
        assert_eq!(spec.program, "codex");
        assert!(spec.args.is_empty());
        let envs: std::collections::HashMap<_, _> = spec.envs.into_iter().collect();
        assert_eq!(
            envs.get("CODEX_HOME").map(String::as_str),
            Some("/Users/x/.codex-personal")
        );
        assert_eq!(
            envs.get("AIWITCH_CURRENT").map(String::as_str),
            Some("personal")
        );
    }

    #[test]
    fn build_command_passes_remaining_args_through() {
        let cmd = vec![
            "codex".to_string(),
            "exec".to_string(),
            "hello world".to_string(),
        ];
        let spec = build_command(&pf(), "personal", &cmd).unwrap();
        assert_eq!(spec.program, "codex");
        assert_eq!(
            spec.args,
            vec!["exec".to_string(), "hello world".to_string()]
        );
    }

    #[test]
    fn build_command_for_claude_profile_uses_claude_config_dir() {
        let cmd = vec!["claude".to_string()];
        let spec = build_command(&pf(), "claude-main", &cmd).unwrap();
        let envs: std::collections::HashMap<_, _> = spec.envs.into_iter().collect();
        assert_eq!(
            envs.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/Users/x/.claude-main")
        );
        assert_eq!(
            envs.get("AIWITCH_CURRENT").map(String::as_str),
            Some("claude-main")
        );
    }

    #[test]
    fn build_command_rejects_empty_cmd() {
        let cmd: Vec<String> = vec![];
        let err = build_command(&pf(), "personal", &cmd).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("expected a command"));
    }

    #[test]
    fn build_command_rejects_unknown_profile() {
        let cmd = vec!["codex".to_string()];
        let err = build_command(&pf(), "ghost", &cmd).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ghost"));
        assert!(msg.contains("personal"));
    }

    #[test]
    fn build_command_rejects_invalid_profile_name() {
        let cmd = vec!["codex".to_string()];
        assert!(build_command(&pf(), "with.dot", &cmd).is_err());
        assert!(build_command(&pf(), "-leading", &cmd).is_err());
        assert!(build_command(&pf(), "", &cmd).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_maps_signal_to_128_plus_signal() {
        use std::os::unix::process::ExitStatusExt;
        let status = ExitStatus::from_raw(9); // SIGKILL: low byte = signal, no core dump bit
        assert_eq!(exit_code(status), 128 + 9);
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_returns_status_when_present() {
        use std::os::unix::process::ExitStatusExt;
        // wait(2) status: exit code N is encoded as (N << 8).
        let status = ExitStatus::from_raw(7 << 8);
        assert_eq!(exit_code(status), 7);
    }
}
