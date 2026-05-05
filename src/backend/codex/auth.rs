use crate::error::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/** Schema of `$CODEX_HOME/auth.json` (file backend only). */
#[derive(Debug, Deserialize)]
pub struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub auth_mode: String,
    pub tokens: Option<Tokens>,
    #[allow(dead_code)]
    /** Deserialized for schema fidelity; not consumed yet. */
    pub last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Tokens {
    pub id_token: String,
    #[allow(dead_code)]
    /** Reserved for future refresh-token flows. */
    pub access_token: String,
    #[allow(dead_code)]
    pub refresh_token: String,
    #[allow(dead_code)]
    pub account_id: Option<String>,
}

impl AuthFile {
    pub fn has_api_key(&self) -> bool {
        self.auth_mode.eq_ignore_ascii_case("apikey") && self.openai_api_key.is_some()
    }
}

/** Reads `$CODEX_HOME/auth.json`. File-backend only (keyring is v0.1.1+). */
pub fn read(codex_home: &Path) -> Result<AuthFile> {
    let path = codex_home.join("auth.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/** Pure variant for tests; takes file contents directly. */
pub fn parse(text: &str) -> Result<AuthFile> {
    serde_json::from_str(text).map_err(anyhow::Error::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn parse_with_tokens_ok() {
        let auth = parse(
            r#"{
                "auth_mode": "ChatGPT",
                "tokens": {
                    "id_token": "id",
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "account_id": "acct"
                },
                "last_refresh": "now"
            }"#,
        )
        .unwrap();

        assert_eq!(auth.auth_mode, "ChatGPT");
        let tokens = auth.tokens.unwrap();
        assert_eq!(tokens.id_token, "id");
        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token, "refresh");
        assert_eq!(tokens.account_id.as_deref(), Some("acct"));
    }

    #[test]
    fn parse_apikey_mode_without_tokens_ok() {
        let auth = parse(
            r#"{
                "OPENAI_API_KEY": "sk-test",
                "auth_mode": "ApiKey",
                "tokens": null
            }"#,
        )
        .unwrap();

        assert_eq!(auth.auth_mode, "ApiKey");
        assert_eq!(auth.openai_api_key.as_deref(), Some("sk-test"));
        assert!(auth.has_api_key());
        assert!(auth.tokens.is_none());
    }

    #[test]
    fn parse_apikey_mode_matches_codex_cli_casing() {
        let auth = parse(
            r#"{
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test"
            }"#,
        )
        .unwrap();

        assert!(auth.has_api_key());
        assert!(auth.tokens.is_none());
    }

    #[test]
    fn parse_missing_required_auth_mode_errs() {
        assert!(parse(r#"{"tokens": null}"#).is_err());
    }

    #[test]
    fn parse_invalid_json_errs() {
        assert!(parse("{").is_err());
    }

    #[test]
    fn parse_extra_fields_ignored() {
        let auth = parse(
            r#"{
                "auth_mode": "ApiKey",
                "tokens": null,
                "extra": "ignored"
            }"#,
        )
        .unwrap();

        assert_eq!(auth.auth_mode, "ApiKey");
    }

    #[test]
    fn read_propagates_missing_file_with_path() {
        let tmp = tempdir();
        let err = read(tmp.path()).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("auth.json"));
        assert!(msg.contains(tmp.path().to_str().unwrap()));
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
        let dir = std::env::temp_dir().join(format!("aiwitch-auth-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
