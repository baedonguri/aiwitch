use crate::error::Result;
use serde::Deserialize;
use std::path::Path;

/** Schema of `$CODEX_HOME/auth.json` (file backend only). */
#[derive(Debug, Deserialize)]
pub struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub auth_mode: String,
    pub tokens: Option<Tokens>,
    pub last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Tokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: Option<String>,
}

pub fn read(_codex_home: &Path) -> Result<AuthFile> {
    todo!("read $CODEX_HOME/auth.json and parse")
}
