use crate::error::Result;
use anyhow::{anyhow, ensure};
use base64::Engine;
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
    #[serde(default, deserialize_with = "lenient_datetime")]
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

fn lenient_datetime<'de, D>(d: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::String(s)) => DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        Some(serde_json::Value::Number(n)) => n.as_i64().and_then(parse_unix_ts),
        _ => None,
    })
}

fn parse_unix_ts(n: i64) -> Option<DateTime<Utc>> {
    if n <= -1_000_000_000_000 || n >= 1_000_000_000_000 {
        DateTime::from_timestamp_millis(n)
    } else {
        DateTime::from_timestamp(n, 0)
    }
}

/** Decode the payload segment of a JWT. Signature is not verified. */
pub fn decode_payload(id_token: &str) -> Result<IdTokenPayload> {
    let mut parts = id_token.split('.');
    let _header = parts.next().ok_or_else(|| anyhow!("jwt missing header"))?;
    let payload = parts.next().ok_or_else(|| anyhow!("jwt missing payload"))?;
    let _sig = parts
        .next()
        .ok_or_else(|| anyhow!("jwt missing signature segment"))?;
    ensure!(parts.next().is_none(), "jwt has more than 3 segments");

    let normalized = payload.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(normalized)
        .map_err(|e| anyhow!("jwt payload base64 decode failed: {e}"))?;

    serde_json::from_slice(&bytes).map_err(|e| anyhow!("jwt payload json parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload_json: &str) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"");
        format!("{header}.{payload}.{sig}")
    }

    #[test]
    fn decode_payload_happy_path() {
        let payload = decode_payload(&jwt(r#"{"email":"a@b.com","name":"A"}"#)).unwrap();
        assert_eq!(payload.email.as_deref(), Some("a@b.com"));
        assert_eq!(payload.name.as_deref(), Some("A"));
    }

    #[test]
    fn decode_payload_with_openai_auth_full() {
        let payload = decode_payload(&jwt(r#"{
                "email": "a@b.com",
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": "plus",
                    "chatgpt_subscription_active_until": "2026-05-05T12:34:56Z"
                }
            }"#))
        .unwrap();

        let auth = payload.openai_auth.unwrap();
        assert_eq!(payload.email.as_deref(), Some("a@b.com"));
        assert_eq!(auth.chatgpt_plan_type.as_deref(), Some("plus"));
        assert_eq!(
            auth.chatgpt_subscription_active_until.unwrap().to_rfc3339(),
            "2026-05-05T12:34:56+00:00"
        );
    }

    #[test]
    fn decode_payload_lenient_timestamp_unix_seconds() {
        let payload = decode_payload(&jwt(r#"{
                "https://api.openai.com/auth": {
                    "chatgpt_subscription_active_until": 1777984496
                }
            }"#))
        .unwrap();

        let dt = payload
            .openai_auth
            .unwrap()
            .chatgpt_subscription_active_until
            .unwrap();
        assert_eq!(dt.timestamp(), 1_777_984_496);
    }

    #[test]
    fn decode_payload_lenient_timestamp_unix_millis() {
        let payload = decode_payload(&jwt(r#"{
                "https://api.openai.com/auth": {
                    "chatgpt_subscription_active_until": 1777984496123
                }
            }"#))
        .unwrap();

        let dt = payload
            .openai_auth
            .unwrap()
            .chatgpt_subscription_active_until
            .unwrap();
        assert_eq!(dt.timestamp_millis(), 1_777_984_496_123);
    }

    #[test]
    fn decode_payload_lenient_timestamp_invalid_string_keeps_other_fields() {
        let payload = decode_payload(&jwt(r#"{
                "email": "a@b.com",
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": "team",
                    "chatgpt_subscription_active_until": "not-a-date"
                }
            }"#))
        .unwrap();

        let auth = payload.openai_auth.unwrap();
        assert_eq!(payload.email.as_deref(), Some("a@b.com"));
        assert_eq!(auth.chatgpt_plan_type.as_deref(), Some("team"));
        assert!(auth.chatgpt_subscription_active_until.is_none());
    }

    #[test]
    fn decode_payload_lenient_timestamp_i64_min_keeps_other_fields() {
        let payload = decode_payload(&jwt(r#"{
                "email": "a@b.com",
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": "team",
                    "chatgpt_subscription_active_until": -9223372036854775808
                }
            }"#))
        .unwrap();

        let auth = payload.openai_auth.unwrap();
        assert_eq!(payload.email.as_deref(), Some("a@b.com"));
        assert_eq!(auth.chatgpt_plan_type.as_deref(), Some("team"));
        assert!(auth.chatgpt_subscription_active_until.is_none());
    }

    #[test]
    fn decode_payload_padded_input_ok() {
        let token = jwt(r#"{"email":"a@b.com"}"#);
        let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
        parts[1].push_str("==");
        let payload = decode_payload(&parts.join(".")).unwrap();
        assert_eq!(payload.email.as_deref(), Some("a@b.com"));
    }

    #[test]
    fn decode_payload_two_segments_errs() {
        assert!(decode_payload("a.b").is_err());
    }

    #[test]
    fn decode_payload_invalid_base64_errs() {
        let err = decode_payload("a.%%%.").unwrap_err();
        assert!(format!("{err}").contains("base64"));
    }

    #[test]
    fn decode_payload_invalid_json_errs() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json");
        let err = decode_payload(&format!("{header}.{payload}.")).unwrap_err();
        assert!(format!("{err}").contains("json"));
    }
}
