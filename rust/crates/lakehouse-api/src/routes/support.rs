//! Small helpers shared by the read-only route modules (`catalog`, `overview`,
//! `ops`, `governance`, `storage`).

use std::fmt::Display;

use serde_json::{Map, Value};

/// Render any JSON value the way `String(x)` renders it in `TypeScript`,
/// with `null`/missing treated as `""` (matching `String(row[name] ?? "")`
/// in `src/app/api/catalog/[id]/route.ts`).
#[must_use]
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other @ (Value::Array(_) | Value::Object(_))) => other.to_string(),
    }
}

/// Parse a `ClickHouse` `toString(...)` column as an integer, defaulting to
/// `0` on a missing row, a missing column, or a value that doesn't parse —
/// matching `Number(x) || 0` in `TypeScript`, where `NaN` (a failed parse)
/// is falsy and also becomes `0`.
#[must_use]
pub(crate) fn num_or_zero(row: Option<&Map<String, Value>>, key: &str) -> i64 {
    row.and_then(|r| r.get(key))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// A `ClickHouse` string column, defaulting to `""` when the row or column
/// is missing.
#[must_use]
pub(crate) fn str_col<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `s.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase())` —
/// underscores become spaces, then the first letter of every word is
/// capitalized. Ported identically in `catalog/route.ts`,
/// `catalog/[id]/route.ts`.
#[must_use]
pub(crate) fn prettify(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut out = String::with_capacity(spaced.len());
    let mut at_word_start = true;
    for c in spaced.chars() {
        if at_word_start && c.is_ascii_alphanumeric() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        // `\w` is `[A-Za-z0-9_]`; since underscores were already replaced
        // with spaces, any non-alphanumeric character starts a new word.
        at_word_start = !c.is_ascii_alphanumeric();
    }
    out
}

/// Render an error the way `String(e)` renders a thrown `Error` in
/// `TypeScript`: `"Error: <message>"`. Every ported route's outer
/// `catch (e)` block builds its 503 body with `String(e)`.
#[must_use]
pub(crate) fn js_error(err: impl Display) -> String {
    format!("Error: {err}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn prettify_replaces_underscores_and_titlecases() {
        assert_eq!(prettify("mart_wisman"), "Mart Wisman");
    }

    #[test]
    fn prettify_handles_single_word() {
        assert_eq!(prettify("atlas"), "Atlas");
    }

    #[test]
    fn prettify_handles_leading_and_trailing_underscores() {
        assert_eq!(prettify("_foo_bar_"), " Foo Bar ");
    }

    #[test]
    fn js_string_converts_null_to_empty_string() {
        assert_eq!(js_string(Some(&Value::Null)), "");
        assert_eq!(js_string(None), "");
    }

    #[test]
    fn js_string_passes_through_plain_string() {
        assert_eq!(js_string(Some(&Value::String("2014".to_owned()))), "2014");
    }

    #[test]
    fn js_string_stringifies_number() {
        assert_eq!(js_string(Some(&serde_json::json!(476))), "476");
    }

    #[test]
    fn num_or_zero_defaults_on_missing_row() {
        assert_eq!(num_or_zero(None, "n"), 0);
    }

    #[test]
    fn num_or_zero_defaults_on_unparseable_value() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("not-a-number".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 0);
    }

    #[test]
    fn num_or_zero_parses_valid_string() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("42".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 42);
    }

    #[test]
    fn str_col_defaults_to_empty_string() {
        let m = Map::new();
        assert_eq!(str_col(&m, "missing"), "");
    }

    #[test]
    fn js_error_formats_like_string_of_error() {
        assert_eq!(js_error("bad sql"), "Error: bad sql");
    }
}
