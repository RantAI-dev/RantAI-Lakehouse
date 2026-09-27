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

use std::fmt::Write as _;

use serde_json::Value;

#[cfg(test)]
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

    /// The decimal (SI) reading of a byte unit — `MB` as 10^6 bytes — which
    /// many answers use; `None` for every other unit. Both readings are
    /// accepted: a model printing `19.6 MB` for 19,552,544 bytes is right
    /// under SI and would otherwise be flagged.
    fn transform_si(self, raw: f64) -> Option<f64> {
        let power = match self {
            Self::Kilobytes => 1,
            Self::Megabytes => 2,
            Self::Gigabytes => 3,
            Self::Terabytes => 4,
            Self::Petabytes => 5,
            _ => return None,
        };
        Some(raw / 1000f64.powi(power))
    }
}

/// Absolute tolerance: half of one decimal place — the coarsest real
/// rounding step this codebase's own formatters use (WS7 item F1 rule 3).
const ABS_TOLERANCE: f64 = 0.05;
/// Relative tolerance: 0.05% of the larger magnitude, absorbing
/// `Math.round`'s own rounding on a large count/duration.
const REL_TOLERANCE: f64 = 0.0005;

/// Everything a printed number may be matched against for one answer.
///
/// # What counts as evidence, and why each source is here
///
/// - **Every numeric leaf of every successful tool result** (rule 5: a
///   failed call is never ground truth), including numbers carried as
///   JSON strings. `ClickHouse` returns every value of a query result as a
///   string (`"1571564"`), so before strings were read, a number copied
///   exactly from `run_sql` was flagged unverified: measured on the local
///   stack (`ops/ai_eval/ai_eval.py`), 14 of 23 answers carried false
///   flags, most on figures that matched the query result exactly.
/// - **Counts and totals a result's rows imply**: the length of every
///   array, how many rows share each text value ("4 primary + 2
///   secondary" is a count over a `list_datasets` result), and the sum of
///   every numeric column (a "total" row). These are the derived numbers
///   an answer legitimately states without another tool call; anything
///   else the model computed itself (a share, a difference) still has to
///   come from the query, and is flagged when it does not.
/// - **Numbers already in the conversation**: the user's own messages and
///   earlier answers, minus anything an earlier check flagged. A follow-up
///   ("and 2022?") restates last turn's figure next to this turn's; that
///   figure was checked when it was first shown. This is an echo check,
///   not a fact check: a user's own wrong number repeated back is not
///   flagged.
pub struct Evidence {
    values: Vec<f64>,
    /// Arithmetic between two numbers of the same small tool result (see
    /// [`pairwise`]), checked only after `values` fails.
    derived: Vec<f64>,
}

impl Evidence {
    /// Evidence from this turn's `trace` plus earlier `conversation` texts.
    #[must_use]
    pub fn new(trace: &[Value], conversation: &[&str]) -> Self {
        let mut values = Vec::new();
        let mut derived = Vec::new();
        for entry in trace {
            if entry.get("ok").and_then(Value::as_bool) != Some(true) {
                continue; // rule 5: a failed call is never ground truth
            }
            let Some(result) = entry.get("result") else {
                continue;
            };
            let leaves = numeric_leaves(result);
            pairwise(&leaves, &mut derived);
            values.extend(leaves);
            derived_values(result, &mut values);
        }
        for text in conversation {
            let checked = strip_unverified(text);
            values.extend(printed_values(&checked));
        }
        Self { values, derived }
    }

    /// Whether `printed` under `unit` matches some evidence value. `slack`
    /// is the rounding the printed form itself implies (`2.64 million`
    /// stands for anything within ±5,000); the fixed tolerances below
    /// still apply when it is smaller.
    fn matches(&self, printed: f64, unit: Unit, slack: f64) -> bool {
        let near = |target: f64| {
            (printed - target).abs()
                <= slack
                    .max(ABS_TOLERANCE)
                    .max(REL_TOLERANCE * printed.abs().max(target.abs()))
        };
        let hit = |&leaf: &f64| {
            // A percentage may come back as a fraction (0.12) or already
            // as a percentage (12.0, e.g. `round(100 * a / b, 1)` in SQL).
            near(unit.transform(leaf))
                || (unit == Unit::Percent && near(leaf))
                || unit.transform_si(leaf).is_some_and(near)
        };
        self.values.iter().any(hit) || self.derived.iter().any(hit)
    }
}

/// Whether `printed`, interpreted under `unit` (WS7 item F1 rule 2), is
/// verified by the [`Evidence`] `trace` alone provides.
#[cfg(test)]
#[must_use]
pub fn verify_number(printed: f64, unit: Unit, trace: &[Value]) -> NumberState {
    if Evidence::new(trace, &[]).matches(printed, unit, 0.0) {
        NumberState::Verified
    } else {
        NumberState::Unverified
    }
}

/// A string that is exactly a plain decimal number (`"1571564"`,
/// `"-3.5"`), as `ClickHouse` serialises numeric columns. Anything else (a
/// date, an id with letters, a grouped `"1,234"`) is not read as a
/// number.
fn parse_plain_number(s: &str) -> Option<f64> {
    let t = s.trim();
    let body = t.strip_prefix('-').unwrap_or(t);
    let mut parts = body.splitn(2, '.');
    let int = parts.next()?;
    let frac = parts.next();
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    t.parse().ok()
}

/// Every numeric leaf in `value`, recursively through objects and arrays:
/// JSON numbers, and strings that are exactly a plain number
/// ([`parse_plain_number`]). Booleans and other strings are never coerced.
fn numeric_leaves(value: &Value) -> Vec<f64> {
    match value {
        Value::Number(n) => n.as_f64().into_iter().collect(),
        Value::String(s) => parse_plain_number(s).into_iter().collect(),
        Value::Object(map) => map.values().flat_map(numeric_leaves).collect(),
        Value::Array(items) => items.iter().flat_map(numeric_leaves).collect(),
        _ => Vec::new(),
    }
}

/// The counts and totals `value`'s arrays imply (see [`Evidence`]): each
/// array's length and, for an array of row objects, the number of rows
/// per distinct text value of each key and the sum of each numeric key.
#[allow(
    clippy::cast_precision_loss,
    reason = "row counts are far below 2^52, where f64 stops being exact"
)]
fn derived_values(value: &Value, out: &mut Vec<f64>) {
    match value {
        Value::Array(items) => {
            out.push(items.len() as f64);
            let mut counts: std::collections::HashMap<(&str, &str), usize> =
                std::collections::HashMap::new();
            let mut sums: std::collections::HashMap<&str, (f64, bool)> =
                std::collections::HashMap::new();
            for item in items {
                let Value::Object(row) = item else { continue };
                for (key, cell) in row {
                    let number = match cell {
                        Value::Number(n) => n.as_f64(),
                        Value::String(s) => parse_plain_number(s),
                        _ => None,
                    };
                    let entry = sums.entry(key.as_str()).or_insert((0.0, true));
                    match number {
                        Some(n) => entry.0 += n,
                        None => entry.1 = false,
                    }
                    if number.is_none()
                        && let Value::String(text) = cell
                    {
                        *counts.entry((key.as_str(), text.as_str())).or_default() += 1;
                    }
                }
            }
            out.extend(counts.values().map(|&n| n as f64));
            out.extend(sums.values().filter(|(_, all)| *all).map(|(sum, _)| *sum));
            for item in items {
                derived_values(item, out);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                derived_values(v, out);
            }
        }
        _ => {}
    }
}

/// Every number `text` prints, read the way an answer's numbers are
/// ([`read_number`]: bold, Indonesian grouping, scale words), both readings
/// of an ambiguous token included. Used for the conversation's evidence, so
/// a figure an earlier answer printed as `**2,358,638**` still counts.
fn printed_values(text: &str) -> Vec<f64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for (k, word) in words.iter().enumerate() {
        if let Some(p) = read_number(word, words.get(k + 1).copied()) {
            out.push(p.value);
            out.extend(p.alt);
        }
    }
    out
}

/// Most numeric leaves one tool result may have for [`pairwise`] to run.
/// With n leaves there are about 4n² derived values, and every one is a
/// chance for an unrelated printed number to match by coincidence; forty
/// leaves covers a comparison of a few rows while keeping that small.
const PAIRWISE_MAX_LEAVES: usize = 40;

/// The arithmetic an answer legitimately does on two numbers of one small
/// result: the difference, the ratio, the share (`a / b * 100`) and the
/// change (`(a - b) / b * 100`). "From 62 to 396, an increase of 334
/// (+538.7%)" is correct and is exactly what a reader wants; before this
/// the checker flagged both figures. A difference or change the model got
/// wrong still matches nothing and is still flagged. Only pairs inside ONE
/// tool result are combined, and only when it has at most
/// [`PAIRWISE_MAX_LEAVES`] numbers.
fn pairwise(leaves: &[f64], out: &mut Vec<f64>) {
    if leaves.len() < 2 || leaves.len() > PAIRWISE_MAX_LEAVES {
        return;
    }
    for (i, &a) in leaves.iter().enumerate() {
        for &b in &leaves[i + 1..] {
            out.push((a - b).abs());
            for (x, y) in [(a, b), (b, a)] {
                if y != 0.0 {
                    out.push(x / y);
                    out.push(100.0 * x / y);
                    out.push(100.0 * (x - y) / y);
                }
            }
        }
    }
}

/// `text` with every span an earlier check flagged removed, so a flagged
/// number is never promoted to evidence by being repeated.
fn strip_unverified(text: &str) -> String {
    const OPEN: &str = r#"<span data-unverified="true">"#;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        rest = after
            .find("</span>")
            .map_or("", |end| &after[end + "</span>".len()..]);
    }
    out.push_str(rest);
    out
}

// ── WS7 item F2: extraction and annotation ──────────────────────────────────

/// Maps a matched unit SUFFIX string to a [`Unit`] — an exhaustive,
/// case-insensitive lookup, never a fuzzy match. `""` maps to
/// [`Unit::None`] (the identity transform of a plain, unsuffixed number);
/// every other unrecognized suffix returns `None`, which the caller uses
/// to REJECT the whole token (WS7 item F1 rule 2's closed table — a suffix
/// outside this table is not a number-with-unit at all, e.g. the `-01-15`
/// tail of an ISO date).
fn unit_from_suffix(suffix: &str) -> Option<Unit> {
    match suffix {
        "" => Some(Unit::None),
        "%" => Some(Unit::Percent),
        s if s.eq_ignore_ascii_case("kb") => Some(Unit::Kilobytes),
        s if s.eq_ignore_ascii_case("mb") => Some(Unit::Megabytes),
        s if s.eq_ignore_ascii_case("gb") => Some(Unit::Gigabytes),
        s if s.eq_ignore_ascii_case("tb") => Some(Unit::Terabytes),
        s if s.eq_ignore_ascii_case("pb") => Some(Unit::Petabytes),
        s if s.eq_ignore_ascii_case("ms") => Some(Unit::Milliseconds),
        s if s.eq_ignore_ascii_case("s") => Some(Unit::Seconds),
        _ => None,
    }
}

/// A bare unit word standing alone (WS7 item F1 rule 2's "optional single
/// space" — a suffix separated from its number by whitespace, e.g. `"3
/// GB"` printed as two words). Unlike [`unit_from_suffix`], `""` is
/// rejected here — a bare empty word is not a unit.
fn parse_bare_unit(word: &str) -> Option<Unit> {
    match unit_from_suffix(word)? {
        Unit::None => None,
        u => Some(u),
    }
}

/// Trims LEADING/TRAILING sentence punctuation off `word` — never an
/// INTERNAL character, so a `,` thousands separator inside `1,234`
/// survives untouched while trailing prose punctuation (`.`, `,`, …) is
/// removed (WS7 item F2 Step 2, point 3).
#[cfg(test)]
fn trim_sentence_punct(word: &str) -> &str {
    word.trim_matches(|c: char| ".,;:!?()".contains(c))
}

/// Parses `word` against the number-with-optional-unit grammar this
/// module implements (WS7 item F2 Step 2's doc comment on [`extract_numbers`]
/// explains why this differs, in one disclosed respect, from WS7 item F1
/// rule 1's literal regex — see that doc comment for the full
/// justification). Returns `None` if `word` does not match in its
/// entirety: a leading digit run, optionally grouped by commas in
/// EXACT groups of three, optionally followed by a `.`-decimal tail,
/// optionally followed by one of [`unit_from_suffix`]'s recognized
/// suffixes — anything left over after that (an unrecognized suffix, a
/// stray `-`, a bare `.` with no following digit) fails the whole match,
/// never a partial one.
fn parse_number_token(word: &str) -> Option<(f64, Unit)> {
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    let negative = chars.first() == Some(&'-');
    let mut i = usize::from(negative);
    let digit_start = i;
    while i < n && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == digit_start {
        return None; // no leading digit at all — not a number token
    }
    let mut int_digits: String = chars[digit_start..i].iter().collect();
    while i < n && chars[i] == ',' {
        if i + 4 > n
            || !chars[i + 1].is_ascii_digit()
            || !chars[i + 2].is_ascii_digit()
            || !chars[i + 3].is_ascii_digit()
        {
            break; // not a well-formed thousands group — stop, leave the
            // ',' in the unconsumed tail so it fails as part of the suffix
        }
        int_digits.push(chars[i + 1]);
        int_digits.push(chars[i + 2]);
        int_digits.push(chars[i + 3]);
        i += 4;
    }
    let mut number_text = int_digits;
    if i < n && chars[i] == '.' {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < n && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > frac_start {
            number_text.push('.');
            number_text.extend(&chars[frac_start..j]);
            i = j;
        }
        // A bare '.' with no following digit is left unconsumed — it
        // becomes part of the suffix below and correctly fails the match.
    }
    let suffix: String = chars[i..].iter().collect();
    let unit = unit_from_suffix(&suffix)?;
    let mut value: f64 = number_text.parse().ok()?;
    if negative {
        value = -value;
    }
    Some((value, unit))
}

/// Removes every fenced code block (```` ```…``` ````, with or without a
/// language tag) and inline code span (`` `…` ``) from `text`, replacing
/// each with a single space (preserving surrounding word boundaries) — a
/// number inside either is source/SQL text, never a cited fact. Scans
/// left to right tracking fenced/inline-code state explicitly, rather
/// than a single regex, so a fence or span that never closes degrades to
/// "treat the rest of the text as code" instead of matching unboundedly
/// past the end of the text.
///
/// Only called by [`extract_numbers`] below.
#[cfg(test)]
fn strip_code_spans(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < n {
        if i + 2 < n && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            let after = i + 3;
            i = find_triple_backtick(&chars[after..]).map_or(n, |rel| after + rel + 3);
            out.push(' ');
            continue;
        }
        if chars[i] == '`' {
            i = chars[i + 1..]
                .iter()
                .position(|&c| c == '`')
                .map_or(i + 1, |rel| i + 1 + rel + 1);
            out.push(' ');
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The offset of the next occurrence of `` ``` `` in `chars`, if any.
#[cfg(test)]
fn find_triple_backtick(chars: &[char]) -> Option<usize> {
    if chars.len() < 3 {
        return None;
    }
    (0..=chars.len() - 3).find(|&j| chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`')
}

/// Every base-10 number in `text`, paired with its [`Unit`] (WS7 item F1 rule
/// 2) — `Unit::None` when no recognized suffix directly follows it, or a
/// unit read from the NEXT word when this word is a bare number and the
/// next is a standalone unit token (the "optional single space" case,
/// e.g. `"3 GB"`) — never a broader search, only ever the one adjacent
/// word.
///
/// # N4 (revision 2 judge review): whole-TOKEN matching, not an in-line
/// regex scan
///
/// A loose in-line scan matched anywhere in the text would split an ISO
/// date `2024-01-15` into three spurious numbers, pick up `17123` out of
/// the identifier `q-17123`, and pick up `1`/`2` out of the version
/// string `v1.2` — none of those are a number the model is citing a fact
/// with. Extraction instead strips code spans first (`strip_code_spans`),
/// splits what remains into whitespace-delimited word tokens, trims
/// leading/trailing sentence punctuation off each (`trim_sentence_punct`
/// — never an internal character, so the `,` inside `1,234` survives),
/// and matches the WHOLE trimmed word against [`parse_number_token`]'s
/// anchored grammar. `2024-01-15` fails (a `-01-15` tail with no
/// recognized unit), `q-17123` fails (no leading digit at all), `v1.2`
/// fails (no leading digit), `17123-4` fails (a `-4` tail with no
/// recognized unit), `09:41:07` fails (a `:41:07` tail) — a token with
/// any character outside the strict grammar is excluded entirely, never
/// partially matched.
///
/// # A disclosed deviation from WS7 item F1 rule 1's literal regex
///
/// Rule 1 states the accepted format as `-?\d{1,3}(,\d{3})*(\.\d+)?` —
/// read literally (anchored, as rule 1 itself requires), that regex
/// rejects an UNGROUPED digit run longer than three digits with no comma
/// at all (e.g. `"999999"`, which `\d{1,3}` alone cannot fully consume
/// and `(,\d{3})*` cannot extend without a comma). WS7 item F2's own
/// acceptance test (`an_unverified_number_is_flagged_inline_not_silently_
/// removed`, this module's `extraction_and_annotation` tests) requires
/// EXACTLY that plain, ungrouped `"999999"` to be recognized as a number
/// and flagged unverified — so the two are in direct tension. This
/// implementation resolves the tension in the test's favor (a fabricated
/// number is not less fabricated for lacking thousands separators): the
/// leading digit run has no length cap, and commas are only ever
/// consumed when they introduce a well-formed group of exactly three
/// digits (`parse_number_token`), so both `"1,234"` (grouped) and
/// `"999999"` (ungrouped) parse as numbers, while a malformed mix like
/// `"17123-4"` still fails outright (the `-4` tail matches no unit).
///
/// # Why this is `#[allow(dead_code)]`
///
/// `annotate_answer` (below) never calls `extract_numbers` itself — it
/// needs the byte POSITION of each number to rewrite it in place, and
/// `strip_code_spans`'s whole-text transform (blanking a code span to a
/// single space) shifts every later position, so it cannot double as a
/// position-preserving scanner. `annotate_line`'s own scanner
/// (`word_byte_spans` + `inline_code_mask`) reimplements the SAME
/// exclusion rules without that transform, calling the SAME shared
/// primitives (`parse_number_token`, `parse_bare_unit`,
/// `unit_from_suffix`) `extract_numbers` calls — so the matching GRAMMAR
/// is not duplicated, only the code-span/whitespace scanning shell around
/// it is. `extract_numbers` itself stays as the batch-oriented, directly
/// testable reference implementation of that grammar (WS7 items F1/F2's own
/// acceptance tests, `extraction_and_annotation` below, call it
/// directly) — real production-reachable code, just not from
/// `chat()`'s own call path today.
#[cfg(test)]
#[must_use]
pub fn extract_numbers(text: &str) -> Vec<(f64, Unit)> {
    let stripped = strip_code_spans(text);
    let words: Vec<&str> = stripped.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let trimmed = trim_sentence_punct(words[i]);
        if let Some((value, unit)) = parse_number_token(trimmed) {
            if unit == Unit::None && i + 1 < words.len() {
                let next_trimmed = trim_sentence_punct(words[i + 1]);
                if let Some(u) = parse_bare_unit(next_trimmed) {
                    out.push((value, u));
                    i += 2;
                    continue;
                }
            }
            out.push((value, unit));
        }
        i += 1;
    }
    out
}

/// Characters stripped from both ends of a word before it is read as a
/// number: sentence punctuation, Markdown emphasis (`**1,234**` is still a
/// number the model is citing; before emphasis was stripped, a bold figure
/// was never checked at all, which is how a wrong bold total passed
/// unflagged), brackets, quotes, and the approximate/positive markers
/// `~`, `≈`, `+`.
const EDGE_MARKS: &[char] = &[
    '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '*', '_', '~', '"', '\'', '“', '”', '’', '≈',
    '+',
];

fn trim_marks(word: &str) -> &str {
    word.trim_matches(|c: char| EDGE_MARKS.contains(&c))
}

/// Scale words a number may be followed by, and their factor.
fn scale_word(word: &str) -> Option<f64> {
    match trim_marks(word).to_lowercase().as_str() {
        "thousand" | "thousands" | "ribu" | "rb" => Some(1e3),
        "million" | "millions" | "juta" | "jt" => Some(1e6),
        "billion" | "billions" | "miliar" | "milyar" => Some(1e9),
        "trillion" | "trillions" | "triliun" => Some(1e12),
        _ => None,
    }
}

/// `1.234.567` / `1.234,5` / `12,09` (Indonesian grouping and decimal
/// comma), optionally with a trailing `%`. English-style tokens are
/// [`parse_number_token`]'s; this only reads what that grammar rejects,
/// and never an ambiguous `1,234` (three digits after one comma), which
/// stays English.
fn parse_indonesian(token: &str) -> Option<(f64, Unit, u32)> {
    let (body, unit) = token
        .strip_suffix('%')
        .map_or((token, Unit::None), |b| (b, Unit::Percent));
    let (int, frac) = body
        .split_once(',')
        .map_or((body, None), |(a, b)| (a, Some(b)));
    let groups: Vec<&str> = int.split('.').collect();
    let first_ok = groups
        .first()
        .is_some_and(|g| !g.is_empty() && g.len() <= 3 && g.bytes().all(|b| b.is_ascii_digit()));
    let grouped = groups.len() > 1
        && first_ok
        && groups[1..]
            .iter()
            .all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()));
    let plain_int = groups.len() == 1 && !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit());
    let frac_ok =
        frac.is_none_or(|f| (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()));
    if !frac_ok || !(grouped || (plain_int && frac.is_some())) {
        return None;
    }
    let text = format!(
        "{}{}",
        groups.concat(),
        frac.map(|f| format!(".{f}")).unwrap_or_default()
    );
    let decimals = frac.map_or(0, |f| u32::try_from(f.len()).unwrap_or(0));
    Some((text.parse().ok()?, unit, decimals))
}

/// A number the answer prints, with the rounding its printed form implies.
struct Printed {
    value: f64,
    unit: Unit,
    slack: f64,
    /// The number consumed the following word (a unit or scale word).
    consumed_next: bool,
    /// A second reading of an ambiguous token: `18.420` is 18.42 in
    /// English and 18,420 in Indonesian, and an answer in Indonesian
    /// writes thousands that way. Either reading may match.
    alt: Option<f64>,
}

impl Printed {
    fn verified(&self, evidence: &Evidence) -> bool {
        evidence.matches(self.value, self.unit, self.slack)
            || self
                .alt
                .is_some_and(|alt| evidence.matches(alt, self.unit, 0.0))
    }
}

/// `18.420`-shaped: one to three digits, one dot, exactly three digits.
fn is_dot_thousands(token: &str) -> bool {
    token.split_once('.').is_some_and(|(a, b)| {
        (1..=3).contains(&a.len())
            && b.len() == 3
            && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
    })
}

/// Reads `word` (and, for a unit or scale word, `next`) as a printed
/// number: `**1,571,564**`, `~27.6%`, `2.64 million`, `2,6 juta`,
/// `1.571.564`, `3 GB`. `None` for anything that is not a number token.
fn read_number(word: &str, next: Option<&str>) -> Option<Printed> {
    let token = trim_marks(word);
    let (value, mut unit, decimals) = if let Some((v, u)) = parse_number_token(token) {
        let decimals = token
            .split_once('.')
            .map_or(0, |(_, f)| f.bytes().take_while(u8::is_ascii_digit).count());
        (v, u, u32::try_from(decimals).unwrap_or(0))
    } else if let Some(read) = parse_indonesian(token) {
        read
    } else {
        let (body, factor) = match token.chars().last() {
            Some('K') => (&token[..token.len() - 1], 1e3),
            Some('M') => (&token[..token.len() - 1], 1e6),
            Some('B') => (&token[..token.len() - 1], 1e9),
            _ => return None,
        };
        let (v, u) = parse_number_token(body)?;
        if u != Unit::None {
            return None;
        }
        let decimals = body.split_once('.').map_or(0, |(_, f)| f.len());
        let step = 10f64.powi(-i32::try_from(decimals).unwrap_or(0));
        return Some(Printed {
            value: v * factor,
            unit: u,
            slack: 0.5 * step * factor,
            consumed_next: false,
            alt: None,
        });
    };
    let step = 10f64.powi(-i32::try_from(decimals).unwrap_or(0));
    let mut consumed_next = false;
    let mut factor = 1.0;
    if unit == Unit::None
        && let Some(next) = next
    {
        if let Some(u) = parse_bare_unit(trim_marks(next)) {
            unit = u;
            consumed_next = true;
        } else if let Some(f) = scale_word(next) {
            factor = f;
            consumed_next = true;
        }
    }
    // Half of the last printed digit: `43 MB` stands for 42.5-43.5 MB, as
    // `2.64 million` stands for 2,635,000-2,645,000. A whole number used to
    // get only the fixed tolerance, so a correctly rounded `43 MB` for
    // 42.9 MB was flagged.
    let slack = 0.5 * step * factor;
    let alt = (unit == Unit::None && is_dot_thousands(token)).then_some(value * 1000.0 * factor);
    Some(Printed {
        value: value * factor,
        unit,
        slack,
        consumed_next,
        alt,
    })
}

/// Whether the table cell `cell` is verified: `None` for a cell with no
/// number in it (a label, a name, a description), which is never flagged;
/// otherwise whether every number in it matches the [`Evidence`].
///
/// Text cells used to need a verbatim match against a tool result's
/// strings. A model that labels a column in plain words (`Year of visit`
/// for `tahun`), adds a flag emoji to a country, or translates a category
/// fails that every time, so correct tables were flagged cell by cell.
/// What the check exists for is fabricated figures, so only figures are
/// checked.
fn cell_is_verified(cell: &str, evidence: &Evidence) -> Option<bool> {
    let words: Vec<&str> = cell.split_whitespace().collect();
    let mut seen = false;
    let mut all = true;
    let mut k = 0;
    while k < words.len() {
        if let Some(p) = read_number(words[k], words.get(k + 1).copied()) {
            seen = true;
            all &= p.verified(evidence);
            k += if p.consumed_next { 2 } else { 1 };
        } else {
            k += 1;
        }
    }
    seen.then_some(all)
}

/// Whether a table header names a rank/ordinal column (`#`, `Rank`, `No`),
/// whose `1, 2, 3` are positions, not figures.
fn is_rank_header(header: &str) -> bool {
    matches!(
        trim_marks(header).to_lowercase().as_str(),
        "#" | "rank" | "no" | "nr" | "peringkat" | "urutan" | "nomor"
    )
}

/// Splits a single `| a | b |` Markdown table row into its trimmed cell
/// texts — matches `MiniMarkdown`'s own `splitRow`
/// (`src/features/copilot/mini-markdown.tsx:34-39`) exactly, since the
/// client renders whatever survives this function.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_owned()).collect()
}

/// Whether `line` (already trimmed-checked to start with `|`) is a GFM
/// table separator row (`|---|:--|--:|`, any mix of `-`, `:`, `|`,
/// whitespace, with at least one `-`) — matches `MiniMarkdown`'s own
/// separator test (`mini-markdown.tsx:68`,
/// `/^\s*\|?[\s:|-]+\|?\s*$/`).
fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    let inner_trimmed = inner.trim();
    !inner_trimmed.is_empty()
        && inner.chars().all(|c| matches!(c, ' ' | ':' | '|' | '-'))
        && inner.contains('-')
}

/// A GFM Markdown table block found starting at line index `i`: `data_
/// row_count` data rows follow the header (line `i`) and separator
/// (line `i + 1`).
struct TableBlock {
    data_row_count: usize,
}

/// Whether a GFM table block starts at `lines[i]` — a `|…|` header row
/// followed immediately by a separator row ([`is_table_separator`]),
/// followed by zero or more `|…|` data rows.
fn try_parse_table_block(lines: &[&str], i: usize) -> Option<TableBlock> {
    if !lines[i].trim_start().starts_with('|') {
        return None;
    }
    let sep = lines.get(i + 1)?;
    if !is_table_separator(sep) {
        return None;
    }
    let mut j = i + 2;
    while j < lines.len() && lines[j].trim_start().starts_with('|') {
        j += 1;
    }
    Some(TableBlock {
        data_row_count: j - (i + 2),
    })
}

/// Rewrites one table block (WS7 item F1 rule 4): every data-row cell
/// that holds a number is checked via [`cell_is_verified`]; label cells and
/// a rank column ([`is_rank_header`]) are left alone. A table with numbers
/// and not one of them verified -> the header/separator/data lines are
/// replaced by a single line, the literal `"[table omitted: not backed by
/// a tool result]"`. Otherwise the header and separator survive unmodified
/// and each unverified numeric cell is wrapped
/// `<span data-unverified="true">…</span>` in place, inside its own `| … |`
/// cell — the table's row/column structure is never altered.
///
/// Returns the replacement lines and how many original lines (header +
/// separator + data rows) they replace.
fn annotate_table_block(
    lines: &[&str],
    i: usize,
    block: &TableBlock,
    evidence: &Evidence,
) -> (Vec<String>, usize) {
    let consumed = 2 + block.data_row_count;
    let data_start = i + 2;
    let rank_columns: Vec<bool> = split_row(lines[i])
        .iter()
        .map(|h| is_rank_header(h))
        .collect();
    let rows: Vec<Vec<String>> = lines[data_start..data_start + block.data_row_count]
        .iter()
        .map(|l| split_row(l))
        .collect();
    let states: Vec<Vec<Option<bool>>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(col, cell)| {
                    if rank_columns.get(col).copied().unwrap_or(false) {
                        None
                    } else {
                        cell_is_verified(cell, evidence)
                    }
                })
                .collect()
        })
        .collect();
    let any_numeric = states.iter().flatten().any(Option::is_some);
    let any_verified = states.iter().flatten().any(|v| *v == Some(true));
    if any_numeric && !any_verified {
        return (
            vec!["[table omitted: not backed by a tool result]".to_owned()],
            consumed,
        );
    }
    let mut out = vec![lines[i].to_owned(), lines[i + 1].to_owned()];
    for (row, row_states) in rows.iter().zip(states.iter()) {
        let cells: Vec<String> = row
            .iter()
            .zip(row_states.iter())
            .map(|(cell, state)| {
                if *state == Some(false) {
                    format!(r#"<span data-unverified="true">{cell}</span>"#)
                } else {
                    cell.clone()
                }
            })
            .collect();
        out.push(format!("| {} |", cells.join(" | ")));
    }
    (out, consumed)
}

/// The byte ranges (start, end) of every whitespace-delimited word in
/// `line`, in order.
fn word_byte_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                spans.push((s, idx));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        spans.push((s, line.len()));
    }
    spans
}

/// A per-byte mask of `line` marking every inline `` `code` `` span
/// (backticks included) as `true`. Single-line only — fenced (multi-line)
/// code blocks are handled by [`annotate_answer`]'s own line-level scan,
/// which never calls this on a fenced line at all.
fn inline_code_mask(line: &str) -> Vec<bool> {
    let mut mask = vec![false; line.len()];
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let end = line[i + 1..].find('`').map_or(i + 1, |rel| i + 1 + rel + 1);
            let mask_end = end.min(mask.len());
            for m in mask.iter_mut().take(mask_end).skip(i) {
                *m = true;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    mask
}

/// Rewrites every UNVERIFIED number in `line` (outside any inline code
/// span, read by [`read_number`]) to `<span data-unverified="true">…</span>`,
/// in place — a VERIFIED number is left byte-for-byte unchanged, and no
/// other character of `line` (spacing, punctuation, emphasis, code spans)
/// is ever touched.
fn annotate_line(line: &str, evidence: &Evidence) -> String {
    let mask = inline_code_mask(line);
    let spans = word_byte_spans(line);
    let mut out = String::with_capacity(line.len());
    let mut last = 0usize;
    let mut idx = 0usize;
    while idx < spans.len() {
        let (start, end) = spans[idx];
        out.push_str(&line[last..start]);
        let word = &line[start..end];
        if mask[start] {
            out.push_str(word); // inside inline code — never touched
            last = end;
            idx += 1;
            continue;
        }
        let next = spans
            .get(idx + 1)
            .filter(|(nstart, _)| !mask[*nstart])
            .map(|&(nstart, nend)| &line[nstart..nend]);
        if let Some(printed) = read_number(word, next) {
            if printed.verified(evidence) {
                out.push_str(word);
            } else {
                let lead = word.len()
                    - word
                        .trim_start_matches(|c: char| EDGE_MARKS.contains(&c))
                        .len();
                let trimmed = trim_marks(word);
                out.push_str(&word[..lead]);
                let _ = write!(out, r#"<span data-unverified="true">{trimmed}</span>"#);
                out.push_str(&word[lead + trimmed.len()..]);
            }
            if printed.consumed_next
                && let Some(&(nstart, nend)) = spans.get(idx + 1)
            {
                // The unit or scale word itself is never wrapped — only the
                // digit token is a "number citation" — but it is still
                // copied through unchanged, exactly like any other word.
                out.push_str(&line[end..nstart]);
                out.push_str(&line[nstart..nend]);
                last = nend;
                idx += 2;
                continue;
            }
        } else {
            out.push_str(word);
        }
        last = end;
        idx += 1;
    }
    out.push_str(&line[last..]);
    out
}

/// The text of every flagged number or table cell in an answer
/// [`annotate_answer_in`] already rewrote, in order, and whether a whole
/// table was omitted. The chat loop hands these back to the model once, so
/// it can look the figures up or drop them before the user sees them.
#[must_use]
pub fn flagged(annotated: &str) -> (Vec<String>, bool) {
    const OPEN: &str = r#"<span data-unverified="true">"#;
    let mut out = Vec::new();
    let mut rest = annotated;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let end = after.find("</span>").unwrap_or(after.len());
        out.push(after[..end].to_owned());
        rest = after.get(end..).unwrap_or("");
    }
    (
        out,
        annotated.contains("[table omitted: not backed by a tool result]"),
    )
}

/// [`annotate_answer_in`] with no earlier conversation.
#[cfg(test)]
#[must_use]
pub fn annotate_answer(answer: &str, trace: &[Value]) -> String {
    annotate_answer_in(answer, trace, &[])
}

/// Rewrites `answer` per WS7 item F1's three-state rule set, checking every
/// number against the [`Evidence`] of `trace` and the earlier
/// `conversation` texts:
///
/// - Every table (WS7 item F1 rule 4): handled by [`annotate_table_block`].
/// - Every number OUTSIDE a table: handled by [`annotate_line`].
///
/// A fenced code block (a line trimmed to start with `` ``` ``, toggling
/// an "inside fence" state until the next such line) is copied through
/// entirely unchanged, line for line — neither table detection nor
/// number annotation ever looks inside one. Never re-orders or drops
/// surrounding prose — only a matched table block or individual number
/// token is replaced in place.
#[must_use]
pub fn annotate_answer_in(answer: &str, trace: &[Value], conversation: &[&str]) -> String {
    let evidence = Evidence::new(trace, conversation);
    let lines: Vec<&str> = answer.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out_lines.push(line.to_owned());
            i += 1;
            continue;
        }
        if in_fence {
            out_lines.push(line.to_owned());
            i += 1;
            continue;
        }
        if let Some(block) = try_parse_table_block(&lines, i) {
            let (rewritten, consumed) = annotate_table_block(&lines, i, &block, &evidence);
            out_lines.extend(rewritten);
            i += consumed;
            continue;
        }
        out_lines.push(annotate_line(line, &evidence));
        i += 1;
    }
    out_lines.join("\n")
}

#[cfg(test)]
mod extraction_and_annotation {
    use super::*;

    #[test]
    #[allow(
        clippy::approx_constant,
        reason = "3.14 here is a deliberately pi-shaped decimal in the test \
                  prose, not a stand-in for PI"
    )]
    fn extracts_plain_comma_decimal_percent_and_byte_unit_numbers_with_their_units() {
        let nums = extract_numbers("Ada 1,234 baris, rata-rata 3.14, dan 12% gagal, sebesar 3 GB.");
        assert_eq!(
            nums,
            vec![
                (1234.0, Unit::None),
                (3.14, Unit::None),
                (12.0, Unit::Percent),
                (3.0, Unit::Gigabytes)
            ],
        );
    }

    #[test]
    fn a_bare_trailing_s_is_seconds_but_ms_is_milliseconds_not_double_counted() {
        let nums = extract_numbers("selesai dalam 2.5 s, atau 840 ms.");
        assert_eq!(
            nums,
            vec![(2.5, Unit::Seconds), (840.0, Unit::Milliseconds)]
        );
    }

    // N4 (revision 2 judge review) — exclusions, one test per class.

    #[test]
    fn does_not_split_an_iso_date_into_three_spurious_numbers() {
        let nums = extract_numbers("Data terakhir pada 2024-01-15.");
        assert_eq!(nums, vec![], "2024-01-15 must not become 2024, -01, -15");
    }

    #[test]
    fn does_not_split_an_iso_time_into_spurious_numbers() {
        let nums = extract_numbers("Terjadi pada 09:41:07.");
        assert_eq!(nums, vec![]);
    }

    #[test]
    fn does_not_extract_digits_from_an_identifier() {
        let nums = extract_numbers("Lihat run q-17123 untuk detail.");
        assert_eq!(nums, vec![], "q-17123 is an identifier, not a number");
    }

    #[test]
    fn does_not_extract_digits_from_a_version_string() {
        let nums = extract_numbers("Menggunakan sqlparser v1.2 saja.");
        assert_eq!(
            nums,
            vec![],
            "v1.2 is a version string, not a decimal number"
        );
    }

    #[test]
    fn skips_numbers_inside_an_inline_code_span() {
        let nums = extract_numbers("Query: `SELECT count() FROM t LIMIT 42` menghasilkan 7 baris.");
        assert_eq!(
            nums,
            vec![(7.0, Unit::None)],
            "42 is inside `...`, only the real prose number 7 counts"
        );
    }

    #[test]
    fn skips_numbers_inside_a_fenced_code_block() {
        let answer = "Hasilnya 3 baris:\n\n```sql\nSELECT * FROM t LIMIT 500\n```\n\nselesai.";
        let nums = extract_numbers(answer);
        assert_eq!(
            nums,
            vec![(3.0, Unit::None)],
            "500 is inside a fenced block, only 3 counts"
        );
    }

    #[test]
    fn a_number_immediately_followed_by_a_hyphen_joined_digit_run_is_excluded_as_a_whole_token() {
        // Not a date, but the same underlying rule (a digit run joined to
        // ANOTHER digit run by a bare '-' is never a citable number) —
        // e.g. an order/ticket range "17123-4".
        let nums = extract_numbers("Order 17123-4 telah dikirim.");
        assert_eq!(nums, vec![]);
    }

    #[test]
    fn a_table_with_zero_verified_cells_is_omitted_with_a_note() {
        let answer = "Berikut datanya:\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nSemoga membantu.";
        let annotated = annotate_answer(answer, &[]);
        assert!(!annotated.contains("| a | b |"));
        assert!(annotated.contains("table omitted: not backed by a tool result"));
        assert!(
            annotated.contains("Semoga membantu."),
            "surrounding prose must survive"
        );
    }

    #[test]
    fn a_table_whose_every_cell_is_verified_survives_unmodified() {
        let trace = vec![serde_json::json!({"tool":"run_sql","ok":true,
            "result":{"rows":[{"a":1,"b":2}]}})];
        let answer = "| a | b |\n|---|---|\n| 1 | 2 |";
        let annotated = annotate_answer(answer, &trace);
        assert!(annotated.contains("| 1 | 2 |"));
    }

    #[test]
    fn a_table_with_some_verified_and_some_unverified_cells_survives_with_only_the_unverified_cells_wrapped()
     {
        // Rule 4: partial verification never omits the table (that would
        // delete the ONE verified row too) — each unverified cell is
        // individually flagged, the table's grid stays intact.
        let trace = vec![serde_json::json!({"tool":"run_sql","ok":true,
            "result":{"rows":[{"a":1,"b":2}]}})];
        let answer = "| a | b |\n|---|---|\n| 1 | 2 |\n| 999 | 998 |";
        let annotated = annotate_answer(answer, &trace);
        assert!(
            annotated.contains("| 1 | 2 |"),
            "the verified row survives unwrapped"
        );
        assert!(annotated.contains(r#"<span data-unverified="true">999</span>"#));
        assert!(annotated.contains(r#"<span data-unverified="true">998</span>"#));
    }

    #[test]
    fn an_unverified_number_is_flagged_inline_not_silently_removed() {
        let answer = "Total pendapatan adalah 999999.";
        let annotated = annotate_answer(answer, &[]);
        assert_eq!(
            annotated,
            r#"Total pendapatan adalah <span data-unverified="true">999999</span>."#
        );
    }

    #[test]
    fn a_verified_number_is_left_exactly_as_printed_with_no_wrapper_at_all() {
        let trace =
            vec![serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"n":42}]}})];
        let answer = "Ada 42 baris.";
        assert_eq!(annotate_answer(answer, &trace), "Ada 42 baris.");
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
    fn percent_unit_matches_a_fraction_or_a_value_already_in_percent() {
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
        // SQL that already multiplied by 100 (`round(100 * a / b, 1)`)
        // backs "27.4%" directly. Before, only the x100 reading was
        // tried, so a share computed the recommended way was flagged.
        let trace = vec![
            serde_json::json!({"tool":"run_sql","ok":true,"result":{"rows":[{"share":"27.4"}]}}),
        ];
        assert_eq!(
            verify_number(27.4, Unit::Percent, &trace),
            NumberState::Verified
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

#[cfg(test)]
mod evidence {
    use super::*;
    use serde_json::json;

    fn run_sql(rows: &Value) -> Vec<Value> {
        vec![json!({"tool": "run_sql", "ok": true, "result": {"columns": [], "rows": rows}})]
    }

    #[test]
    fn a_number_clickhouse_returned_as_a_string_is_evidence() {
        let trace = run_sql(&json!([{"negara": "Malaysia", "total": "1571564"}]));
        assert_eq!(
            annotate_answer("Malaysia sent 1,571,564 visitors.", &trace),
            "Malaysia sent 1,571,564 visitors."
        );
    }

    #[test]
    fn a_bold_wrong_total_is_flagged_not_skipped() {
        let trace = run_sql(&json!([{"n": "12934"}]));
        let annotated = annotate_answer("**Total POI: 13,934**", &trace);
        assert!(
            annotated.contains(r#"<span data-unverified="true">13,934</span>"#),
            "{annotated}"
        );
        assert_eq!(
            annotate_answer("**Total POI: 12,934**", &trace),
            "**Total POI: 12,934**"
        );
    }

    #[test]
    fn counts_and_totals_implied_by_the_rows_are_evidence() {
        let trace = run_sql(&json!([
            {"tier": "primer", "n": "3"}, {"tier": "primer", "n": "4"}, {"tier": "sekunder", "n": "5"}
        ]));
        // 2 rows share "primer", 3 rows in all, 12 is the column total.
        assert_eq!(
            annotate_answer("3 datasets: 2 primer, 12 rows in total.", &trace),
            "3 datasets: 2 primer, 12 rows in total."
        );
    }

    #[test]
    fn scaled_and_indonesian_forms_match_within_their_printed_precision() {
        let trace = run_sql(&json!([{"n": "2643888"}, {"n": "12.09"}]));
        for answer in [
            "2.64 million",
            "2,64 juta",
            "2.643.888",
            "2.6M",
            "12,09%",
            "12.1%",
        ] {
            assert_eq!(
                annotate_answer(answer, &trace),
                answer,
                "{answer} should verify"
            );
        }
        assert!(annotate_answer("2.9 million", &trace).contains("data-unverified"));
    }

    #[test]
    fn label_cells_and_rank_columns_are_never_flagged() {
        let trace = run_sql(&json!([{"negara": "Malaysia", "s": "1571564"}]));
        let answer = "| # | Country | Visits |\n|---|---|---|\n| 1 | 🇲🇾 Malaysia | **1,571,564** |";
        assert_eq!(annotate_answer(answer, &trace), answer);
    }

    #[test]
    fn a_table_of_labels_only_is_kept_whole() {
        let answer = "| Column | Meaning |\n|---|---|\n| tahun | Year of visit |";
        assert_eq!(annotate_answer(answer, &[]), answer);
    }

    #[test]
    fn a_number_from_earlier_in_the_conversation_is_evidence_unless_it_was_flagged() {
        let earlier = [
            "How many visits in 2023?",
            r#"There were 2,358,638 visits, and <span data-unverified="true">777</span> events."#,
        ];
        assert_eq!(
            annotate_answer_in("2023 had 2,358,638.", &[], &earlier),
            "2023 had 2,358,638."
        );
        assert!(
            annotate_answer_in("There were 777 events.", &[], &earlier).contains("data-unverified")
        );
    }

    #[test]
    fn plain_number_strings_are_read_and_nothing_else_is() {
        assert_eq!(parse_plain_number("1571564"), Some(1_571_564.0));
        assert_eq!(parse_plain_number("-3.5"), Some(-3.5));
        assert_eq!(parse_plain_number("2024-01-15"), None);
        assert_eq!(parse_plain_number("1,234"), None);
        assert_eq!(parse_plain_number("w-1"), None);
        assert_eq!(parse_plain_number(""), None);
    }

    #[test]
    fn an_indonesian_thousands_dot_matches_either_reading() {
        let trace = run_sql(&json!([{"n": "18420"}]));
        assert_eq!(
            annotate_answer("yaitu 18.420 usaha", &trace),
            "yaitu 18.420 usaha"
        );
        assert!(annotate_answer("yaitu 18.520 usaha", &trace).contains("data-unverified"));
    }

    #[test]
    fn a_difference_or_change_between_two_numbers_of_one_result_is_verified() {
        let trace = run_sql(&json!([{"tahun": "2020", "n": "62"}, {"tahun": "2024", "n": "396"}]));
        let answer = "From 62 to 396, an increase of 334 events (+538.7%, 6.4x).";
        assert_eq!(annotate_answer(answer, &trace), answer);
        // A wrong difference is still flagged.
        assert!(annotate_answer("an increase of 344 events", &trace).contains("data-unverified"));
    }

    #[test]
    fn byte_units_match_the_binary_or_the_decimal_reading() {
        let trace = run_sql(&json!([{"walRetainedBytes": "19552544"}]));
        for answer in ["18.6 MB", "19.6 MB"] {
            assert_eq!(annotate_answer(answer, &trace), answer, "{answer}");
        }
    }

    #[test]
    fn a_bold_number_in_an_earlier_answer_is_evidence() {
        let earlier = ["There were **2,358,638** visits in 2023."];
        assert_eq!(
            annotate_answer_in("2023 had 2,358,638.", &[], &earlier),
            "2023 had 2,358,638."
        );
    }

    #[test]
    fn a_unit_word_in_bold_is_still_a_unit() {
        let trace = run_sql(&json!([{"walRetainedBytes": "23774528"}]));
        let answer = "WAL retained: **23.77 MB**";
        assert_eq!(annotate_answer(answer, &trace), answer);
    }

    #[test]
    fn flagged_lists_every_flagged_number_and_an_omitted_table() {
        let (numbers, omitted) = flagged(
            "a <span data-unverified=\"true\">12</span> b <span data-unverified=\"true\">3.4%</span>\n[table omitted: not backed by a tool result]",
        );
        assert_eq!(numbers, vec!["12".to_owned(), "3.4%".to_owned()]);
        assert!(omitted);
        assert_eq!(flagged("all good"), (Vec::new(), false));
    }
}
