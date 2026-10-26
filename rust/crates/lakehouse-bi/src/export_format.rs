//! How a chart's value format reaches a downloaded file (`BI-1`, feature
//! decision 7, plan decision D6).
//!
//! Two things come out of one [`ValueFormat`]:
//!
//! - [`format_text`]: the text the chart shows, for a CSV "formatted" file.
//!   It is a port of `formatValue` in `src/lib/value-format.ts` for the two
//!   locales the console ships (`en-US`, `id-ID`) and the five currencies
//!   whose `Intl` symbol it knows ([`symbol`]); any other currency is written
//!   as its code. The cases in `export_format_cases.jsonl` were produced by
//!   running the console's own `formatValue` (`bun`) over the formats and
//!   values listed there, and a unit test compares every line.
//! - [`excel_number_format`]: the Excel number-format string for a cell that
//!   stays a number, so the person can still sum and sort it. Excel's format
//!   language cannot say everything a [`ValueFormat`] says, and this function
//!   returns the closest format rather than pretending (principle 14):
//!   - separators are always written `.` and `,` in the file; the reader's
//!     Excel shows them by the reader's own regional settings, so the
//!     deployment's number locale does not travel with the file (it only picks
//!     the currency symbol);
//!   - compact has room for three tiers in Excel (two conditions and the
//!     rest): billions (`B`), millions (`M`) and everything below written in
//!     full. Thousands (`K`) and trillions (`T`) cannot be added, and a
//!     negative number never matches a `>=` condition, so it is written in
//!     full too;
//!   - a `number` format without decimals, and an `auto` format, are left to
//!     Excel's `General` (Excel has no "at most three decimals" code that does
//!     not leave a dangling point after a whole number);
//!   - a `percent` without decimals shows two, a `compact` without decimals
//!     one, a `scientific` without decimals two.
//!
//! The functions know nothing about rows or files.

use crate::style::{CurrencyDisplay, FormatStyle, ValueFormat};

/// A locale's separators.
#[derive(Clone, Copy)]
struct Separators {
    decimal: char,
    group: char,
}

/// `en-US` unless the locale is `id-ID` (the only two the console ships,
/// [`crate::formatting::NUMBER_LOCALES`]).
fn separators(locale: &str) -> Separators {
    if locale == "id-ID" {
        Separators {
            decimal: ',',
            group: '.',
        }
    } else {
        Separators {
            decimal: '.',
            group: ',',
        }
    }
}

/// Currencies `Intl` writes with no fraction digits by default.
const ZERO_DECIMAL: [&str; 11] = [
    "JPY", "KRW", "VND", "CLP", "ISK", "UGX", "XAF", "XOF", "XPF", "PYG", "RWF",
];
/// Currencies `Intl` writes with three fraction digits by default.
const THREE_DECIMAL: [&str; 7] = ["KWD", "BHD", "OMR", "JOD", "TND", "IQD", "LYD"];

/// The fraction digits `Intl` gives a currency with no `decimals` set.
fn currency_digits(code: &str) -> usize {
    if ZERO_DECIMAL.contains(&code) {
        0
    } else if THREE_DECIMAL.contains(&code) {
        3
    } else {
        2
    }
}

/// How a currency is written in front of the number: the text and whether a
/// space follows it. `Intl` (as `bun` ships it) writes `Rp 1.234,00` in
/// `id-ID` but `IDR 1,234.00` in `en-US`, `$1,234.00` and `US$1.234,00`; a
/// code (`IDR`, or any currency not known here) always has a space.
fn symbol(code: &str, display: CurrencyDisplay, locale: &str) -> (String, bool) {
    if display == CurrencyDisplay::Code {
        return (code.to_owned(), true);
    }
    let id = locale == "id-ID";
    match (code, id) {
        ("IDR", true) => ("Rp".to_owned(), true),
        ("USD", true) => ("US$".to_owned(), false),
        ("USD", false) => ("$".to_owned(), false),
        ("EUR", _) => ("€".to_owned(), false),
        ("GBP", _) => ("£".to_owned(), false),
        ("JPY", true) => ("JP¥".to_owned(), false),
        ("JPY", false) => ("¥".to_owned(), false),
        _ => (code.to_owned(), true),
    }
}

/// The compact step of `v`: the scaled number and its letter, or the number
/// itself below 1,000 (`compactParts` in the console).
fn compact_parts(v: f64) -> (f64, &'static str) {
    let a = v.abs();
    for (size, letter) in [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")] {
        if a >= size {
            return (v / size, letter);
        }
    }
    (v, "")
}

/// The digits of `abs` rounded half away from zero to `max_frac` places, as
/// (integer part, fraction part with at least `min_frac` digits and no
/// trailing zero beyond that). Rounds the exact binary expansion, as `Intl`
/// does, so `0.125` at two places is `0.13`.
fn round_digits(abs: f64, min_frac: usize, max_frac: usize) -> (String, String) {
    let exact = format!("{abs:.60}");
    let (int_part, frac_part) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).collect();
    let keep = int_part.len() + max_frac;
    if round_up_at(&mut digits, keep) {
        digits.insert(0, b'1');
    }
    let int_len = digits.len() - max_frac;
    let int_digits = String::from_utf8_lossy(&digits[..int_len]).into_owned();
    let mut frac_digits = String::from_utf8_lossy(&digits[int_len..]).into_owned();
    while frac_digits.len() > min_frac && frac_digits.ends_with('0') {
        frac_digits.pop();
    }
    let int_digits = int_digits.trim_start_matches('0');
    let int_digits = if int_digits.is_empty() {
        "0"
    } else {
        int_digits
    };
    (int_digits.to_owned(), frac_digits)
}

/// Cut `digits` (ASCII decimal digits) to `keep` of them, rounding half away
/// from zero on the first one dropped. `true` when the rounding carried out of
/// the first digit (all nines): the digits are then all zeros and the caller
/// puts a leading `1` in front.
fn round_up_at(digits: &mut Vec<u8>, keep: usize) -> bool {
    let round_up = digits.get(keep).is_some_and(|d| *d >= b'5');
    digits.truncate(keep);
    if !round_up {
        return false;
    }
    let mut at = digits.len();
    while at > 0 {
        at -= 1;
        if digits[at] == b'9' {
            digits[at] = b'0';
        } else {
            digits[at] += 1;
            return false;
        }
    }
    true
}

/// `v` as `Intl.NumberFormat` writes a plain number: at least `min_frac` and
/// at most `max_frac` fraction digits, grouped when asked, with the sign.
fn fixed(v: f64, (min_frac, max_frac): (usize, usize), grouping: bool, sep: Separators) -> String {
    let (int_digits, frac_digits) = round_digits(v.abs(), min_frac, max_frac);
    let mut out = String::new();
    if v.is_sign_negative() {
        out.push('-');
    }
    if grouping {
        let len = int_digits.len();
        for (i, c) in int_digits.chars().enumerate() {
            if i > 0 && (len - i) % 3 == 0 {
                out.push(sep.group);
            }
            out.push(c);
        }
    } else {
        out.push_str(&int_digits);
    }
    if !frac_digits.is_empty() {
        out.push(sep.decimal);
        out.push_str(&frac_digits);
    }
    out
}

/// The fraction digits the format asks for, or `fallback` as (min, max).
fn fraction(decimals: Option<u8>, fallback: (usize, usize)) -> (usize, usize) {
    decimals.map_or(fallback, |d| (usize::from(d), usize::from(d)))
}

/// `1.23E6` as `Intl` writes scientific notation: one digit before the point,
/// `decimals` after it (at most two, none trailing, when unset), the exponent
/// plain. Rounds half away from zero on the exact expansion, like [`fixed`].
fn scientific(v: f64, decimals: Option<u8>, sep: Separators) -> String {
    let (min_frac, max_frac) = decimals.map_or((0, 2), |d| (usize::from(d), usize::from(d)));
    // `{:e}` with many places prints the exact digits of the double.
    let exact = format!("{:.40e}", v.abs());
    let (mantissa, exponent) = exact.split_once('e').unwrap_or((exact.as_str(), "0"));
    let mut exponent: i32 = exponent.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    if round_up_at(&mut digits, 1 + max_frac) {
        // 9.99 rounded up is 10.0: one digit moves into the exponent.
        digits.insert(0, b'1');
        digits.truncate(1 + max_frac);
        exponent += 1;
    }
    let int_digit = char::from(digits[0]);
    let mut frac: String = digits[1..].iter().map(|d| char::from(*d)).collect();
    while frac.len() > min_frac && frac.ends_with('0') {
        frac.pop();
    }
    let sign = if v.is_sign_negative() { "-" } else { "" };
    let point = if frac.is_empty() {
        String::new()
    } else {
        format!("{}{frac}", sep.decimal)
    };
    format!("{sign}{int_digit}{point}E{exponent}")
}

/// The text a chart with `format` shows for `value`, in `locale`; `None` when
/// the format is `auto` (the console's own choice depends on the whole
/// column, so a file keeps the engine's digits) or the value is not a finite
/// number.
#[must_use]
pub fn format_text(value: f64, format: &ValueFormat, locale: &str) -> Option<String> {
    if !value.is_finite() || format.style == FormatStyle::Auto {
        return None;
    }
    let sep = separators(locale);
    let grouping = format.thousands;
    let body = match format.style {
        FormatStyle::Auto => return None,
        FormatStyle::Number => fixed(value, fraction(format.decimals, (0, 3)), grouping, sep),
        FormatStyle::Percent => {
            let n = fixed(
                value * 100.0,
                fraction(format.decimals, (0, 2)),
                grouping,
                sep,
            );
            format!("{n}%")
        }
        FormatStyle::Scientific => scientific(value, format.decimals, sep),
        FormatStyle::Compact => {
            let (scaled, letter) = compact_parts(value);
            if let Some(code) = &format.currency {
                // The console asks for one decimal on a letter step when none is set.
                let digits = currency_digits(code);
                let places = match (format.decimals, letter.is_empty()) {
                    (Some(d), _) => (usize::from(d), usize::from(d)),
                    (None, false) => (1, 1),
                    (None, true) => (digits, digits),
                };
                let text = currency_text(scaled, code, format, places, locale);
                if letter.is_empty() {
                    text
                } else {
                    format!("{text} {letter}")
                }
            } else {
                let n = fixed(scaled, fraction(format.decimals, (0, 1)), grouping, sep);
                if letter.is_empty() {
                    n
                } else {
                    format!("{n} {letter}")
                }
            }
        }
        FormatStyle::Currency => {
            let code = format.currency.as_deref().unwrap_or("IDR");
            let digits = currency_digits(code);
            currency_text(
                value,
                code,
                format,
                fraction(format.decimals, (digits, digits)),
                locale,
            )
        }
    };
    Some(format!(
        "{}{body}{}",
        format.prefix.as_deref().unwrap_or(""),
        format.suffix.as_deref().unwrap_or("")
    ))
}

/// A currency amount: sign, symbol, the number.
fn currency_text(
    value: f64,
    code: &str,
    format: &ValueFormat,
    places: (usize, usize),
    locale: &str,
) -> String {
    let sep = separators(locale);
    let (sym, spaced) = symbol(code, format.currency_display, locale);
    let number = fixed(value.abs(), places, format.thousands, sep);
    let sign = if value.is_sign_negative() { "-" } else { "" };
    format!("{sign}{sym}{}{number}", if spaced { " " } else { "" })
}

/// An Excel literal: the text in quotes, or, when it holds a quote, every
/// character escaped with a backslash.
fn literal(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else if text.contains('"') {
        text.chars().flat_map(|c| ['\\', c]).collect()
    } else {
        format!("\"{text}\"")
    }
}

/// `0.00` / `#,##0.00` from the places and grouping (no fraction part when
/// `places` is 0).
fn number_code(places: usize, grouping: bool) -> String {
    let int = if grouping { "#,##0" } else { "0" };
    if places == 0 {
        int.to_owned()
    } else {
        format!("{int}.{}", "0".repeat(places))
    }
}

/// The Excel number format for `format`, or `None` when the cell should keep
/// Excel's `General` (an `auto` format, a `number` with no decimals set).
/// See the module notes for what Excel cannot say.
///
/// ```text
/// style compact, currency IDR, decimals 2, id-ID:
/// [>=1000000000]"Rp "#,##0.00,,," B";[>=1000000]"Rp "#,##0.00,," M";"Rp "#,##0.00
/// ```
#[must_use]
pub fn excel_number_format(format: &ValueFormat, locale: &str) -> Option<String> {
    let pre = literal(format.prefix.as_deref().unwrap_or(""));
    let post = literal(format.suffix.as_deref().unwrap_or(""));
    let wrap = |code: String| format!("{pre}{code}{post}");
    let grouping = format.thousands;
    let money = |code: &str| {
        let (sym, spaced) = symbol(code, format.currency_display, locale);
        literal(&format!("{sym}{}", if spaced { " " } else { "" }))
    };
    match format.style {
        FormatStyle::Auto => None,
        FormatStyle::Number => format
            .decimals
            .map(|d| wrap(number_code(usize::from(d), grouping))),
        FormatStyle::Percent => Some(wrap(format!(
            "{}%",
            number_code(format.decimals.map_or(2, usize::from), false)
        ))),
        FormatStyle::Scientific => {
            let places = format.decimals.map_or(2, usize::from);
            let mantissa = if places == 0 {
                "0".to_owned()
            } else {
                format!("0.{}", "0".repeat(places))
            };
            Some(wrap(format!("{mantissa}E+00")))
        }
        FormatStyle::Currency => {
            let code = format.currency.as_deref().unwrap_or("IDR");
            let places = format
                .decimals
                .map_or_else(|| currency_digits(code), usize::from);
            Some(wrap(format!(
                "{}{}",
                money(code),
                number_code(places, grouping)
            )))
        }
        FormatStyle::Compact => {
            let places = format.decimals.map_or(1, usize::from);
            let lead = format.currency.as_deref().map(money).unwrap_or_default();
            let number = number_code(places, grouping);
            let tier = |commas: &str, letter: &str| {
                wrap(format!(
                    "{lead}{number}{commas}{}",
                    literal(&format!(" {letter}"))
                ))
            };
            Some(format!(
                "[>=1000000000]{};[>=1000000]{};{}",
                tier(",,,", "B"),
                tier(",,", "M"),
                wrap(format!("{lead}{number}"))
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::Value;

    use super::*;

    fn format(json: &str) -> ValueFormat {
        serde_json::from_str(json).unwrap()
    }

    /// Every line is `[locale, format, value, text]`, the text the console's
    /// `formatValue` wrote (see the module notes).
    #[test]
    fn the_csv_text_is_what_the_console_shows_for_every_recorded_case() {
        let cases = include_str!("export_format_cases.jsonl");
        let mut checked = 0;
        for line in cases.lines().filter(|l| !l.trim().is_empty()) {
            let case: Value = serde_json::from_str(line).unwrap();
            let [locale, fmt, value, expected] = case.as_array().unwrap().as_slice() else {
                panic!("a case is [locale, format, value, text]: {line}");
            };
            let fmt: ValueFormat = serde_json::from_value(fmt.clone()).unwrap();
            let got = format_text(value.as_f64().unwrap(), &fmt, locale.as_str().unwrap());
            if fmt.style == FormatStyle::Auto {
                assert_eq!(got, None);
            } else {
                assert_eq!(got.as_deref(), expected.as_str(), "{line}");
            }
            checked += 1;
        }
        assert!(checked > 300, "{checked} cases");
    }

    #[test]
    fn idr_compact_two_decimals_is_a_three_tier_excel_format() {
        let fmt = format(r#"{"style":"compact","currency":"IDR","decimals":2}"#);
        assert_eq!(
            excel_number_format(&fmt, "id-ID").as_deref(),
            Some(
                r##"[>=1000000000]"Rp "#,##0.00,,," B";[>=1000000]"Rp "#,##0.00,," M";"Rp "#,##0.00"##
            )
        );
        // en-US writes the code, as the console does.
        assert_eq!(
            excel_number_format(&fmt, "en-US").as_deref(),
            Some(
                r##"[>=1000000000]"IDR "#,##0.00,,," B";[>=1000000]"IDR "#,##0.00,," M";"IDR "#,##0.00"##
            )
        );
    }

    #[test]
    fn the_other_styles_map_to_the_closest_excel_codes() {
        let cases = [
            (r#"{"style":"auto"}"#, None),
            (r#"{"style":"number"}"#, None),
            (r#"{"style":"number","decimals":2}"#, Some("#,##0.00")),
            (r#"{"style":"number","decimals":0}"#, Some("#,##0")),
            (
                r#"{"style":"number","decimals":1,"thousands":false}"#,
                Some("0.0"),
            ),
            (r#"{"style":"percent"}"#, Some("0.00%")),
            (r#"{"style":"percent","decimals":0}"#, Some("0%")),
            (r#"{"style":"scientific","decimals":3}"#, Some("0.000E+00")),
            (
                r#"{"style":"currency","currency":"USD","decimals":2}"#,
                Some(r##""$"#,##0.00"##),
            ),
            (
                r#"{"style":"currency","currency":"IDR","currencyDisplay":"code","decimals":0}"#,
                Some(r##""IDR "#,##0"##),
            ),
            (
                r#"{"style":"number","decimals":0,"prefix":"~","suffix":" pcs"}"#,
                Some(r##""~"#,##0" pcs""##),
            ),
            (
                r#"{"style":"number","decimals":0,"prefix":"a\"b"}"#,
                Some(r#"\a\"\b#,##0"#),
            ),
        ];
        for (json, expected) in cases {
            assert_eq!(
                excel_number_format(&format(json), "en-US").as_deref(),
                expected,
                "{json}"
            );
        }
    }

    #[test]
    fn a_tie_rounds_away_from_zero_like_intl() {
        let fmt = format(r#"{"style":"number","decimals":2}"#);
        assert_eq!(format_text(0.125, &fmt, "en-US").as_deref(), Some("0.13"));
        assert_eq!(format_text(-0.125, &fmt, "en-US").as_deref(), Some("-0.13"));
        assert_eq!(format_text(0.995, &fmt, "en-US").as_deref(), Some("0.99"));
    }

    #[test]
    fn a_value_that_is_not_finite_or_an_auto_format_has_no_text() {
        let fmt = format(r#"{"style":"number"}"#);
        assert_eq!(format_text(f64::NAN, &fmt, "en-US"), None);
        assert_eq!(format_text(f64::INFINITY, &fmt, "en-US"), None);
        assert_eq!(format_text(1.0, &ValueFormat::default(), "en-US"), None);
    }
}
