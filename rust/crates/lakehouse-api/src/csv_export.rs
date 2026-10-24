//! The CSV of a whole-result export (`BI-16` part A, T8; `BI-1` D6 streams it).
//!
//! One writer, one set of rules: UTF-8 with a byte-order mark (so a
//! spreadsheet reads the encoding right), CRLF line ends and RFC 4180 quoting.
//! A text cell that starts with `=`, `+`, `-`, `@`, a tab or a carriage return
//! is prefixed with a single quote so a spreadsheet shows it as text instead
//! of running it as a formula (OWASP CSV-injection guidance); a JSON number
//! stays a number and is written as the digits the engine sent. This is the
//! only place the export neutralises cells, so the rule cannot drift between
//! callers.

use serde_json::Value;

/// The UTF-8 byte-order mark.
const BOM: &str = "\u{feff}";

/// A cell as the text that goes in the file, before quoting.
pub(crate) fn cell_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::String(s)) => neutralise(s),
        Some(other) => neutralise(&other.to_string()),
    }
}

/// Prefix a text that a spreadsheet could read as a formula. The one rule for
/// every download: the CSV writer below and the `.xlsx` writer
/// (`export.rs`) both call it (`BI-1` D6, AGENTS.md rule 4).
pub(crate) fn neutralise(text: &str) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

/// RFC 4180: a field with a comma, a double quote, CR or LF is quoted and its
/// quotes doubled.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// One line of the file from fields that are already text (neutralised by the
/// caller where they are text, [`neutralise`]): quoted when they need it, ended
/// by CRLF. `BI-1` D6 writes a download a line at a time.
pub(crate) fn line(fields: &[String]) -> String {
    let quoted: Vec<String> = fields.iter().map(|f| quote(f)).collect();
    format!("{}\r\n", quoted.join(","))
}

/// The start of every file: the byte-order mark and the header row of `labels`
/// (neutralised like any text cell).
pub(crate) fn header(labels: &[String]) -> String {
    let neutral: Vec<String> = labels.iter().map(|l| neutralise(l)).collect();
    format!("{BOM}{}", line(&neutral))
}

/// The whole file in memory, for the tests: a header row of `labels` (neutralised like any text cell), then
/// one row per entry of `rows` with the values of `columns`, in order.
#[cfg(test)]
pub(crate) fn write_csv(
    labels: &[String],
    columns: &[String],
    rows: &[serde_json::Map<String, Value>],
) -> String {
    let mut out = header(labels);
    for row in rows {
        let fields: Vec<String> = columns.iter().map(|c| cell_text(row.get(c))).collect();
        out.push_str(&line(&fields));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn row(v: &Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn the_file_has_a_bom_crlf_line_ends_and_a_labelled_header() {
        let csv = write_csv(
            &["Place".to_owned(), "Visitors".to_owned()],
            &["place".to_owned(), "visitors".to_owned()],
            &[row(&json!({ "place": "Ubud", "visitors": 12 }))],
        );
        assert_eq!(csv, "\u{feff}Place,Visitors\r\nUbud,12\r\n");
    }

    #[test]
    fn fields_with_a_comma_a_quote_or_a_line_break_are_quoted_and_quotes_doubled() {
        let csv = write_csv(
            &["a".to_owned()],
            &["a".to_owned()],
            &[
                row(&json!({ "a": "x,y" })),
                row(&json!({ "a": "say \"hi\"" })),
                row(&json!({ "a": "two\nlines" })),
            ],
        );
        assert_eq!(
            csv,
            "\u{feff}a\r\n\"x,y\"\r\n\"say \"\"hi\"\"\"\r\n\"two\nlines\"\r\n"
        );
    }

    #[test]
    fn a_text_that_could_run_as_a_formula_is_prefixed_and_a_number_is_not() {
        let cells = ["=1+1", "+1", "-1", "@SUM(A1)", "\tx", "\rx"];
        let rows: Vec<_> = cells.iter().map(|c| row(&json!({ "a": c }))).collect();
        let csv = write_csv(&["a".to_owned()], &["a".to_owned()], &rows);
        for c in cells {
            assert!(csv.contains(&format!("'{c}")), "{c:?} in {csv:?}");
        }
        // A real negative number is data, not text.
        let csv = write_csv(
            &["a".to_owned()],
            &["a".to_owned()],
            &[row(&json!({ "a": -5 }))],
        );
        assert_eq!(csv, "\u{feff}a\r\n-5\r\n");
        // A label is text too.
        let csv = write_csv(&["=evil".to_owned()], &["a".to_owned()], &[]);
        assert_eq!(csv, "\u{feff}'=evil\r\n");
    }

    #[test]
    fn nulls_are_empty_and_dates_and_big_numbers_keep_the_engines_text() {
        let csv = write_csv(
            &["d".to_owned(), "n".to_owned(), "z".to_owned()],
            &["d".to_owned(), "n".to_owned(), "z".to_owned()],
            &[row(&json!({ "d": "2026-03-05", "n": 1.5, "z": null }))],
        );
        assert_eq!(csv, "\u{feff}d,n,z\r\n2026-03-05,1.5,\r\n");
    }
}
