use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;

/** Subset of `id_token` payload claims used for display. Signature is not verified. */
#[derive(Debug, Deserialize)]
pub struct IdTokenPayload {
    pub email: Option<String>,
    pub name: Option<String>,
    pub exp: Option<i64>,
    #[serde(rename = "https://api.openai.com/auth")]
    pub openai_auth: Option<OpenAiAuth>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiAuth {
    pub chatgpt_plan_type: Option<String>,
    pub chatgpt_subscription_active_until: Option<DateTime<Utc>>,
    pub chatgpt_account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
    #[serde(default)]
    pub organizations: Vec<Organization>,
}

#[derive(Debug, Deserialize)]
pub struct Organization {
    pub id: String,
    pub role: Option<String>,
    pub title: Option<String>,
    pub is_default: Option<bool>,
}

/** Decode the payload segment of a JWT. Signature is not verified. */
pub fn decode_payload(_id_token: &str) -> Result<IdTokenPayload> {
    todo!("split on '.', base64url-decode segment[1] with padding, parse JSON")
}
