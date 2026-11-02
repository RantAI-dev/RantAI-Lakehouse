//! The install's `.env`, as the tool reads it and writes it.
//!
//! One place reads and edits the file, so the install, the update, the
//! rollback and the status cannot disagree about a setting (rule 4).
//! The file holds the stack's secrets, so it is never printed: only
//! the values the tool needs — the published ports — are read here.
//!
//! # The text of the file
//!
//! The file is a list of `NAME=value` lines. A commented-out line sets
//! nothing, and a line without `=` is not a setting: the two rules
//! hold for every reader and every writer here, so a line the tool
//! wrote and a line the operator wrote mean the same thing.

use std::collections::BTreeMap;
use std::path::Path;

/// The settings the install's `.env` holds.
///
/// Only the lines that assign a value count: a commented-out line and
/// a line without `=` are not settings. A file that is not there is an
/// empty set of settings, not an error: the install writes it, and a
/// tool that asks about a setting before the install has one is asking
/// about a default.
#[must_use]
pub fn read(path: &Path) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return settings;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            settings.insert(
                name.trim().to_string(),
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    settings
}

/// The value the settings set for a port.
///
/// A value that is not a port is not a port: the fallback is used, and
/// the stack publishes the default one.
#[must_use]
pub fn port(settings: &BTreeMap<String, String>, name: &str, fallback: u16) -> u16 {
    settings
        .get(name)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(fallback)
}

/// The name a line sets, when it sets one.
///
/// A commented-out line sets nothing, and a line without `=` is not a
/// setting.
#[must_use]
pub fn name_of(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (name, _) = line.split_once('=')?;
    Some(name.trim())
}

/// The `.env` with a variable set to `value`.
///
/// A line that sets the variable already is REPLACED, so the file never
/// holds two of them and the last one is not the one that counts. A
/// variable the file does not set is added at the end.
#[must_use]
pub fn with_variable(text: &str, name: &str, value: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if name_of(line).is_some_and(|set| set == name) {
            if !replaced {
                lines.push(format!("{name}={value}"));
                replaced = true;
            }
            continue;
        }
        lines.push(line.to_string());
    }
    if !replaced {
        lines.push(format!("{name}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_settings_of_an_env_file_are_read() {
        let settings = read(Path::new("/does/not/exist"));
        assert!(
            settings.is_empty(),
            "a file that is not there has no settings"
        );
    }

    #[test]
    fn only_the_lines_that_assign_a_value_are_settings() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join(".env");
        std::fs::write(
            &path,
            "# a comment\nAPI_HOST_PORT=9090\n\nQUOTED=\"8080\"\nNOT_A_SETTING\n",
        )
        .expect("the env file is written");

        let settings = read(&path);
        assert_eq!(
            settings.get("API_HOST_PORT").map(String::as_str),
            Some("9090")
        );
        assert_eq!(settings.get("QUOTED").map(String::as_str), Some("8080"));
        assert!(
            !settings.contains_key("NOT_A_SETTING"),
            "a line without an equals sign is not a setting: {settings:?}"
        );
    }

    #[test]
    fn a_port_that_is_not_a_port_is_not_one() {
        let settings = read(Path::new("/does/not/exist"));
        assert_eq!(port(&settings, "API_HOST_PORT", 8080), 8080);
    }

    #[test]
    fn a_commented_out_line_sets_nothing() {
        assert_eq!(name_of("# COMPOSE_PROFILES=dagster"), None);
        assert_eq!(
            name_of("COMPOSE_PROFILES=dagster"),
            Some("COMPOSE_PROFILES")
        );
        assert_eq!(name_of("NOT_A_SETTING"), None);
    }

    #[test]
    fn a_variable_is_set_in_the_env_text() {
        let text = with_variable("POSTGRES_PASSWORD=secret\n", "COMPOSE_PROFILES", "dagster");
        assert!(
            text.contains("COMPOSE_PROFILES=dagster"),
            "the variable was not set: {text}"
        );
        assert!(
            text.contains("POSTGRES_PASSWORD=secret"),
            "the other settings were lost: {text}"
        );
    }

    #[test]
    fn a_variable_is_never_set_twice() {
        let text = with_variable(
            "COMPOSE_PROFILES=trino\nPOSTGRES_PASSWORD=secret\n",
            "COMPOSE_PROFILES",
            "dagster",
        );
        assert_eq!(
            text.matches("COMPOSE_PROFILES").count(),
            1,
            "the variable is set twice: {text}"
        );
        assert!(
            text.contains("COMPOSE_PROFILES=dagster"),
            "the variable was not set: {text}"
        );
    }
}
