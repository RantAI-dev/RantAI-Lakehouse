//! Copilot citation checking (WS7 plan, grand plan §9): after the model's
//! final answer, every number and Markdown table it printed is matched
//! against the JSON already sitting in `tool_trace` — never re-run against
//! a live source, since `tool_trace` IS the record of what actually ran
//! for this turn. See this module's own doc comment for the complete,
//! exact matching rule set (WS7 plan, Phase F, WS7 item F1).
//!
//! # What `Verified` does and does not mean (N3)
//!
//! `NumberState::Verified` means the printed value MATCHES a numeric leaf
//! inside a successful (`ok: true`) tool-trace result, under the printed
//! unit's own transform, within tolerance — nothing more. It is a
//! coincidence check over VALUES, not a semantic check over MEANING: a
//! printed year that happens to equal an unrelated row's `tahun` column, a
//! row id, or an echoed `LIMIT` value all satisfy it. This is the grand
//! plan's own definition of citation-checking ("present in a tool
//! result") — this module implements that definition precisely and never
//! claims to prove the model's SENTENCE around the number is correct,
//! only that the DIGIT is not fabricated out of nothing. Every
//! user-facing surface (the UI tooltip, WS7 item F6) labels this "matches a
//! tool result", never bare "verified".

// This module lands one commit ahead of WS7 item F2, which wires
// `annotate_answer` (built on these primitives) into `chat()` — until
// that lands, nothing in production code calls this module yet, which
// `-D warnings` dead_code would otherwise flag. Real, disclosed
// sequencing (not a permanent exception): this `allow` is removed in the
// WS7 item F2 commit, once `chat()` makes everything here reachable.
#![allow(
    dead_code,
    reason = "wired into chat() by WS7 item F2, the very next commit in this phase"
)]

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberState {
    Verified,
    Unverified,
}

/// The closed unit table — WS7 item F1 rule 2. `None` is the identity
/// transform (a plain, unsuffixed number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    Percent,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terabytes,
    Petabytes,
    Milliseconds,
    Seconds,
}

impl Unit {
    /// Maps a raw tool-result leaf to the value a printed number under
    /// this unit is compared against.
    fn transform(self, raw: f64) -> f64 {
        match self {
            Self::None | Self::Milliseconds => raw,
            Self::Percent => raw * 100.0,
            Self::Kilobytes => raw / 1024.0,
            Self::Megabytes => raw / 1024f64.powi(2),
            Self::Gigabytes => raw / 1024f64.powi(3),
            Self::Terabytes => raw / 1024f64.powi(4),
            Self::Petabytes => raw / 1024f64.powi(5),
            Self::Seconds => raw / 1000.0,
        }
    }
}

/// Absolute tolerance: half of one decimal place — the coarsest real
/// rounding step this codebase's own formatters use (WS7 item F1 rule 3).
const ABS_TOLERANCE: f64 = 0.05;
/// Relative tolerance: 0.05% of the larger magnitude, absorbing
/// `Math.round`'s own rounding on a large count/duration.
const REL_TOLERANCE: f64 = 0.0005;

/// Whether `printed`, interpreted under `unit` (WS7 item F1 rule 2 — the SAME
/// transform the model's own suffix implies, never a broader search
/// across every transform), is verified by any successful (`ok: true`)
/// entry in `trace`.
#[must_use]
pub fn verify_number(printed: f64, unit: Unit, trace: &[Value]) -> NumberState {
    for entry in trace {
        if entry.get("ok").and_then(Value::as_bool) != Some(true) {
            continue; // rule 5: a failed call is never ground truth
        }
        let Some(result) = entry.get("result") else {
            continue;
        };
        for leaf in numeric_leaves(result) {
            if close(printed, unit.transform(leaf)) {
                return NumberState::Verified;
            }
        }
    }
    NumberState::Unverified
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= ABS_TOLERANCE.max(REL_TOLERANCE * a.abs().max(b.abs()))
}

/// Every `f64`-representable numeric leaf in `value`, recursively through
/// objects and arrays. Booleans and non-numeric strings are never coerced
/// (a `"status":"ok"` string is not a number a printed digit could ever
/// legitimately match).
fn numeric_leaves(value: &Value) -> Vec<f64> {
    match value {
        Value::Number(n) => n.as_f64().into_iter().collect(),
        Value::Object(map) => map.values().flat_map(numeric_leaves).collect(),
        Value::Array(items) => items.iter().flat_map(numeric_leaves).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod number_matching {
    use super::*;

    #[test]
    fn exact_integer_with_no_unit_matches_a_tool_result_number() {
        let trace =
            vec![serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"n":42}]}})];
        assert_eq!(
            verify_number(42.0, Unit::None, &trace),
            NumberState::Verified
        );
    }

    #[test]
    fn thousands_separated_number_already_parsed_matches_the_raw_value() {
        // Extraction (WS7 item F2) strips the "," before calling this
        // function — this test proves the MATCHING side of that
        // contract: a parsed 1234.0 matches a raw 1234 leaf.
        let trace =
            vec![serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"n":1234}]}})];
        assert_eq!(
            verify_number(1234.0, Unit::None, &trace),
            NumberState::Verified
        );
    }

    #[test]
    fn percent_unit_checks_only_the_times_100_transform() {
        let trace =
            vec![serde_json::json!({"tool":"describe_mart","ok":true,"result":{"pct":0.12}})];
        assert_eq!(
            verify_number(12.0, Unit::Percent, &trace),
            NumberState::Verified
        );
        // The SAME raw trace does NOT verify "12" printed with NO unit —
        // 0.12 under the identity transform is nowhere near 12.
        assert_eq!(
            verify_number(12.0, Unit::None, &trace),
            NumberState::Unverified
        );
    }

    #[test]
    fn gigabyte_unit_checks_only_the_div_1024_cubed_transform() {
        let trace = vec![
            serde_json::json!({"tool":"run_sql","ok":true,"result":{"scannedBytes":3_221_225_472_i64}}),
        ]; // exactly 3 GiB
        assert_eq!(
            verify_number(3.0, Unit::Gigabytes, &trace),
            NumberState::Verified
        );
        assert_eq!(
            verify_number(3.0, Unit::None, &trace),
            NumberState::Unverified
        );
    }

    #[test]
    fn seconds_unit_checks_the_div_1000_transform_against_a_ms_duration() {
        let trace =
            vec![serde_json::json!({"tool":"run_sql","ok":true,"result":{"durationMs":2500}})];
        assert_eq!(
            verify_number(2.5, Unit::Seconds, &trace),
            NumberState::Verified
        );
    }

    #[test]
    #[allow(
        clippy::approx_constant,
        reason = "3.14159 here is a deliberately pi-shaped test average \
                  from a mocked tool result, not a stand-in for PI"
    )]
    fn matches_within_the_real_half_decimal_rounding_tolerance() {
        // formatBytes/formatPercent/formatCost all round to at most ONE
        // decimal place — 3.14159 printed as "3.1" must verify.
        let trace = vec![
            serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"avg":3.14159}]}}),
        ];
        assert_eq!(
            verify_number(3.1, Unit::None, &trace),
            NumberState::Verified
        );
    }

    #[test]
    fn a_near_miss_outside_tolerance_is_unverified_not_verified() {
        let trace =
            vec![serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"n":100}]}})];
        assert_eq!(
            verify_number(103.0, Unit::None, &trace),
            NumberState::Unverified
        );
    }

    #[test]
    fn a_number_with_no_tool_result_at_all_is_unverified_never_omitted() {
        // Per the grand plan's own wording: a bare number is flagged
        // unverified, never removed — only a whole TABLE can be omitted.
        assert_eq!(
            verify_number(999.0, Unit::None, &[]),
            NumberState::Unverified
        );
    }

    #[test]
    fn a_failed_tool_call_result_never_verifies_a_number() {
        // ok: false results (a tool error object) must never be treated
        // as ground truth just because a number happens to appear in the
        // error payload (e.g. an echoed limit).
        let trace = vec![
            serde_json::json!({"tool":"run_sql","ok":false,"result":{"error":"...","limit":42}}),
        ];
        assert_eq!(
            verify_number(42.0, Unit::None, &trace),
            NumberState::Unverified
        );
    }
}
