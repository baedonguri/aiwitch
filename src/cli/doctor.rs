use crate::backend::{Backend, BackendKind, ProfileMeta, claude, codex};
use crate::cli::current::{AIWITCH_CURRENT_KEY, EnvLookup, SystemEnv};
use crate::error::Result;
use crate::profile::{Profile, store};
use chrono::{DateTime, Utc};
use std::fmt::Write as _;
use std::io::Write;

const EXPIRES_SOON_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub status: Status,
    pub subject: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.count(Status::Err)
    }
    pub fn warnings(&self) -> usize {
        self.count(Status::Warn)
    }
    fn count(&self, s: Status) -> usize {
        self.checks.iter().filter(|c| c.status == s).count()
    }
}

/** Inputs per profile. Filesystem reads happen in `collect_facts` so that
 *  `build_report` is a pure function and trivially testable. */
#[derive(Debug, Clone)]
pub struct ProfileFacts {
    pub profile: Profile,
    pub home_exists: bool,
    /** True if the on-disk credentials file exists (`auth.json` for Codex,
     *  `.credentials.json` for Claude). On macOS Claude stores creds in the
     *  Keychain, so this is typically `false` even when logged in. */
    pub credentials_present: bool,
    /** `Some` when the credentials file parsed; `None` means parse failed.
     *  An "all-None" `ProfileMeta` means the file parsed but contained no
     *  usable session (e.g. post-logout `auth.json` with `tokens: null`). */
    pub meta: Option<ProfileMeta>,
    /** macOS Claude credentials read from the Keychain. `NotApplicable` on
     *  non-macOS, non-claude, or the default `~/.claude` dir — in which case the
     *  file-based `meta` path above applies instead. Injected by `collect_facts`
     *  so `build_report` stays pure. */
    pub keychain: claude::keychain::KeychainStatus,
}

/** Inputs not tied to a single profile. `*_cli_on_path = None` means no
 *  profile uses that provider, so the check is skipped. */
#[derive(Debug, Clone, Default)]
pub struct GlobalFacts {
    pub codex_cli_on_path: Option<bool>,
    pub claude_cli_on_path: Option<bool>,
    pub aiwitch_current: Option<String>,
    pub is_macos: bool,
}

pub fn build_report(
    profiles: &[ProfileFacts],
    globals: &GlobalFacts,
    now: DateTime<Utc>,
) -> Report {
    let mut checks = Vec::with_capacity(profiles.len() + 4);
    if profiles.is_empty() {
        checks.push(Check {
            status: Status::Warn,
            subject: "profiles".to_string(),
            detail: "no profiles configured (run `aiwitch add <provider> <name>`)".to_string(),
        });
    }
    for f in profiles {
        checks.push(profile_check(f, globals, now));
    }
    if let Some(present) = globals.codex_cli_on_path {
        checks.push(cli_check("codex", present));
    }
    if let Some(present) = globals.claude_cli_on_path {
        checks.push(cli_check("claude", present));
    }
    checks.push(aiwitch_current_check(
        globals.aiwitch_current.as_deref(),
        profiles,
    ));
    Report { checks }
}

fn profile_check(f: &ProfileFacts, globals: &GlobalFacts, now: DateTime<Utc>) -> Check {
    let subject = profile_subject(&f.profile);
    if !f.home_exists {
        return Check {
            status: Status::Err,
            subject,
            detail: format!("home dir missing: {}", f.profile.home_dir.display()),
        };
    }
    match f.profile.backend {
        BackendKind::Codex => codex_profile_check(subject, f, now),
        BackendKind::Claude => claude_profile_check(subject, f, globals, now),
    }
}

fn codex_profile_check(subject: String, f: &ProfileFacts, now: DateTime<Utc>) -> Check {
    if !f.credentials_present {
        return Check {
            status: Status::Warn,
            subject,
            detail: format!("not logged in (run `aiwitch login {}`)", f.profile.name),
        };
    }
    let Some(meta) = &f.meta else {
        return Check {
            status: Status::Err,
            subject,
            detail: format!(
                "auth.json present but failed to parse (run `aiwitch login {}`)",
                f.profile.name
            ),
        };
    };
    auth_meta_check(subject, meta, now, &f.profile.name)
}

fn claude_profile_check(
    subject: String,
    f: &ProfileFacts,
    globals: &GlobalFacts,
    now: DateTime<Utc>,
) -> Check {
    use claude::keychain::KeychainStatus;
    // On macOS the OAuth blob lives in the Keychain; `collect_facts` reads it
    // into `f.keychain`. `NotApplicable` means the file-based path below applies.
    match &f.keychain {
        KeychainStatus::Found(meta) => return auth_meta_check(subject, meta, now, &f.profile.name),
        KeychainStatus::NotFound => {
            return Check {
                status: Status::Warn,
                subject,
                detail: format!(
                    "not logged in (run `aiwitch login {}` and use `/login`)",
                    f.profile.name
                ),
            };
        }
        KeychainStatus::Denied => {
            return Check {
                status: Status::Warn,
                subject,
                detail: "keychain access unavailable (allow access, or verify with `claude`)"
                    .to_string(),
            };
        }
        KeychainStatus::NotApplicable => {}
    }

    if !f.credentials_present {
        let detail = if globals.is_macos {
            "metadata unavailable (macOS keychain — verify with `claude` if needed)".to_string()
        } else {
            format!(
                "not logged in (run `aiwitch login {}` and use `/login`)",
                f.profile.name
            )
        };
        return Check {
            status: Status::Warn,
            subject,
            detail,
        };
    }
    let Some(meta) = &f.meta else {
        return Check {
            status: Status::Err,
            subject,
            detail: format!(
                ".credentials.json present but failed to parse (run `aiwitch login {}`)",
                f.profile.name
            ),
        };
    };
    auth_meta_check(subject, meta, now, &f.profile.name)
}

fn auth_meta_check(
    subject: String,
    meta: &ProfileMeta,
    now: DateTime<Utc>,
    profile_name: &str,
) -> Check {
    if meta.email.is_none() && meta.plan.is_none() && meta.subscription_until.is_none() {
        return Check {
            status: Status::Warn,
            subject,
            detail: format!(
                "credentials present but no usable session (run `aiwitch login {profile_name}`)"
            ),
        };
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(email) = &meta.email {
        parts.push(format!("logged in as {email}"));
    } else {
        parts.push("logged in".to_string());
    }
    if let Some(plan) = &meta.plan {
        parts.push(format!("plan {plan}"));
    }

    if let Some(until) = meta.subscription_until {
        let date = until.format("%Y-%m-%d");
        if until <= now {
            return Check {
                status: Status::Err,
                subject,
                detail: format!("{}; token expired on {date}", parts.join(", ")),
            };
        }
        let days = (until - now).num_days();
        if days < EXPIRES_SOON_DAYS {
            let unit = if days == 1 { "day" } else { "days" };
            return Check {
                status: Status::Warn,
                subject,
                detail: format!("{}; expires in {days} {unit} ({date})", parts.join(", ")),
            };
        }
        parts.push(format!("expires {date}"));
    }

    Check {
        status: Status::Ok,
        subject,
        detail: parts.join(", "),
    }
}

fn cli_check(name: &str, present: bool) -> Check {
    if present {
        Check {
            status: Status::Ok,
            subject: format!("{name} CLI"),
            detail: "on PATH".to_string(),
        }
    } else {
        Check {
            status: Status::Err,
            subject: format!("{name} CLI"),
            detail: format!("not found on PATH (install {name} CLI and try again)"),
        }
    }
}

fn aiwitch_current_check(current: Option<&str>, profiles: &[ProfileFacts]) -> Check {
    let subject = AIWITCH_CURRENT_KEY.to_string();
    match current {
        None | Some("") => Check {
            status: Status::Ok,
            subject,
            detail: "unset (no shell-active profile)".to_string(),
        },
        Some(name) if profiles.iter().any(|p| p.profile.name == name) => Check {
            status: Status::Ok,
            subject,
            detail: format!("set to {name}"),
        },
        Some(name) => Check {
            status: Status::Err,
            subject,
            detail: format!("set to unknown profile {name:?} (run `aiwitch list`)"),
        },
    }
}

fn profile_subject(profile: &Profile) -> String {
    let provider = match profile.backend {
        BackendKind::Codex => "codex",
        BackendKind::Claude => "claude",
    };
    format!("{provider}/{}", profile.name)
}

pub fn render(report: &Report) -> String {
    let max_subject = report
        .checks
        .iter()
        .map(|c| c.subject.chars().count())
        .max()
        .unwrap_or(0);

    let mut out = String::new();
    for c in &report.checks {
        let tag = match c.status {
            Status::Ok => "[ok]  ",
            Status::Warn => "[warn]",
            Status::Err => "[err] ",
        };
        let pad = " ".repeat(max_subject - c.subject.chars().count());
        let _ = writeln!(out, "{tag}  {}{pad}  {}", c.subject, c.detail);
    }
    out.push('\n');
    let errors = report.errors();
    let warnings = report.warnings();
    if errors == 0 && warnings == 0 {
        out.push_str("all checks passed\n");
    } else {
        let e_unit = if errors == 1 { "error" } else { "errors" };
        let w_unit = if warnings == 1 { "warning" } else { "warnings" };
        let _ = writeln!(out, "{errors} {e_unit}, {warnings} {w_unit}");
    }
    out
}

pub fn run() -> Result<()> {
    let profiles_file = store::load()?;
    let now = Utc::now();
    let env = SystemEnv;

    let profile_facts: Vec<ProfileFacts> =
        profiles_file.profiles.iter().map(collect_facts).collect();
    let globals = collect_globals(&profiles_file.profiles, &env);

    let report = build_report(&profile_facts, &globals, now);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(render(&report).as_bytes())?;

    if report.errors() > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn collect_facts(profile: &Profile) -> ProfileFacts {
    let home_exists = profile.home_dir.is_dir();
    let credentials_present = home_exists
        && match profile.backend {
            BackendKind::Codex => profile.home_dir.join("auth.json").is_file(),
            BackendKind::Claude => profile.home_dir.join(".credentials.json").is_file(),
        };
    let meta = if credentials_present {
        read_strict_meta(profile)
    } else {
        None
    };
    let keychain = if home_exists {
        collect_keychain_status(profile)
    } else {
        claude::keychain::KeychainStatus::NotApplicable
    };
    ProfileFacts {
        profile: profile.clone(),
        home_exists,
        credentials_present,
        meta,
        keychain,
    }
}

/** Reads the macOS Keychain entry for a Claude profile (the impure edge).
 *  `NotApplicable` for non-claude or the default `~/.claude` dir. */
#[cfg(target_os = "macos")]
fn collect_keychain_status(profile: &Profile) -> claude::keychain::KeychainStatus {
    use claude::keychain::{self, KeychainStatus};
    if profile.backend != BackendKind::Claude {
        return KeychainStatus::NotApplicable;
    }
    let Ok(home) = std::env::var("HOME") else {
        return KeychainStatus::NotApplicable;
    };
    match keychain::keychain_target(&profile.home_dir, std::path::Path::new(&home)) {
        Some(service) => keychain::read_status(&service),
        None => KeychainStatus::NotApplicable,
    }
}

#[cfg(not(target_os = "macos"))]
fn collect_keychain_status(_profile: &Profile) -> claude::keychain::KeychainStatus {
    claude::keychain::KeychainStatus::NotApplicable
}

fn collect_globals(profiles: &[Profile], env: &impl EnvLookup) -> GlobalFacts {
    let codex_used = profiles.iter().any(|p| p.backend == BackendKind::Codex);
    let claude_used = profiles.iter().any(|p| p.backend == BackendKind::Claude);
    GlobalFacts {
        codex_cli_on_path: codex_used.then(|| cli_on_path("codex")),
        claude_cli_on_path: claude_used.then(|| cli_on_path("claude")),
        aiwitch_current: env.get(AIWITCH_CURRENT_KEY),
        is_macos: cfg!(target_os = "macos"),
    }
}

/** Strict meta reader for doctor: surfaces parse failures as `None` so the
 *  caller can emit `[err]` instead of the best-effort `[warn]` that
 *  `ClaudeBackend::read_meta` returns by design (it cannot distinguish
 *  "missing on macOS keychain" from "corrupt file"). */
fn read_strict_meta(profile: &Profile) -> Option<ProfileMeta> {
    match profile.backend {
        BackendKind::Codex => codex::CodexBackend.read_meta(profile).ok(),
        BackendKind::Claude => {
            let creds = claude::auth::read(&profile.home_dir).ok()?;
            let oauth = creds.claude_ai_oauth.unwrap_or_default();
            Some(ProfileMeta {
                email: oauth.email,
                plan: oauth.subscription_type,
                subscription_until: oauth.expires_at.and_then(claude::timestamp_to_datetime),
            })
        }
    }
}

fn cli_on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable_file(&dir.join(name)))
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap()
    }

    fn profile(name: &str, kind: BackendKind, home: &str) -> Profile {
        Profile {
            name: name.to_string(),
            backend: kind,
            home_dir: PathBuf::from(home),
        }
    }

    fn ok_facts(name: &str, kind: BackendKind) -> ProfileFacts {
        ProfileFacts {
            profile: profile(name, kind, &format!("/abs/{name}")),
            home_exists: true,
            credentials_present: true,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: Some(ProfileMeta {
                email: Some("u@e.com".into()),
                plan: Some("plus".into()),
                subscription_until: Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()),
            }),
        }
    }

    fn empty_globals() -> GlobalFacts {
        GlobalFacts::default()
    }

    #[test]
    fn report_counts_each_status() {
        let report = Report {
            checks: vec![
                Check {
                    status: Status::Ok,
                    subject: "a".into(),
                    detail: String::new(),
                },
                Check {
                    status: Status::Warn,
                    subject: "b".into(),
                    detail: String::new(),
                },
                Check {
                    status: Status::Err,
                    subject: "c".into(),
                    detail: String::new(),
                },
                Check {
                    status: Status::Err,
                    subject: "d".into(),
                    detail: String::new(),
                },
            ],
        };
        assert_eq!(report.errors(), 2);
        assert_eq!(report.warnings(), 1);
    }

    #[test]
    fn missing_home_dir_is_error() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Codex, "/abs/p"),
            home_exists: false,
            credentials_present: false,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Err);
        assert!(r.checks[0].detail.contains("home dir missing"));
        assert!(r.checks[0].detail.contains("/abs/p"));
    }

    #[test]
    fn codex_no_credentials_is_warning_not_logged_in() {
        let f = ProfileFacts {
            profile: profile("work", BackendKind::Codex, "/abs/work"),
            home_exists: true,
            credentials_present: false,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("not logged in"));
        assert!(r.checks[0].detail.contains("aiwitch login work"));
    }

    #[test]
    fn codex_credentials_unparseable_is_error() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Codex, "/abs/p"),
            home_exists: true,
            credentials_present: true,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Err);
        assert!(r.checks[0].detail.contains("failed to parse"));
    }

    #[test]
    fn empty_meta_is_warning_no_session() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Codex, "/abs/p"),
            home_exists: true,
            credentials_present: true,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: Some(ProfileMeta::default()),
        };
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("no usable session"));
    }

    fn claude_keychain_facts(status: claude::keychain::KeychainStatus) -> ProfileFacts {
        ProfileFacts {
            profile: profile("work", BackendKind::Claude, "/abs/.claude-work"),
            home_exists: true,
            credentials_present: false, // macOS: no .credentials.json file
            keychain: status,
            meta: None,
        }
    }

    #[test]
    fn keychain_found_shows_plan_and_expiry() {
        use claude::keychain::KeychainStatus;
        let meta = ProfileMeta {
            email: None, // never present in the keychain blob
            plan: Some("max".into()),
            subscription_until: Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()),
        };
        let r = build_report(
            &[claude_keychain_facts(KeychainStatus::Found(meta))],
            &empty_globals(),
            now(),
        );
        assert_eq!(r.checks[0].status, Status::Ok);
        assert!(r.checks[0].detail.contains("max"));
        assert!(r.checks[0].detail.contains("2027-01-01"));
    }

    #[test]
    fn keychain_not_found_is_not_logged_in_warning() {
        use claude::keychain::KeychainStatus;
        let r = build_report(
            &[claude_keychain_facts(KeychainStatus::NotFound)],
            &empty_globals(),
            now(),
        );
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("not logged in"));
    }

    #[test]
    fn keychain_denied_is_distinct_from_not_logged_in() {
        use claude::keychain::KeychainStatus;
        let r = build_report(
            &[claude_keychain_facts(KeychainStatus::Denied)],
            &empty_globals(),
            now(),
        );
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("keychain access"));
        // Must NOT mislabel a denial as not-logged-in / suggest login.
        assert!(!r.checks[0].detail.contains("not logged in"));
        assert!(!r.checks[0].detail.contains("aiwitch login"));
    }

    #[test]
    fn full_meta_with_future_expiry_is_ok() {
        let r = build_report(
            &[ok_facts("personal", BackendKind::Codex)],
            &empty_globals(),
            now(),
        );
        assert_eq!(r.checks[0].status, Status::Ok);
        assert!(r.checks[0].detail.contains("u@e.com"));
        assert!(r.checks[0].detail.contains("plus"));
        assert!(r.checks[0].detail.contains("2027-01-01"));
    }

    #[test]
    fn expired_token_is_error() {
        let mut f = ok_facts("p", BackendKind::Codex);
        f.meta.as_mut().unwrap().subscription_until =
            Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Err);
        assert!(r.checks[0].detail.contains("expired"));
        assert!(r.checks[0].detail.contains("2026-01-01"));
    }

    #[test]
    fn expires_within_window_is_warning() {
        let mut f = ok_facts("p", BackendKind::Codex);
        f.meta.as_mut().unwrap().subscription_until =
            Some(Utc.with_ymd_and_hms(2026, 5, 12, 0, 0, 0).unwrap()); // 4 days from `now()`
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("expires in 4 days"));
    }

    #[test]
    fn expires_at_seven_days_is_still_ok() {
        let mut f = ok_facts("p", BackendKind::Codex);
        f.meta.as_mut().unwrap().subscription_until =
            Some(Utc.with_ymd_and_hms(2026, 5, 15, 0, 0, 0).unwrap()); // 7 days from `now()`
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Ok);
    }

    #[test]
    fn claude_no_credentials_on_macos_is_warning_keychain() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Claude, "/abs/p"),
            home_exists: true,
            credentials_present: false,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let g = GlobalFacts {
            is_macos: true,
            ..Default::default()
        };
        let r = build_report(&[f], &g, now());
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("keychain"));
    }

    #[test]
    fn claude_no_credentials_off_macos_is_warning_not_logged_in() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Claude, "/abs/p"),
            home_exists: true,
            credentials_present: false,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let g = GlobalFacts {
            is_macos: false,
            ..Default::default()
        };
        let r = build_report(&[f], &g, now());
        assert_eq!(r.checks[0].status, Status::Warn);
        assert!(r.checks[0].detail.contains("not logged in"));
    }

    #[test]
    fn cli_check_skipped_when_no_profile_uses_provider() {
        let g = GlobalFacts {
            codex_cli_on_path: None,
            claude_cli_on_path: None,
            ..Default::default()
        };
        let r = build_report(&[], &g, now());
        assert!(!r.checks.iter().any(|c| c.subject == "codex CLI"));
        assert!(!r.checks.iter().any(|c| c.subject == "claude CLI"));
    }

    #[test]
    fn cli_missing_is_error() {
        let g = GlobalFacts {
            codex_cli_on_path: Some(false),
            ..Default::default()
        };
        let r = build_report(&[], &g, now());
        let c = r.checks.iter().find(|c| c.subject == "codex CLI").unwrap();
        assert_eq!(c.status, Status::Err);
        assert!(c.detail.contains("not found"));
    }

    #[test]
    fn aiwitch_current_unset_is_ok() {
        let r = build_report(&[], &GlobalFacts::default(), now());
        let c = r.checks.last().unwrap();
        assert_eq!(c.subject, "AIWITCH_CURRENT");
        assert_eq!(c.status, Status::Ok);
        assert!(c.detail.contains("unset"));
    }

    #[test]
    fn aiwitch_current_unknown_profile_is_error() {
        let g = GlobalFacts {
            aiwitch_current: Some("ghost".into()),
            ..Default::default()
        };
        let r = build_report(&[ok_facts("real", BackendKind::Codex)], &g, now());
        let c = r
            .checks
            .iter()
            .find(|c| c.subject == "AIWITCH_CURRENT")
            .unwrap();
        assert_eq!(c.status, Status::Err);
        assert!(c.detail.contains("ghost"));
    }

    #[test]
    fn aiwitch_current_known_profile_is_ok() {
        let g = GlobalFacts {
            aiwitch_current: Some("real".into()),
            ..Default::default()
        };
        let r = build_report(&[ok_facts("real", BackendKind::Codex)], &g, now());
        let c = r
            .checks
            .iter()
            .find(|c| c.subject == "AIWITCH_CURRENT")
            .unwrap();
        assert_eq!(c.status, Status::Ok);
        assert!(c.detail.contains("real"));
    }

    #[test]
    fn render_aligns_subjects_by_max_width() {
        let r = Report {
            checks: vec![
                Check {
                    status: Status::Ok,
                    subject: "short".into(),
                    detail: "a".into(),
                },
                Check {
                    status: Status::Err,
                    subject: "much-longer-name".into(),
                    detail: "b".into(),
                },
            ],
        };
        let out = render(&r);
        // Both lines should be aligned: detail starts at the same column on each.
        let lines: Vec<&str> = out.lines().collect();
        let pos_a = lines[0].find('a').unwrap();
        let pos_b = lines[1].find('b').unwrap();
        assert_eq!(pos_a, pos_b);
    }

    #[test]
    fn render_summary_line_no_issues() {
        let r = Report {
            checks: vec![Check {
                status: Status::Ok,
                subject: "x".into(),
                detail: "y".into(),
            }],
        };
        let out = render(&r);
        assert!(out.ends_with("all checks passed\n"));
    }

    #[test]
    fn empty_profiles_emits_warning_with_hint() {
        let r = build_report(&[], &empty_globals(), now());
        let c = r.checks.iter().find(|c| c.subject == "profiles").unwrap();
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.contains("no profiles configured"));
        assert!(c.detail.contains("aiwitch add"));
    }

    #[test]
    fn claude_credentials_unparseable_is_error() {
        let f = ProfileFacts {
            profile: profile("p", BackendKind::Claude, "/abs/p"),
            home_exists: true,
            credentials_present: true,
            keychain: claude::keychain::KeychainStatus::NotApplicable,
            meta: None,
        };
        let r = build_report(&[f], &empty_globals(), now());
        assert_eq!(r.checks[0].status, Status::Err);
        assert!(r.checks[0].detail.contains(".credentials.json"));
        assert!(r.checks[0].detail.contains("failed to parse"));
    }

    #[test]
    fn read_strict_meta_returns_none_for_malformed_claude_credentials() {
        let tmp = std::env::temp_dir().join(format!(
            "aiwitch-doctor-claude-malformed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(".credentials.json"), "{ not json").unwrap();
        let p = profile("p", BackendKind::Claude, tmp.to_str().unwrap());

        let meta = read_strict_meta(&p);
        assert!(
            meta.is_none(),
            "malformed creds must surface as None for doctor"
        );

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn is_executable_file_rejects_non_executable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!(
            "aiwitch-doctor-exec-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let plain = tmp.join("plain");
        let exec = tmp.join("exec");
        std::fs::write(&plain, "").unwrap();
        std::fs::write(&exec, "").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!is_executable_file(&plain));
        assert!(is_executable_file(&exec));
        assert!(!is_executable_file(&tmp.join("missing")));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn render_summary_line_pluralizes() {
        let r = Report {
            checks: vec![
                Check {
                    status: Status::Err,
                    subject: "a".into(),
                    detail: String::new(),
                },
                Check {
                    status: Status::Err,
                    subject: "b".into(),
                    detail: String::new(),
                },
                Check {
                    status: Status::Warn,
                    subject: "c".into(),
                    detail: String::new(),
                },
            ],
        };
        let out = render(&r);
        assert!(out.ends_with("2 errors, 1 warning\n"));
    }
}
