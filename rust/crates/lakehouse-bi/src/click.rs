//! What a click on a chart does, and a dashboard's saved auto-refresh
//! (`BI-18` part B).
//!
//! A chart with no `click` opens the drill menu, as every chart saved before
//! this existed does. The server checks only the shape and size of a click
//! and the URL rule below; it does not look for the board, the column or the
//! saved query, because any of them can be created or deleted after the
//! chart is saved. The console says so when the target is missing.
//!
//! The URL rule is the security part. A click URL is written by someone who
//! may edit the dashboard and followed by whoever views it, so it is limited
//! to `http`, `https` or a path inside the console. Everything else is
//! refused at save, in particular `javascript:` and `data:` (script in the
//! viewer's session), `//host` (a scheme-relative link that leaves the
//! console), and a `{value}` in the scheme or host position (the clicked
//! cell would choose where the viewer goes).

use serde::{Deserialize, Serialize};

/// The longest id of a board or a saved query.
const ID_MAX_CHARS: usize = 128;
/// The longest column name.
const COLUMN_MAX_CHARS: usize = 128;
/// The longest URL template.
const URL_MAX_CHARS: usize = 2048;

/// The placeholder in a URL template that the console replaces with the
/// clicked value, URL-encoded.
pub const URL_PLACEHOLDER: &str = "{value}";

/// The auto-refresh intervals a dashboard may save, in seconds: off, then
/// 1, 5, 10, 15, 30 and 60 minutes (Metabase's list, `BI-18` decision 4).
pub const REFRESH_SECONDS: [u32; 7] = [0, 60, 300, 600, 900, 1800, 3600];

/// What a click on a chart does instead of opening the drill menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClickAction {
    /// Open another dashboard filtered to the clicked value.
    Dashboard {
        /// The destination dashboard's id.
        board: String,
        /// The destination column the value filters.
        column: String,
    },
    /// Open a saved query in Query Studio.
    Query {
        /// The saved query's id.
        id: String,
    },
    /// Open a URL with the clicked value inserted at `{value}`.
    Url {
        /// The template.
        url: String,
    },
}

impl ClickAction {
    /// Checks the shape and size of the click and, for a URL, the URL rule.
    ///
    /// # Errors
    ///
    /// A plain message of ours naming what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Dashboard { board, column } => {
                check_id(board, "dashboard")?;
                if column.trim().is_empty() {
                    return Err("choose the column of the dashboard the click filters.".to_owned());
                }
                if column.chars().count() > COLUMN_MAX_CHARS || column.chars().any(char::is_control)
                {
                    return Err("the click's column name is not valid.".to_owned());
                }
                Ok(())
            }
            Self::Query { id } => check_id(id, "saved query"),
            Self::Url { url } => validate_click_url(url),
        }
    }
}

/// A board or saved-query id: present, short, no spaces or control
/// characters (the console puts it in a path or query string, encoded).
fn check_id(id: &str, what: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err(format!("choose the {what} the click opens."));
    }
    if id.chars().count() > ID_MAX_CHARS || id.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(format!("the click's {what} id is not valid."));
    }
    Ok(())
}

/// The URL rule of `BI-18` part B: `https://…`, `http://…`, or a path that
/// starts with a single `/`, with `{value}` allowed anywhere but the scheme
/// and the host.
///
/// Backslashes and whitespace are refused everywhere: browsers read `\` as
/// `/`, so `/\host` would be the scheme-relative link the rule exists to
/// stop, and whitespace or control characters are how a scheme gets hidden
/// from a prefix check.
///
/// # Errors
///
/// A plain message of ours naming what is wrong.
pub fn validate_click_url(url: &str) -> Result<(), String> {
    let refused = |why: &str| Err(format!("the click URL is not allowed: {why}."));
    if url.is_empty() {
        return refused("it is empty");
    }
    if url.chars().count() > URL_MAX_CHARS {
        return refused("it is too long");
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return refused("it has spaces or control characters");
    }
    if url.contains('\\') {
        return refused("it has a backslash");
    }
    if let Some(rest) = url.strip_prefix('/') {
        // `//host` leaves the console; `/\host` is refused above.
        if rest.starts_with('/') {
            return refused("a path must start with a single /");
        }
        return Ok(());
    }
    // The scheme is compared case-insensitively on its own bytes, so a
    // multi-byte character just after it cannot split a boundary.
    let Some(after_scheme) = ["https://", "http://"].into_iter().find_map(|scheme| {
        url.get(..scheme.len())
            .filter(|head| head.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
    }) else {
        return refused("use https://, http:// or a path starting with /");
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.is_empty() {
        return refused("it has no host");
    }
    if authority.contains(URL_PLACEHOLDER) || authority.contains('{') {
        return refused("the value cannot be part of the host");
    }
    Ok(())
}

/// Checks a saved auto-refresh interval.
///
/// # Errors
///
/// A plain message of ours listing the accepted intervals.
pub fn validate_refresh_seconds(seconds: u32) -> Result<(), String> {
    if REFRESH_SECONDS.contains(&seconds) {
        Ok(())
    } else {
        Err(format!(
            "refreshSeconds must be one of {}.",
            REFRESH_SECONDS.map(|s| s.to_string()).join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn url(u: &str) -> Result<(), String> {
        validate_click_url(u)
    }

    #[test]
    fn the_accepted_forms_pass() {
        for ok in [
            "https://example.com",
            "https://example.com/a?b={value}",
            "HTTP://Example.COM/{value}",
            "https://example.com:8443/x#{value}",
            "https://example.com?q={value}",
            "https://user@example.com/x",
            "/dashboards/b_1?f={value}",
            "/",
            "/{value}",
        ] {
            assert_eq!(url(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn script_and_data_schemes_are_refused() {
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert({value})",
            "data:text/html,hi",
            "vbscript:x",
            "file:///etc/passwd",
            "ftp://example.com",
            "mailto:a@example.com",
            "example.com/x",
            "https:example.com",
            "https:/example.com",
            " javascript:alert(1)",
            "java\tscript:alert(1)",
            "https//example.com",
            "",
        ] {
            assert!(url(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_scheme_relative_link_is_refused_in_every_spelling() {
        for bad in ["//example.com", "///example.com", "/\\example.com", "\\\\x"] {
            assert!(url(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn the_value_cannot_choose_the_scheme_or_the_host() {
        for bad in [
            "{value}://example.com",
            "{value}",
            "https://{value}/x",
            "https://{value}.example.com/x",
            "https://example.com{value}",
            "https://user:{value}@example.com/x",
            "https://example.com:{value}/x",
            "http://{value}",
        ] {
            assert!(url(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn an_url_with_no_host_or_with_hidden_characters_is_refused() {
        for bad in [
            "https://",
            "https:///path",
            "https://?x",
            "https://example.com/a b",
            "https://example.com/\u{0}",
            "https://example.com/\u{85}",
        ] {
            assert!(url(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_non_ascii_url_does_not_panic_on_the_scheme_check() {
        assert!(url("ｈｔｔｐｓ://example.com").is_err());
        assert!(url("https:/é").is_err());
        assert!(url("é").is_err());
    }

    #[test]
    fn a_url_over_the_size_limit_is_refused() {
        let long = format!("https://example.com/{}", "a".repeat(URL_MAX_CHARS));
        assert!(url(&long).is_err());
    }

    #[test]
    fn a_dashboard_click_needs_a_board_and_a_column() {
        let click = |board: &str, column: &str| ClickAction::Dashboard {
            board: board.to_owned(),
            column: column.to_owned(),
        };
        assert_eq!(click("b_1", "region").validate(), Ok(()));
        assert!(click("", "region").validate().is_err());
        assert!(click("b_1", "  ").validate().is_err());
        assert!(click("b 1", "region").validate().is_err());
        assert!(
            click("b_1", &"c".repeat(COLUMN_MAX_CHARS + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_query_click_needs_an_id() {
        let click = |id: &str| ClickAction::Query { id: id.to_owned() };
        assert_eq!(click("sq-1").validate(), Ok(()));
        assert!(click("").validate().is_err());
        assert!(click("a\nb").validate().is_err());
    }

    #[test]
    fn a_click_reads_and_writes_the_documented_shapes() {
        let dashboard: ClickAction =
            serde_json::from_str(r#"{"kind":"dashboard","board":"b_1","column":"kab"}"#).unwrap();
        assert_eq!(
            dashboard,
            ClickAction::Dashboard {
                board: "b_1".to_owned(),
                column: "kab".to_owned()
            }
        );
        let query: ClickAction = serde_json::from_str(r#"{"kind":"query","id":"sq-1"}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&query).unwrap(),
            r#"{"kind":"query","id":"sq-1"}"#
        );
        let url: ClickAction =
            serde_json::from_str(r#"{"kind":"url","url":"/a/{value}"}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&url).unwrap(),
            r#"{"kind":"url","url":"/a/{value}"}"#
        );
        assert!(serde_json::from_str::<ClickAction>(r#"{"kind":"menu"}"#).is_err());
    }

    #[test]
    fn only_the_listed_refresh_intervals_are_accepted() {
        for ok in REFRESH_SECONDS {
            assert_eq!(validate_refresh_seconds(ok), Ok(()));
        }
        for bad in [1, 59, 61, 120, 3601, 86_400, u32::MAX] {
            assert!(validate_refresh_seconds(bad).is_err(), "{bad}");
        }
    }
}
