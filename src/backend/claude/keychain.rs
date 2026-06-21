//! macOS Keychain access for Claude Code credentials.
//!
//! Claude Code stores its OAuth blob in the login Keychain under the service
//! name `Claude Code-credentials-<first 8 hex of sha256(config_dir)>` for any
//! non-default `CLAUDE_CONFIG_DIR`, and the *unsuffixed* `Claude Code-credentials`
//! for the default `~/.claude`. This naming is undocumented; all coupling to it
//! lives in this module. Every entry point is best-effort — callers must tolerate
//! failure and fall back to current behavior.

use super::auth;
use crate::backend::ProfileMeta;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/** Outcome of reading a Keychain entry. Lets `doctor` tell apart "no session"
 *  from "user denied access" so it does not mislabel a denial as not-logged-in. */
#[derive(Debug, Clone)]
pub enum KeychainStatus {
    /** Not consulted: non-macOS, non-claude, or the default `~/.claude` dir. */
    NotApplicable,
    /** `security` reported the item does not exist (exit 44). */
    NotFound,
    /** `security` failed for another reason (access denied, spawn error, or a
     *  present-but-unparseable blob). */
    Denied,
    /** Item found and parsed. `email` is always `None` (absent from the blob). */
    Found(ProfileMeta),
}

/** Result of a verified delete attempt. */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    /** Nothing to delete, or the entry did not verify as a Claude OAuth blob. */
    Skipped,
    Failed,
}

/** Keychain service name for a config-dir string. The input MUST be the exact
 *  string aiwitch exports as `CLAUDE_CONFIG_DIR` (`~`-expanded, no further
 *  canonicalization), so the hash matches the entry Claude Code created. */
pub fn service_name(config_dir: &str) -> String {
    let digest = Sha256::digest(config_dir.as_bytes());
    let mut suffix = String::with_capacity(8);
    for b in digest.iter().take(4) {
        let _ = write!(suffix, "{b:02x}");
    }
    format!("Claude Code-credentials-{suffix}")
}

/** Service name to operate on for a resolved config dir, or `None` when it is
 *  the default `~/.claude` — whose credentials live under the unsuffixed
 *  `Claude Code-credentials` entry (the user's MAIN account) and must never be
 *  touched. This guard is the safety primitive for both read and delete. */
pub fn keychain_target(resolved: &Path, home: &Path) -> Option<String> {
    if resolved == home.join(".claude") {
        return None;
    }
    Some(service_name(resolved.to_str()?))
}

/** Read the Keychain entry for `service` and classify the result. Invokes
 *  `security`; on macOS this may prompt for Keychain access on first use. */
pub fn read_status(service: &str) -> KeychainStatus {
    match Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
    {
        Ok(o) => status_from_output(
            o.status.success(),
            o.status.code(),
            &String::from_utf8_lossy(&o.stdout),
        ),
        Err(_) => KeychainStatus::Denied,
    }
}

/** Pure classifier for a `security find-generic-password -w` result, split out
 *  so the mapping is unit-testable without a real Keychain. */
pub(crate) fn status_from_output(success: bool, code: Option<i32>, stdout: &str) -> KeychainStatus {
    if success {
        return match auth::parse(stdout.trim()) {
            Ok(creds) => {
                let oauth = creds.claude_ai_oauth.unwrap_or_default();
                KeychainStatus::Found(ProfileMeta {
                    email: oauth.email,
                    plan: oauth.subscription_type,
                    subscription_until: oauth.expires_at.and_then(super::timestamp_to_datetime),
                })
            }
            Err(_) => KeychainStatus::Denied,
        };
    }
    match code {
        Some(44) => KeychainStatus::NotFound,
        _ => KeychainStatus::Denied,
    }
}

/** Delete `service` only after reading it back and confirming it parses as a
 *  Claude OAuth blob (read-before-delete identity check). A truncated-hash
 *  collision or a changed naming scheme yields a non-matching read, so we skip
 *  rather than risk deleting an unrelated entry. */
pub fn delete_verified(service: &str) -> DeleteOutcome {
    match read_status(service) {
        KeychainStatus::Found(_) => match Command::new("security")
            .args(["delete-generic-password", "-s", service])
            .output()
        {
            Ok(o) if o.status.success() => DeleteOutcome::Deleted,
            _ => DeleteOutcome::Failed,
        },
        _ => DeleteOutcome::Skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn service_name_golden_vector() {
        // Validated against the real on-disk Keychain entry.
        assert_eq!(
            service_name("/Users/aiden/.claude-claude-lemon"),
            "Claude Code-credentials-ae0b1a7d"
        );
    }

    #[test]
    fn service_name_is_verbatim_not_canonicalized() {
        // Trailing slash and `..` are different byte strings → different hashes;
        // the module must not normalize them away.
        assert_ne!(service_name("/x/.claude-a"), service_name("/x/.claude-a/"));
        assert_ne!(
            service_name("/x/.claude-a"),
            service_name("/x/y/../.claude-a")
        );
    }

    #[test]
    fn keychain_target_refuses_default_dir() {
        let home = PathBuf::from("/Users/x");
        assert!(keychain_target(&home.join(".claude"), &home).is_none());
    }

    #[test]
    fn keychain_target_returns_suffixed_for_profile_dir() {
        let home = PathBuf::from("/Users/x");
        let dir = home.join(".claude-work");
        assert_eq!(
            keychain_target(&dir, &home),
            Some(service_name(dir.to_str().unwrap()))
        );
    }

    #[test]
    fn status_from_output_found_parses_plan_and_expiry() {
        let blob = r#"{"claudeAiOauth":{"accessToken":"secret","subscriptionType":"max","expiresAt":1735689600000}}"#;
        let KeychainStatus::Found(meta) = status_from_output(true, Some(0), blob) else {
            panic!("expected Found");
        };
        assert_eq!(meta.plan.as_deref(), Some("max"));
        assert!(meta.email.is_none());
        assert_eq!(
            meta.subscription_until
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "2025-01-01"
        );
    }

    #[test]
    fn status_from_output_success_but_garbage_is_denied() {
        assert!(matches!(
            status_from_output(true, Some(0), "{ not json"),
            KeychainStatus::Denied
        ));
    }

    #[test]
    fn status_from_output_exit_44_is_not_found() {
        assert!(matches!(
            status_from_output(false, Some(44), ""),
            KeychainStatus::NotFound
        ));
    }

    #[test]
    fn status_from_output_other_failure_is_denied() {
        assert!(matches!(
            status_from_output(false, Some(1), ""),
            KeychainStatus::Denied
        ));
        assert!(matches!(
            status_from_output(false, None, ""),
            KeychainStatus::Denied
        ));
    }
}
