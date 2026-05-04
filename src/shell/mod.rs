use crate::error::Result;
use anyhow::{anyhow, ensure};

#[derive(Debug, Clone, Copy)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
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

pub fn init_snippet(_shell: Shell) -> &'static str {
    todo!("emit shell function snippet for the selected shell")
}

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
}
