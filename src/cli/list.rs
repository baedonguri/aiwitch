use crate::backend::{AnyBackend, Backend, BackendKind, ProfileMeta};
use crate::cli::current::{AIWITCH_CURRENT_KEY, EnvLookup, SystemEnv};
use crate::error::Result;
use crate::profile::{Profile, ProfilesFile, store};
use std::io::Write;

const CURRENT_MARK: &str = "*";

pub fn run() -> Result<()> {
    let profiles = store::load()?;
    let current = current_profiles(&SystemEnv, &profiles);
    let rows: Vec<(Profile, ProfileMeta)> = profiles
        .profiles
        .into_iter()
        .map(|p| {
            let meta = AnyBackend::from_kind(p.backend)
                .read_meta(&p)
                .unwrap_or_default();
            (p, meta)
        })
        .collect();
    let table = render(&rows, &current);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(table.as_bytes())?;
    Ok(())
}

fn current_profiles<E: EnvLookup>(env: &E, profiles: &ProfilesFile) -> Vec<String> {
    let mut selected = Vec::new();
    let mut selected_backends = Vec::new();

    if let Some(name) = env.get(AIWITCH_CURRENT_KEY)
        && let Some(profile) = profiles.profiles.iter().find(|p| p.name == name)
    {
        selected.push(profile.name.clone());
        selected_backends.push(profile.backend);
    }

    for profile in &profiles.profiles {
        if selected_backends.contains(&profile.backend) {
            continue;
        }

        let exports = AnyBackend::from_kind(profile.backend)
            .env_exports(profile)
            .unwrap_or_default();
        if exports.is_empty() {
            continue;
        }

        let matches = exports
            .iter()
            .all(|(key, value)| env.get(key).as_deref() == Some(value.as_str()));
        if matches {
            selected.push(profile.name.clone());
            selected_backends.push(profile.backend);
        }
    }

    selected
}

/** Pure renderer. Empty profiles -> header-only + hint line. */
pub fn render(rows: &[(Profile, ProfileMeta)], current: &[String]) -> String {
    let header = ["NAME", "PROVIDER", "EMAIL", "PLAN", "EXPIRES", "CURRENT"];
    if rows.is_empty() {
        return format!(
            "{}\n(no profiles configured — see ~/.config/aiwitch/profiles.toml)\n",
            header.join(" | ")
        );
    }

    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|(profile, meta)| {
            [
                profile.name.clone(),
                provider_label(profile.backend).to_string(),
                meta.email.clone().unwrap_or_else(|| "-".to_string()),
                meta.plan.clone().unwrap_or_else(|| "-".to_string()),
                meta.subscription_until
                    .map_or_else(|| "-".to_string(), |dt| dt.format("%Y-%m-%d").to_string()),
                if current.iter().any(|name| name == &profile.name) {
                    CURRENT_MARK.to_string()
                } else {
                    String::new()
                },
            ]
        })
        .collect();

    let mut widths = header.map(str::len);
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let mut out = String::new();
    push_row(&mut out, &header.map(str::to_string), &widths);
    out.push_str(&widths.map(|w| "-".repeat(w)).to_vec().join("+"));
    out.push('\n');
    for row in &cells {
        push_row(&mut out, row, &widths);
    }
    out
}

fn provider_label(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Codex => "codex",
    }
}

fn push_row<const N: usize>(out: &mut String, row: &[String; N], widths: &[usize; N]) {
    for (i, cell) in row.iter().enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        out.push_str(cell);
        if i + 1 < row.len() {
            out.push_str(&" ".repeat(widths[i] - cell.chars().count()));
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendKind, ProfileMeta};
    use crate::profile::{Profile, ProfilesFile};
    use chrono::{DateTime, Utc};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            backend: BackendKind::Codex,
            home_dir: PathBuf::from(format!("/abs/{name}")),
        }
    }

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn profiles(names: &[&str]) -> ProfilesFile {
        ProfilesFile {
            profiles: names.iter().map(|name| profile(name)).collect(),
        }
    }

    #[test]
    fn render_empty_profiles_shows_hint() {
        assert_eq!(
            render(&[], &[]),
            "NAME | PROVIDER | EMAIL | PLAN | EXPIRES | CURRENT\n(no profiles configured — see ~/.config/aiwitch/profiles.toml)\n"
        );
    }

    #[test]
    fn render_one_profile_full_meta() {
        let rows = [(
            profile("personal"),
            ProfileMeta {
                email: Some("a@b.com".to_string()),
                plan: Some("plus".to_string()),
                subscription_until: Some(dt("2026-05-05T12:34:56Z")),
            },
        )];

        assert_eq!(
            render(&rows, &[]),
            "NAME     | PROVIDER | EMAIL   | PLAN | EXPIRES    | CURRENT\n\
             --------+--------+-------+----+----------+-------\n\
             personal | codex    | a@b.com | plus | 2026-05-05 | \n"
        );
    }

    #[test]
    fn render_current_marker_after_expires() {
        let rows = [
            (profile("personal"), ProfileMeta::default()),
            (profile("work"), ProfileMeta::default()),
        ];

        assert_eq!(
            render(&rows, &["work".to_string()]),
            "NAME     | PROVIDER | EMAIL | PLAN | EXPIRES | CURRENT\n\
             --------+--------+-----+----+-------+-------\n\
             personal | codex    | -     | -    | -       | \n\
             work     | codex    | -     | -    | -       | *\n"
        );
    }

    #[test]
    fn render_one_profile_blank_meta_uses_dash() {
        let rows = [(profile("personal"), ProfileMeta::default())];

        assert_eq!(
            render(&rows, &[]),
            "NAME     | PROVIDER | EMAIL | PLAN | EXPIRES | CURRENT\n\
             --------+--------+-----+----+-------+-------\n\
             personal | codex    | -     | -    | -       | \n"
        );
    }

    #[test]
    fn render_multiple_profiles_pads_to_widest_name() {
        let rows = [
            (profile("p"), ProfileMeta::default()),
            (profile("long-name"), ProfileMeta::default()),
        ];

        assert_eq!(
            render(&rows, &[]),
            "NAME      | PROVIDER | EMAIL | PLAN | EXPIRES | CURRENT\n\
             ---------+--------+-----+----+-------+-------\n\
             p         | codex    | -     | -    | -       | \n\
             long-name | codex    | -     | -    | -       | \n"
        );
    }

    #[test]
    fn render_subscription_until_uses_ymd_format() {
        let rows = [(
            profile("p"),
            ProfileMeta {
                subscription_until: Some(dt("2026-12-31T23:59:59Z")),
                ..ProfileMeta::default()
            },
        )];

        let got = render(&rows, &[]);

        assert!(got.contains("2026-12-31"));
        assert!(!got.contains("23:59:59"));
    }

    #[test]
    fn current_profiles_uses_aiwitch_current_when_valid() {
        let env = env_map(&[("AIWITCH_CURRENT", "work")]);

        assert_eq!(
            current_profiles(&env, &profiles(&["personal", "work"])),
            vec!["work".to_string()]
        );
    }

    #[test]
    fn current_profiles_ignores_aiwitch_current_not_in_profiles() {
        let env = env_map(&[("AIWITCH_CURRENT", "ghost")]);

        assert_eq!(
            current_profiles(&env, &profiles(&["personal"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn current_profiles_falls_back_to_codex_home_match() {
        let env = env_map(&[("CODEX_HOME", "/abs/work")]);

        assert_eq!(
            current_profiles(&env, &profiles(&["personal", "work"])),
            vec!["work".to_string()]
        );
    }

    #[test]
    fn current_profiles_uses_one_marker_per_backend() {
        let env = env_map(&[("AIWITCH_CURRENT", "personal"), ("CODEX_HOME", "/abs/work")]);

        assert_eq!(
            current_profiles(&env, &profiles(&["personal", "work"])),
            vec!["personal".to_string()]
        );
    }
}
