//! CSV files that cannot run a formula (`SEC-17`).
//!
//! A spreadsheet reads a cell whose first character is `=`, `+`, `-` or `@`
//! (or a tab or a carriage return, which some programs skip before looking)
//! as a formula, so a value one person loaded runs as a formula on the
//! machine of the person who opens the export. The defence is a leading
//! apostrophe on such a cell, which a spreadsheet shows as text.
//!
//! The rule, identical to `escapeCell` in `src/lib/csv.ts` (feature page
//! `upload-limits-and-safe-csv`, decisions D7 and D8; plan decision E8):
//! a cell whose first character is `=`, `+`, `-`, `@`, U+0009 or U+000D gets
//! `'` in front, unless the whole cell is a plain number,
//! `^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$`, so `-5` and `+3.2e4` stay
//! numbers. The apostrophe goes on before quoting, so it ends up inside the
//! quotes. The test is on the cell's text alone: `-1+1` is not a number and
//! gets the apostrophe.
//!
//! [`neutralize_csv`] applies the rule to bytes `ClickHouse` wrote with
//! `FORMAT CSV`, field by field, with a small state machine instead of a CSV
//! library (this crate has none, and none is added). The price is that the
//! file no longer reads back as the same bytes; Parquet is the format for
//! exact data and is returned untouched.

/// Whether `cell` (the text between a field's quotes, or an unquoted
/// field) must get a leading apostrophe.
///
/// `cell` is the raw bytes of the field. For a quoted field they still hold
/// doubled quotes, which cannot change the answer: the first byte of a field
/// that starts with an escaped quote is `"`, not a trigger, and a number
/// holds no quote.
#[must_use]
pub fn needs_apostrophe(cell: &[u8]) -> bool {
    matches!(
        cell.first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')
    ) && !is_plain_number(cell)
}

/// `^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$` over bytes.
fn is_plain_number(cell: &[u8]) -> bool {
    let digits = |from: usize| {
        cell[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut at = usize::from(matches!(cell.first(), Some(b'+' | b'-')));
    let whole = digits(at);
    at += whole;
    if cell.get(at) == Some(&b'.') {
        at += 1;
        let fraction = digits(at);
        at += fraction;
        // `\.\d+` needs a digit after the point; `\d+\.\d*` already has one.
        if whole == 0 && fraction == 0 {
            return false;
        }
    } else if whole == 0 {
        return false;
    }
    if matches!(cell.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(cell.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = digits(at);
        if exponent == 0 {
            return false;
        }
        at += exponent;
    }
    at == cell.len()
}

/// Rewrite RFC 4180 CSV bytes so no field starts a formula. Fields are
/// separated by `,` and rows end with `\n`, `\r\n` or a lone `\r`, which is
/// what `ClickHouse`'s `FORMAT CSV` writes. Everything else, quoting
/// included, is copied as it is.
///
/// Never fails: bytes that are not well-formed CSV (a quote that is never
/// closed) are still copied through, with the rule applied where a field
/// can be told.
#[must_use]
pub fn neutralize_csv(input: &[u8]) -> Vec<u8> {
    let end = input.len();
    let mut out = Vec::with_capacity(end + end / 16);
    let mut at = 0;
    loop {
        // At the start of a field.
        if input.get(at) == Some(&b'"') {
            let mut close = at + 1;
            while close < end {
                if input[close] != b'"' {
                    close += 1;
                } else if input.get(close + 1) == Some(&b'"') {
                    close += 2;
                } else {
                    break;
                }
            }
            let content = &input[at + 1..close.min(end)];
            out.push(b'"');
            if needs_apostrophe(content) {
                out.push(b'\'');
            }
            out.extend_from_slice(content);
            at = close;
            if at < end {
                out.push(b'"');
                at += 1;
            }
            // Bytes after a closing quote up to the separator stay in the
            // field, as in the dialect the upload preview reads.
            let rest = input[at..]
                .iter()
                .take_while(|b| !matches!(**b, b',' | b'\n' | b'\r'))
                .count();
            out.extend_from_slice(&input[at..at + rest]);
            at += rest;
        } else {
            let length = input[at..]
                .iter()
                .take_while(|b| !matches!(**b, b',' | b'\n' | b'\r'))
                .count();
            let content = &input[at..at + length];
            if needs_apostrophe(content) {
                out.push(b'\'');
            }
            out.extend_from_slice(content);
            at += length;
        }
        // At a separator, or at the end.
        let Some(&separator) = input.get(at) else {
            return out;
        };
        out.push(separator);
        at += 1;
        if separator == b'\r' && input.get(at) == Some(&b'\n') {
            out.push(b'\n');
            at += 1;
        }
        if at >= end {
            // A row end at the very end of the file does not start a field.
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn neutralized(text: &str) -> String {
        String::from_utf8(neutralize_csv(text.as_bytes())).unwrap()
    }

    #[test]
    fn a_cell_that_starts_a_formula_gets_an_apostrophe() {
        for cell in ["=1+1", "+cmd", "-cmd", "@SUM(A1)", "\tx", "\rx"] {
            assert!(needs_apostrophe(cell.as_bytes()), "{cell:?}");
        }
    }

    #[test]
    fn an_ordinary_or_empty_cell_is_left_alone() {
        for cell in ["", "name", "a=b", "1", " =1", "'=1", "x-y"] {
            assert!(!needs_apostrophe(cell.as_bytes()), "{cell:?}");
        }
    }

    #[test]
    fn a_plain_number_is_left_alone() {
        for cell in [
            "-5", "+3.2e4", ".5", "-.5", "5.", "+0", "-1.5E-3", "1e10", "-0.0",
        ] {
            assert!(!needs_apostrophe(cell.as_bytes()), "{cell:?}");
        }
    }

    #[test]
    fn a_cell_that_only_looks_like_a_number_gets_an_apostrophe() {
        for cell in [
            "-1+1", "-", "+", "-.", "-e5", "-5e", "-5e+", "--5", "-5 ", "+1,5",
        ] {
            assert!(needs_apostrophe(cell.as_bytes()), "{cell:?}");
        }
    }

    #[test]
    fn quoted_and_unquoted_fields_are_both_rewritten_with_the_apostrophe_inside_the_quotes() {
        assert_eq!(neutralized("\"=1+1\",=2,x\n"), "\"'=1+1\",'=2,x\n");
    }

    #[test]
    fn a_quoted_field_with_commas_doubled_quotes_and_line_breaks_is_copied_whole() {
        let text = "\"a,b\",\"say \"\"hi\"\"\",\"line1\nline2\"\n";
        assert_eq!(neutralized(text), text);
        assert_eq!(
            neutralized("\"=a,\"\"b\"\"\nc\",x\n"),
            "\"'=a,\"\"b\"\"\nc\",x\n"
        );
    }

    #[test]
    fn a_quoted_field_that_starts_with_an_escaped_quote_is_not_a_formula() {
        let text = "\"\"\"=1\"\n";
        assert_eq!(neutralized(text), text);
    }

    #[test]
    fn a_quoted_number_and_a_quoted_tab_or_carriage_return() {
        assert_eq!(neutralized("\"-5\",\"+3.2e4\"\n"), "\"-5\",\"+3.2e4\"\n");
        assert_eq!(neutralized("\"\tx\",\"\rx\"\n"), "\"'\tx\",\"'\rx\"\n");
    }

    #[test]
    fn empty_fields_stay_empty() {
        assert_eq!(neutralized(",,\n,=1,\n"), ",,\n,'=1,\n");
        assert_eq!(neutralized("a,\n"), "a,\n");
        assert_eq!(neutralized("a,"), "a,");
        assert_eq!(neutralized(""), "");
    }

    #[test]
    fn crlf_lf_and_a_lone_cr_end_a_row() {
        assert_eq!(neutralized("=a\r\n=b\n=c\r=d"), "'=a\r\n'=b\n'=c\r'=d");
    }

    #[test]
    fn the_last_row_is_rewritten_with_or_without_a_row_end() {
        assert_eq!(neutralized("x\n=1"), "x\n'=1");
        assert_eq!(neutralized("x\n=1\n"), "x\n'=1\n");
    }

    #[test]
    fn a_quote_that_is_never_closed_does_not_loop_or_panic() {
        assert_eq!(neutralized("\"=abc"), "\"'=abc");
        assert_eq!(neutralized("\"abc\"\"def"), "\"abc\"\"def");
    }

    #[test]
    fn text_after_a_closing_quote_stays_in_the_field() {
        assert_eq!(neutralized("\"=a\"b,c\n"), "\"'=a\"b,c\n");
    }

    #[test]
    fn multibyte_text_is_copied_byte_for_byte() {
        let text = "\"héllo, wörld\",日本語\n";
        assert_eq!(neutralized(text), text);
    }
}
