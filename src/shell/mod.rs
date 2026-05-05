use crate::error::Result;
use anyhow::{anyhow, ensure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

/** Output flavor for `aiwitch env`. POSIX covers zsh/bash/sh/dash; Fish is its own. */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvFormat {
    Posix,
    Fish,
}

impl Default for EnvFormat {
    fn default() -> Self {
        EnvFormat::Posix
    }
}

/** POSIX single-quote escape. Wraps the value in `'...'` and replaces `'` with `'\''`.
 *  Safe for zsh/bash/sh/dash. Fish needs a separate quoter. */
pub fn sh_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/** Allowed: `[A-Za-z0-9_-]+`. Rejects empty input and a leading dash so the name
 *  cannot be parsed as a CLI flag. */
pub fn validate_profile_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "profile name must not be empty");
    ensure!(
        !name.starts_with('-'),
        "profile name must not start with '-': {name:?}"
    );
    let bytes = name.as_bytes();
    let ok = bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-');
    if !ok {
        return Err(anyhow!(
            "invalid profile name {:?}: only [A-Za-z0-9_-] allowed",
            name
        ));
    }
    Ok(())
}

/** Rejects NUL and control characters (other than space) so the value is safe to
 *  emit between single quotes inside a shell `eval`. Callers must run this
 *  before `sh_quote`. */
pub fn validate_env_value(value: &str) -> Result<()> {
    if let Some(bad) = value.chars().find(|c| *c == '\0' || (c.is_control() && *c != ' ')) {
        return Err(anyhow!(
            "env value contains forbidden control character {:?}",
            bad
        ));
    }
    Ok(())
}

/** Allowed: `^[A-Za-z_][A-Za-z0-9_]*$`. Catches anything that could turn an
 *  `export K=...` line into a different statement after `eval`. */
pub fn validate_env_key(key: &str) -> Result<()> {
    ensure!(!key.is_empty(), "env key must not be empty");
    let bytes = key.as_bytes();
    let first_ok = bytes[0].is_ascii_alphabetic() || bytes[0] == b'_';
    let rest_ok = bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_');
    if !first_ok || !rest_ok {
        return Err(anyhow!(
            "invalid env key {key:?}: only [A-Za-z_][A-Za-z0-9_]* allowed"
        ));
    }
    Ok(())
}

/** Fish single-quote escape. Inside `'...'` only `\` and `'` need escaping;
 *  `$`, backtick, and other shell metacharacters are literal. */
pub fn fish_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

/** Render `(key, value)` pairs as a complete shell snippet for the given format.
 *  Validates every key and value first; on any error nothing is emitted. */
pub fn render_env(format: EnvFormat, pairs: &[(String, String)]) -> Result<String> {
    let mut out = String::new();
    for (k, v) in pairs {
        validate_env_key(k)?;
        validate_env_value(v)?;
        match format {
            EnvFormat::Posix => {
                out.push_str("export ");
                out.push_str(k);
                out.push('=');
                out.push_str(&sh_quote(v));
            }
            EnvFormat::Fish => {
                out.push_str("set -gx ");
                out.push_str(k);
                out.push(' ');
                out.push_str(&fish_quote(v));
            }
        }
        out.push('\n');
    }
    Ok(out)
}

/** Shell function definition that wires up the `aiwitch use <profile>` alias. */
pub fn init_snippet(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh | Shell::Bash => POSIX_INIT,
        Shell::Fish => FISH_INIT,
    }
}

const POSIX_INIT: &str = r#"aiwitch() {
  case "$1" in
    add)
      shift
      __aiwitch_env="$(command aiwitch add --print-env "$@")" || return $?
      eval "$__aiwitch_env"
      printf 'switched: %s\n' "$(command aiwitch current)" >&2
      unset __aiwitch_env
      ;;
    use)
      shift
      __aiwitch_env="$(command aiwitch env "$@")" || return $?
      eval "$__aiwitch_env"
      unset __aiwitch_env
      ;;
    *)
      command aiwitch "$@"
      ;;
  esac
}
"#;

const FISH_INIT: &str = r#"function aiwitch
  switch $argv[1]
    case add
      set -l __aiwitch_env (command aiwitch add --print-env --shell=fish $argv[2..]); or return $status
      printf '%s\n' $__aiwitch_env | source
      printf 'switched: %s\n' (command aiwitch current) >&2
    case use
      set -l __aiwitch_env (command aiwitch env --shell=fish $argv[2..]); or return $status
      printf '%s\n' $__aiwitch_env | source
    case '*'
      command aiwitch $argv
  end
end
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_quote_empty() {
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn sh_quote_plain() {
        assert_eq!(sh_quote("hello"), "'hello'");
    }

    #[test]
    fn sh_quote_with_space() {
        assert_eq!(sh_quote("hello world"), "'hello world'");
    }

    #[test]
    fn sh_quote_with_single_quote() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn sh_quote_multiple_single_quotes() {
        assert_eq!(sh_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn sh_quote_only_single_quote() {
        assert_eq!(sh_quote("'"), "''\\'''");
    }

    #[test]
    fn sh_quote_shell_metacharacters_are_inert_inside_single_quotes() {
        assert_eq!(sh_quote("$VAR"), "'$VAR'");
        assert_eq!(sh_quote("`cmd`"), "'`cmd`'");
        assert_eq!(sh_quote("a\\b"), "'a\\b'");
        assert_eq!(sh_quote("a\"b"), "'a\"b'");
    }

    #[test]
    fn sh_quote_with_newline() {
        assert_eq!(sh_quote("a\nb"), "'a\nb'");
    }

    #[test]
    fn sh_quote_path_with_space() {
        let path = "/Users/x/with space/.codex-personal";
        assert_eq!(sh_quote(path), "'/Users/x/with space/.codex-personal'");
    }

    #[test]
    fn validate_accepts_simple() {
        assert!(validate_profile_name("personal").is_ok());
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("a").is_ok());
    }

    #[test]
    fn validate_accepts_alphanum_underscore_dash() {
        assert!(validate_profile_name("dev_env-2").is_ok());
        assert!(validate_profile_name("ABC-123_xyz").is_ok());
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn validate_rejects_dot() {
        assert!(validate_profile_name("with.dot").is_err());
    }

    #[test]
    fn validate_rejects_slash() {
        assert!(validate_profile_name("a/b").is_err());
        assert!(validate_profile_name("../etc").is_err());
    }

    #[test]
    fn validate_rejects_space() {
        assert!(validate_profile_name("a b").is_err());
    }

    #[test]
    fn validate_rejects_shell_metacharacters() {
        assert!(validate_profile_name("a;b").is_err());
        assert!(validate_profile_name("a$b").is_err());
        assert!(validate_profile_name("a`b").is_err());
        assert!(validate_profile_name("a|b").is_err());
        assert!(validate_profile_name("a&b").is_err());
        assert!(validate_profile_name("a\"b").is_err());
        assert!(validate_profile_name("a'b").is_err());
    }

    #[test]
    fn validate_rejects_newline_and_control() {
        assert!(validate_profile_name("a\nb").is_err());
        assert!(validate_profile_name("a\tb").is_err());
        assert!(validate_profile_name("a\0b").is_err());
    }

    #[test]
    fn validate_rejects_unicode() {
        assert!(validate_profile_name("café").is_err());
        assert!(validate_profile_name("naïve").is_err());
    }

    #[test]
    fn validate_rejects_leading_dash() {
        assert!(validate_profile_name("-foo").is_err());
        assert!(validate_profile_name("--foo").is_err());
        assert!(validate_profile_name("-").is_err());
    }

    #[test]
    fn validate_allows_internal_or_trailing_dash() {
        assert!(validate_profile_name("foo-bar").is_ok());
        assert!(validate_profile_name("foo-").is_ok());
    }

    #[test]
    fn validate_env_value_accepts_normal() {
        assert!(validate_env_value("").is_ok());
        assert!(validate_env_value("/Users/x/.codex").is_ok());
        assert!(validate_env_value("/Users/x/with space/.codex").is_ok());
        assert!(validate_env_value("personal").is_ok());
    }

    #[test]
    fn validate_env_value_rejects_newline() {
        assert!(validate_env_value("a\nb").is_err());
        assert!(validate_env_value("\n").is_err());
        assert!(validate_env_value("trailing\n").is_err());
    }

    #[test]
    fn validate_env_value_rejects_carriage_return() {
        assert!(validate_env_value("a\rb").is_err());
    }

    #[test]
    fn validate_env_value_rejects_tab() {
        assert!(validate_env_value("a\tb").is_err());
    }

    #[test]
    fn validate_env_value_rejects_null() {
        assert!(validate_env_value("a\0b").is_err());
    }

    #[test]
    fn validate_env_value_allows_space() {
        assert!(validate_env_value("hello world").is_ok());
    }

    #[test]
    fn validate_env_key_accepts_normal() {
        assert!(validate_env_key("CODEX_HOME").is_ok());
        assert!(validate_env_key("AIWITCH_CURRENT").is_ok());
        assert!(validate_env_key("_underscore").is_ok());
        assert!(validate_env_key("X").is_ok());
        assert!(validate_env_key("a1").is_ok());
    }

    #[test]
    fn validate_env_key_rejects_empty() {
        assert!(validate_env_key("").is_err());
    }

    #[test]
    fn validate_env_key_rejects_leading_digit() {
        assert!(validate_env_key("1FOO").is_err());
    }

    #[test]
    fn validate_env_key_rejects_metacharacters() {
        assert!(validate_env_key("BAD;cmd").is_err());
        assert!(validate_env_key("BAD KEY").is_err());
        assert!(validate_env_key("BAD=VAL").is_err());
        assert!(validate_env_key("BAD-DASH").is_err());
        assert!(validate_env_key("$BAD").is_err());
        assert!(validate_env_key("BAD'inject").is_err());
    }

    #[test]
    fn fish_quote_empty() {
        assert_eq!(fish_quote(""), "''");
    }

    #[test]
    fn fish_quote_plain() {
        assert_eq!(fish_quote("hello"), "'hello'");
    }

    #[test]
    fn fish_quote_with_space() {
        assert_eq!(fish_quote("hello world"), "'hello world'");
    }

    #[test]
    fn fish_quote_dollar_is_literal() {
        assert_eq!(fish_quote("$VAR"), "'$VAR'");
    }

    #[test]
    fn fish_quote_backtick_is_literal() {
        assert_eq!(fish_quote("`cmd`"), "'`cmd`'");
    }

    #[test]
    fn fish_quote_escapes_single_quote() {
        assert_eq!(fish_quote("it's"), "'it\\'s'");
    }

    #[test]
    fn fish_quote_escapes_backslash() {
        assert_eq!(fish_quote("a\\b"), "'a\\\\b'");
    }

    #[test]
    fn fish_quote_double_quote_is_literal() {
        assert_eq!(fish_quote("a\"b"), "'a\"b'");
    }

    #[test]
    fn render_env_posix_basic() {
        let pairs = vec![
            ("CODEX_HOME".to_string(), "/Users/x/.codex".to_string()),
            ("AIWITCH_CURRENT".to_string(), "personal".to_string()),
        ];
        let got = render_env(EnvFormat::Posix, &pairs).unwrap();
        assert_eq!(
            got,
            "export CODEX_HOME='/Users/x/.codex'\nexport AIWITCH_CURRENT='personal'\n"
        );
    }

    #[test]
    fn render_env_fish_basic() {
        let pairs = vec![
            ("CODEX_HOME".to_string(), "/Users/x/.codex".to_string()),
            ("AIWITCH_CURRENT".to_string(), "personal".to_string()),
        ];
        let got = render_env(EnvFormat::Fish, &pairs).unwrap();
        assert_eq!(
            got,
            "set -gx CODEX_HOME '/Users/x/.codex'\nset -gx AIWITCH_CURRENT 'personal'\n"
        );
    }

    #[test]
    fn render_env_posix_path_with_space() {
        let pairs = vec![("CODEX_HOME".to_string(), "/with space/.codex".to_string())];
        let got = render_env(EnvFormat::Posix, &pairs).unwrap();
        assert_eq!(got, "export CODEX_HOME='/with space/.codex'\n");
    }

    #[test]
    fn render_env_rejects_bad_key() {
        let pairs = vec![("BAD;cmd".to_string(), "v".to_string())];
        assert!(render_env(EnvFormat::Posix, &pairs).is_err());
    }

    #[test]
    fn render_env_rejects_bad_value() {
        let pairs = vec![("OK".to_string(), "with\nnewline".to_string())];
        assert!(render_env(EnvFormat::Posix, &pairs).is_err());
    }

    #[test]
    fn render_env_empty_pairs_yields_empty_string() {
        let got = render_env(EnvFormat::Posix, &[]).unwrap();
        assert_eq!(got, "");
    }

    #[test]
    fn init_snippet_posix_has_use_alias() {
        let s = init_snippet(Shell::Zsh);
        assert!(s.contains("aiwitch()"));
        assert!(s.contains("command aiwitch env"));
        assert_eq!(init_snippet(Shell::Bash), s);
    }

    #[test]
    fn init_snippet_posix_propagates_failure_and_forwards_args() {
        let s = init_snippet(Shell::Zsh);
        assert!(s.contains("|| return $?"), "must propagate exit status");
        assert!(s.contains("\"$@\""), "must forward all remaining args after shift");
        assert!(s.contains("shift"));
    }

    #[test]
    fn init_snippet_posix_auto_switches_after_add() {
        let s = init_snippet(Shell::Zsh);
        assert!(s.contains("case \"$1\" in"));
        assert!(s.contains("add)"));
        assert!(s.contains("command aiwitch add --print-env"));
        assert!(s.contains("eval \"$__aiwitch_env\""));
        assert!(s.contains("switched: %s"));
        assert!(s.contains(">&2"));
    }

    #[test]
    fn init_snippet_fish_uses_source_pipe_with_flag() {
        let s = init_snippet(Shell::Fish);
        assert!(s.contains("function aiwitch"));
        assert!(s.contains("command aiwitch env --shell=fish"));
        assert!(s.contains("| source") || s.contains("|source"));
    }

    #[test]
    fn init_snippet_fish_propagates_failure_and_forwards_args() {
        let s = init_snippet(Shell::Fish);
        assert!(s.contains("or return $status"), "must propagate exit status");
        assert!(s.contains("$argv[2..]"), "must forward all remaining args");
    }

    #[test]
    fn init_snippet_fish_auto_switches_after_add() {
        let s = init_snippet(Shell::Fish);
        assert!(s.contains("case add"));
        assert!(s.contains("command aiwitch add --print-env --shell=fish"));
        assert!(s.contains("| source"));
        assert!(s.contains("switched: %s"));
        assert!(s.contains(">&2"));
    }
}
