use crate::backend::{AnyBackend, Backend, LoginMode, ProviderCommand};
use crate::error::{Context, Result};
#[cfg(test)]
use crate::profile::ProfilesFile;
use crate::profile::{Profile, store};
use crate::shell::validate_profile_name;
use anyhow::ensure;
use std::io::{Read, Write};
use std::ops::Range;
use std::process::{Command, Stdio};
use std::sync::atomic::{Ordering, compiler_fence};

const MAX_API_KEY_INPUT_BYTES: u64 = 8192;

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

fn run_command(spec: ProviderCommand, api_key_input: Option<ApiKeyInput>) -> Result<()> {
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
        stdin
            .write_all(key.as_bytes())
            .with_context(|| format!("failed to write {program} stdin"))?;
        stdin
            .write_all(b"\n")
            .with_context(|| format!("failed to write {program} stdin"))?;
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

fn read_api_key_from_stdin(backend: &AnyBackend) -> Result<ApiKeyInput> {
    read_api_key_from_reader(backend, std::io::stdin().lock())
}

fn read_api_key_from_reader<R: Read>(backend: &AnyBackend, reader: R) -> Result<ApiKeyInput> {
    let mut input = String::new();
    reader
        .take(MAX_API_KEY_INPUT_BYTES + 1)
        .read_to_string(&mut input)
        .context("failed to read API key from stdin")?;
    ensure!(
        input.len() <= MAX_API_KEY_INPUT_BYTES as usize,
        "API key input is too large"
    );
    ApiKeyInput::from_string(backend, input)
}

fn normalize_api_key<'a>(backend: &AnyBackend, input: &'a str) -> Result<&'a str> {
    backend.normalize_api_key(input)
}

struct ApiKeyInput {
    input: Vec<u8>,
    key_range: Range<usize>,
}

impl ApiKeyInput {
    fn from_string(backend: &AnyBackend, input: String) -> Result<Self> {
        let key = normalize_api_key(backend, &input)?;
        let base = input.as_ptr() as usize;
        let start = key.as_ptr() as usize;
        let offset = start
            .checked_sub(base)
            .with_context(|| "normalized API key was not derived from stdin input")?;
        let end = offset
            .checked_add(key.len())
            .with_context(|| "normalized API key was not derived from stdin input")?;
        ensure!(
            end <= input.len(),
            "normalized API key was not derived from stdin input"
        );
        Ok(Self {
            input: input.into_bytes(),
            key_range: offset..end,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        &self.input[self.key_range.clone()]
    }
}

impl Drop for ApiKeyInput {
    #[allow(unsafe_code)]
    /** Volatile zeroing of secret buffer; required to prevent the optimizer from eliding the write. */
    fn drop(&mut self) {
        for byte in &mut self.input {
            // SAFETY: `byte` is a unique mutable reference to a valid `u8`
            // owned by `self.input`. `write_volatile` writes a `u8` to a
            // properly aligned, non-null pointer, so this is sound.
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        compiler_fence(Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::profile::{Profile, ProfilesFile};
    use std::io::Cursor;
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
    fn read_api_key_from_reader_trims_and_keeps_key_bytes() {
        let key = read_api_key_from_reader(&backend(), Cursor::new("  sk-proj-test1234567890\n"))
            .unwrap();

        assert_eq!(key.as_bytes(), b"sk-proj-test1234567890");
    }

    #[test]
    fn read_api_key_from_reader_rejects_oversized_input() {
        let input = "x".repeat(MAX_API_KEY_INPUT_BYTES as usize + 1);
        let err = match read_api_key_from_reader(&backend(), Cursor::new(input)) {
            Ok(_) => panic!("oversized API key input should fail"),
            Err(err) => err,
        };

        assert!(format!("{err}").contains("too large"));
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
