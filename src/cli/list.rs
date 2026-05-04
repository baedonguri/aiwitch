use crate::backend::{AnyBackend, Backend, BackendKind, ProfileMeta};
use crate::error::Result;
use crate::profile::{store, Profile};
use std::io::Write;

pub fn run() -> Result<()> {
    let profiles = store::load()?;
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
    let table = render(&rows);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(table.as_bytes())?;
    Ok(())
}

/** Pure renderer. Empty profiles -> header-only + hint line. */
pub fn render(rows: &[(Profile, ProfileMeta)]) -> String {
    let header = ["NAME", "BACKEND", "EMAIL", "PLAN", "EXPIRES"];
    if rows.is_empty() {
        return format!(
            "{}\n(no profiles configured — see ~/.config/aiwitch/profiles.toml)\n",
            header.join(" | ")
        );
    }

    let cells: Vec<[String; 5]> = rows
        .iter()
        .map(|(profile, meta)| {
            [
                profile.name.clone(),
                backend_label(profile.backend).to_string(),
                meta.email.clone().unwrap_or_else(|| "-".to_string()),
                meta.plan.clone().unwrap_or_else(|| "-".to_string()),
                meta.subscription_until
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "-".to_string()),
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

fn backend_label(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Codex => "codex",
    }
}

fn push_row(out: &mut String, row: &[String; 5], widths: &[usize; 5]) {
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
    use crate::profile::Profile;
    use chrono::{DateTime, Utc};
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

    #[test]
    fn render_empty_profiles_shows_hint() {
        assert_eq!(
            render(&[]),
            "NAME | BACKEND | EMAIL | PLAN | EXPIRES\n(no profiles configured — see ~/.config/aiwitch/profiles.toml)\n"
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
            render(&rows),
            "NAME     | BACKEND | EMAIL   | PLAN | EXPIRES\n\
             --------+-------+-------+----+----------\n\
             personal | codex   | a@b.com | plus | 2026-05-05\n"
        );
    }

    #[test]
    fn render_one_profile_blank_meta_uses_dash() {
        let rows = [(profile("personal"), ProfileMeta::default())];

        assert_eq!(
            render(&rows),
            "NAME     | BACKEND | EMAIL | PLAN | EXPIRES\n\
             --------+-------+-----+----+-------\n\
             personal | codex   | -     | -    | -\n"
        );
    }

    #[test]
    fn render_multiple_profiles_pads_to_widest_name() {
        let rows = [
            (profile("p"), ProfileMeta::default()),
            (profile("long-name"), ProfileMeta::default()),
        ];

        assert_eq!(
            render(&rows),
            "NAME      | BACKEND | EMAIL | PLAN | EXPIRES\n\
             ---------+-------+-----+----+-------\n\
             p         | codex   | -     | -    | -\n\
             long-name | codex   | -     | -    | -\n"
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

        let got = render(&rows);

        assert!(got.contains("2026-12-31"));
        assert!(!got.contains("23:59:59"));
    }
}
