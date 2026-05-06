use crate::error::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/** Schema of `<CLAUDE_CONFIG_DIR>/.credentials.json` (Linux/Windows file backend).
 *
 *  On macOS, Claude Code stores credentials in the system Keychain, so this
 *  file usually does not exist; callers should treat absence as "no metadata
 *  available" rather than an error.
 *
 *  Real-world Claude installations use a nested camelCase layout under
 *  `claudeAiOauth`. We accept that layout, ignore unknown fields, and tolerate
 *  a top-level api-key field for forward compat with future builds. */
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsFile {
    pub claude_ai_oauth: Option<OAuth>,
    /** Tolerate `apiKey` (via `rename_all`) and `ANTHROPIC_API_KEY` (legacy/env-style). */
    #[serde(alias = "ANTHROPIC_API_KEY")]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OAuth {
    #[allow(dead_code)]
    /** Reserved for future refresh flows; not consumed yet. */
    pub access_token: Option<String>,
    #[allow(dead_code)]
    pub refresh_token: Option<String>,
    /** Subscription expiry. Usually Unix epoch milliseconds; some builds emit seconds. */
    pub expires_at: Option<i64>,
    /** Often absent — Claude Code does not always persist email locally. */
    pub email: Option<String>,
    /** Plan tier surfaced by Claude Code (e.g. `"pro"`, `"max"`). */
    pub subscription_type: Option<String>,
    #[allow(dead_code)]
    pub rate_limit_tier: Option<String>,
}

/** Reads `<claude_home>/.credentials.json`. */
pub fn read(claude_home: &Path) -> Result<CredentialsFile> {
    let path = claude_home.join(".credentials.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/** Pure variant for tests; takes file contents directly. */
pub fn parse(text: &str) -> Result<CredentialsFile> {
    serde_json::from_str(text).map_err(anyhow::Error::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn parse_camelcase_nested_oauth_ok() {
        let creds = parse(
            r#"{
                "claudeAiOauth": {
                    "accessToken": "at",
                    "refreshToken": "rt",
                    "expiresAt": 1735689600000,
                    "email": "user@example.com",
                    "subscriptionType": "max",
                    "rateLimitTier": "tier-1"
                }
            }"#,
        )
        .unwrap();

        let oauth = creds.claude_ai_oauth.unwrap();
        assert_eq!(oauth.access_token.as_deref(), Some("at"));
        assert_eq!(oauth.refresh_token.as_deref(), Some("rt"));
        assert_eq!(oauth.expires_at, Some(1_735_689_600_000));
        assert_eq!(oauth.email.as_deref(), Some("user@example.com"));
        assert_eq!(oauth.subscription_type.as_deref(), Some("max"));
        assert_eq!(oauth.rate_limit_tier.as_deref(), Some("tier-1"));
        assert!(creds.api_key.is_none());
    }

    #[test]
    fn parse_with_unknown_fields_ignored() {
        let creds = parse(
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "pro",
                    "unexpected": 42
                },
                "extraTopLevel": null
            }"#,
        )
        .unwrap();

        assert_eq!(
            creds.claude_ai_oauth.unwrap().subscription_type.as_deref(),
            Some("pro")
        );
    }

    #[test]
    fn parse_missing_oauth_block_ok() {
        let creds = parse("{}").unwrap();

        assert!(creds.claude_ai_oauth.is_none());
        assert!(creds.api_key.is_none());
    }

    #[test]
    fn parse_top_level_api_key_camelcase() {
        let creds = parse(r#"{"apiKey": "sk-ant-test"}"#).unwrap();

        assert_eq!(creds.api_key.as_deref(), Some("sk-ant-test"));
    }

    #[test]
    fn parse_top_level_api_key_anthropic_alias() {
        let creds = parse(r#"{"ANTHROPIC_API_KEY": "sk-ant-legacy"}"#).unwrap();

        assert_eq!(creds.api_key.as_deref(), Some("sk-ant-legacy"));
    }

    #[test]
    fn parse_invalid_json_errs() {
        assert!(parse("{").is_err());
    }

    #[test]
    fn read_propagates_missing_file_with_path() {
        let tmp = tempdir();
        let err = read(tmp.path()).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains(".credentials.json"));
        assert!(msg.contains(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn read_returns_parsed_file_when_present() {
        let tmp = tempdir();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"subscriptionType":"max"}}"#,
        )
        .unwrap();

        let creds = read(tmp.path()).unwrap();

        assert_eq!(
            creds.claude_ai_oauth.unwrap().subscription_type.as_deref(),
            Some("max")
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
        let dir = std::env::temp_dir().join(format!("aiwitch-claude-auth-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
