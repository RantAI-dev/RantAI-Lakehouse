//! `{{name}}` parameters in SQL (`QS-5` part 1, plan decisions D1 to D3): find
//! the placeholders an author wrote, check the value each one gets, and
//! print the checked value into the statement as a literal.
//!
//! # Why the printer is the only route from a value into SQL
//!
//! A value arrives as JSON from a person or from the assistant. Nothing of
//! it is copied into the statement as text. [`substitute`] first turns each
//! value into a `lakehouse_bi::params::ParamValue` with `check_value`, which
//! rejects anything that is not a finite number, a real `YYYY-MM-DD` date,
//! text of at most 200 characters without a control character, or (for a
//! dropdown or multi-select) an entry of the definition's own list, and
//! which hands back the stored entry, not the request's text. Then one
//! printer per engine writes the value again from the parsed form:
//! a number from its `f64`, a date from its parts, text as a string literal
//! escaped by the engine's own rule. So the only characters that can come
//! from a caller are inside a quoted text literal, and the escaping closes
//! that literal on the same character the engine closes it.
//!
//! The scanner follows the same rule. `ClickHouse` reads a backslash in a
//! string as an escape and `Trino` does not, so [`Engine`] decides how a
//! quoted run ends, in the scanner and in the printer alike. If the two
//! disagreed about where a literal ends, a value could leave its literal.
//!
//! # What is scanned, and when
//!
//! The scanner reads the text the author wrote, once per request, and never
//! the printer's output: a text value that itself reads `{{x}}` is printed
//! inside a quoted literal and stays there. The statement is rebuilt from
//! slices of the original text, so a statement without a placeholder comes
//! back byte for byte.
//!
//! A placeholder in a comment is ignored. A well-formed placeholder inside a
//! quoted string or identifier is refused, because writing `'{{region}}'`
//! looks like it would work and would not. A `{{` outside both that is not a
//! well-formed placeholder is refused.
//!
//! # What a refusal says
//!
//! [`ParamError`]'s `Display` is the sentence for the person. It names a
//! parameter only after the name passed the `BI-11` name rule, and it never
//! repeats a value.

use std::collections::BTreeMap;
use std::fmt;

use lakehouse_bi::formula::compile::print_number;
use lakehouse_bi::params::{
    ListEntry, MAX_LIST_VALUES, MAX_NAME_LEN, ParamDef, ParamType, ParamValue, check_value,
    valid_name, validate_def,
};
use lakehouse_core::ident::SqlLiteral;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::sql_guard::{skip_block_comment, skip_quoted, skip_to_line_end};
pub use crate::sql_rewrite::Engine;

/// Most parameters one query takes (feature decision 6).
pub const MAX_QUERY_PARAMS: usize = 20;

/// The type of a query parameter (feature decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QueryParamType {
    /// Text of at most 200 characters.
    Text,
    /// A finite number.
    Number,
    /// A calendar day, `YYYY-MM-DD`.
    Date,
    /// Two calendar days, written `{{name.start}}` and `{{name.end}}`.
    DateRange,
    /// One entry of a fixed list.
    Dropdown,
    /// Several entries of a fixed list.
    MultiSelect,
}

/// A saved query's definition of one parameter (feature decision 10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryParamDef {
    /// The name written between the braces.
    pub name: String,
    /// Its type.
    #[serde(rename = "type")]
    pub ty: QueryParamType,
    /// The value used when a run gives none. `null` means no default.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub default: Value,
    /// The choices of a dropdown or multi-select.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ListEntry>,
}

/// Why a statement or a definition was refused. `Display` is the sentence
/// for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamError {
    /// A `{{` that is not a parameter, or a bare range.
    NotAParameter,
    /// A well-formed placeholder inside quotes. Holds the text between the
    /// braces, trimmed.
    InsideQuotes(String),
    /// No value and no default.
    Missing(String),
    /// A multi-select with nothing picked.
    EmptySelection(String),
    /// `.start` or `.end` on a parameter that is not a date range.
    RangeOnly(String),
    /// `check_value`'s sentence, or the shape of a range or list.
    Invalid(String),
    /// More than [`MAX_QUERY_PARAMS`] parameters.
    TooManyParams,
    /// More than 200 values in a multi-select.
    TooManyValues(String),
    /// More than 200 options in a dropdown or multi-select.
    TooManyOptions(String),
    /// A definition whose name fails the name rule.
    BadName,
    /// A definition that does not hold together; the reason is a fixed
    /// sentence.
    BadDefinition(String, String),
    /// Two definitions with one name.
    Duplicate(String),
    /// A definition whose name the SQL does not use.
    NotInSql(String),
    /// A placeholder with no definition.
    NoDefinition(String),
}

/// `{{name}}`, the way a person writes a placeholder.
fn braced(name: &str) -> String {
    ["{{", name, "}}"].concat()
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAParameter => f.write_str("This query has a {{ that is not a parameter."),
            Self::InsideQuotes(written) => {
                let p = braced(written);
                write!(
                    f,
                    "{p} is inside quotes: write it without them, for example = {p}."
                )
            }
            Self::Missing(name) => write!(f, "Fill in {} to run this query.", braced(name)),
            Self::EmptySelection(name) => {
                write!(f, "Pick at least one value for {}.", braced(name))
            }
            Self::RangeOnly(name) => write!(
                f,
                "{} is for a date range, and {name} is not one.",
                braced(&[name, ".start"].concat())
            ),
            Self::Invalid(sentence) => f.write_str(sentence),
            Self::TooManyParams => {
                write!(f, "A query takes at most {MAX_QUERY_PARAMS} parameters.")
            }
            Self::TooManyValues(name) => {
                write!(
                    f,
                    "{} takes at most {MAX_LIST_VALUES} values.",
                    braced(name)
                )
            }
            Self::TooManyOptions(name) => write!(
                f,
                "{} has more than {MAX_LIST_VALUES} options; a dropdown or multi-select takes at most {MAX_LIST_VALUES}.",
                braced(name)
            ),
            Self::BadName => write!(
                f,
                "A parameter name is 1 to {MAX_NAME_LEN} letters, digits or _, and does not start with a digit."
            ),
            Self::BadDefinition(name, reason) => {
                write!(
                    f,
                    "The definition of {} is not valid: {reason}",
                    braced(name)
                )
            }
            Self::Duplicate(name) => write!(f, "Two parameters are named {}.", braced(name)),
            Self::NotInSql(name) => write!(
                f,
                "{} has a definition but the SQL does not use it.",
                braced(name)
            ),
            Self::NoDefinition(name) => write!(
                f,
                "{} is used in the SQL but has no definition.",
                braced(name)
            ),
        }
    }
}

impl std::error::Error for ParamError {}

/// Replace every placeholder in `sql` with the literal for its checked
/// value.
///
/// `parameters` maps a name to the value the run gives it. `definitions` are
/// the saved query's (or the request's); a placeholder with none is read as
/// text, and a `.start` or `.end` one as a date range. A value for a name
/// the SQL does not use is ignored.
///
/// # Errors
///
/// [`ParamError`] when the statement has a `{{` that is not a parameter or a
/// placeholder inside quotes, when a value is missing or does not fit its
/// type, or when a limit is passed.
pub fn substitute(
    engine: Engine,
    sql: &str,
    parameters: &Map<String, Value>,
    definitions: &[QueryParamDef],
) -> Result<String, ParamError> {
    if parameters.len() > MAX_QUERY_PARAMS || definitions.len() > MAX_QUERY_PARAMS {
        return Err(ParamError::TooManyParams);
    }
    let found = scan(engine, sql)?;
    if found.is_empty() {
        return Ok(sql.to_owned());
    }
    let names = distinct_names(&found);
    if names.len() > MAX_QUERY_PARAMS {
        return Err(ParamError::TooManyParams);
    }

    // Each name is checked once: its definition, then the way each
    // placeholder uses it, then its value.
    let mut resolved: BTreeMap<&str, Checked> = BTreeMap::new();
    for name in &names {
        let def = definitions
            .iter()
            .find(|d| d.name == *name)
            .cloned()
            .unwrap_or_else(|| implicit_def(name, &found));
        let list = check_definition(&def)?;
        for f in found.iter().filter(|f| f.name == *name) {
            check_part(&def, f.part)?;
        }
        let value = resolve(&def, list.as_ref(), parameters.get(*name))?;
        resolved.insert(name, value);
    }

    let mut out = String::with_capacity(sql.len());
    let mut at = 0;
    for f in &found {
        out.push_str(&sql[at..f.start]);
        out.push_str(&print_checked(engine, &resolved[f.name.as_str()], f.part)?);
        at = f.end;
    }
    out.push_str(&sql[at..]);
    Ok(out)
}

/// Check a query's definitions against its SQL before it is saved.
///
/// # Errors
///
/// [`ParamError`] when a definition is invalid, two share a name, a
/// definition is not used by the SQL, a placeholder has no definition, or
/// the SQL itself is refused by the scanner.
pub fn validate_definitions(
    engine: Engine,
    sql: &str,
    definitions: &[QueryParamDef],
) -> Result<(), ParamError> {
    if definitions.len() > MAX_QUERY_PARAMS {
        return Err(ParamError::TooManyParams);
    }
    let found = scan(engine, sql)?;
    let names = distinct_names(&found);
    if names.len() > MAX_QUERY_PARAMS {
        return Err(ParamError::TooManyParams);
    }
    for (i, def) in definitions.iter().enumerate() {
        check_definition(def)?;
        if definitions[..i].iter().any(|d| d.name == def.name) {
            return Err(ParamError::Duplicate(def.name.clone()));
        }
        if !names.contains(&def.name.as_str()) {
            return Err(ParamError::NotInSql(def.name.clone()));
        }
    }
    for name in &names {
        let Some(def) = definitions.iter().find(|d| d.name == *name) else {
            return Err(ParamError::NoDefinition((*name).to_owned()));
        };
        for f in found.iter().filter(|f| f.name == *name) {
            check_part(def, f.part)?;
        }
    }
    Ok(())
}

// ── the scanner ─────────────────────────────────────────────────────────

/// Which part of a parameter a placeholder names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// `{{name}}`.
    Whole,
    /// `{{name.start}}`.
    Start,
    /// `{{name.end}}`.
    End,
}

/// One placeholder: its name and the byte range of `{{ … }}` in the SQL.
#[derive(Debug)]
struct Found {
    name: String,
    part: Part,
    start: usize,
    end: usize,
}

/// Whether a backslash escapes the next character inside a quoted run.
fn backslash_escapes(engine: Engine) -> bool {
    match engine {
        Engine::ClickHouse => true,
        Engine::Trino => false,
    }
}

/// The placeholders the author wrote, in order. Comments are skipped, a
/// well-formed placeholder inside quotes is refused, and a `{{` that is not
/// a placeholder outside both is refused.
fn scan(engine: Engine, sql: &str) -> Result<Vec<Found>, ParamError> {
    let chars: Vec<char> = sql.chars().collect();
    // Byte offset of every char, and of the end, so slices of `sql` can be
    // cut where a placeholder starts and ends.
    let mut offsets: Vec<usize> = sql.char_indices().map(|(at, _)| at).collect();
    offsets.push(sql.len());

    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => i = skip_to_line_end(&chars, i),
            '#' => i = skip_to_line_end(&chars, i),
            '/' if next == Some('*') => i = skip_block_comment(&chars, i),
            '\'' | '"' | '`' => {
                let end = skip_quoted(&chars, i, c, backslash_escapes(engine));
                if let Some(written) = placeholder_within(&chars, i, end) {
                    return Err(ParamError::InsideQuotes(written));
                }
                i = end;
            }
            '{' if next == Some('{') => {
                let Some((name, part, end)) = parse_placeholder(&chars, i) else {
                    return Err(ParamError::NotAParameter);
                };
                found.push(Found {
                    name,
                    part,
                    start: offsets[i],
                    end: offsets[end],
                });
                i = end;
            }
            _ => i += 1,
        }
    }
    Ok(found)
}

/// The text of the first well-formed placeholder in `chars[from..to]`.
fn placeholder_within(chars: &[char], from: usize, to: usize) -> Option<String> {
    (from..to.saturating_sub(1))
        .filter(|&at| chars[at] == '{' && chars[at + 1] == '{')
        .find_map(|at| {
            parse_placeholder(chars, at).map(|(name, part, _)| match part {
                Part::Whole => name,
                Part::Start => [name.as_str(), ".start"].concat(),
                Part::End => [name.as_str(), ".end"].concat(),
            })
        })
}

/// A well-formed placeholder at `at` (which holds `{{`): optional
/// whitespace, a name, optionally `.start` or `.end`, optional whitespace,
/// `}}`. Returns the name, the part and the index just past the closing
/// braces.
fn parse_placeholder(chars: &[char], at: usize) -> Option<(String, Part, usize)> {
    let word = |from: usize| -> usize {
        (from..chars.len())
            .find(|&k| !(chars[k].is_ascii_alphanumeric() || chars[k] == '_'))
            .unwrap_or(chars.len())
    };
    let blanks = |from: usize| -> usize {
        (from..chars.len())
            .find(|&k| !chars[k].is_whitespace())
            .unwrap_or(chars.len())
    };
    let start = blanks(at + 2);
    let name_end = word(start);
    let name: String = chars[start..name_end].iter().collect();
    if !valid_name(&name) {
        return None;
    }
    let (part, after) = if chars.get(name_end) == Some(&'.') {
        let part_end = word(name_end + 1);
        let part: String = chars[name_end + 1..part_end].iter().collect();
        match part.as_str() {
            "start" => (Part::Start, part_end),
            "end" => (Part::End, part_end),
            _ => return None,
        }
    } else {
        (Part::Whole, name_end)
    };
    let close = blanks(after);
    (chars.get(close) == Some(&'}') && chars.get(close + 1) == Some(&'}')).then_some((
        name,
        part,
        close + 2,
    ))
}

/// The names used, once each, in order of first use.
fn distinct_names(found: &[Found]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for f in found {
        if !names.contains(&f.name.as_str()) {
            names.push(&f.name);
        }
    }
    names
}

// ── definitions and values ──────────────────────────────────────────────

/// A value that passed its type's check, ready to print.
#[derive(Debug)]
enum Checked {
    One(ParamValue),
    Range(ParamValue, ParamValue),
    Many(Vec<ParamValue>),
}

/// The definition a placeholder without one gets: text, or a date range
/// when it is written `.start` or `.end`.
fn implicit_def(name: &str, found: &[Found]) -> QueryParamDef {
    let ranged = found
        .iter()
        .any(|f| f.name == name && f.part != Part::Whole);
    QueryParamDef {
        name: name.to_owned(),
        ty: if ranged {
            QueryParamType::DateRange
        } else {
            QueryParamType::Text
        },
        default: Value::Null,
        options: Vec::new(),
    }
}

/// `BI-11`'s definition for a plain text, number or date check.
fn scalar_def(name: &str, ty: ParamType) -> ParamDef {
    ParamDef {
        id: String::new(),
        name: name.to_owned(),
        label: name.to_owned(),
        ty,
        default: Value::Null,
        min: None,
        max: None,
        max_length: None,
        values: None,
        created_by: String::new(),
        updated_at: None,
    }
}

/// Check a definition, and for a dropdown or multi-select return the
/// `BI-11` list definition that checks its values.
fn check_definition(def: &QueryParamDef) -> Result<Option<ParamDef>, ParamError> {
    if !valid_name(&def.name) {
        return Err(ParamError::BadName);
    }
    let bad = |reason: &str| ParamError::BadDefinition(def.name.clone(), reason.to_owned());
    let list = match def.ty {
        QueryParamType::Dropdown | QueryParamType::MultiSelect => {
            if def.options.len() > MAX_LIST_VALUES {
                return Err(ParamError::TooManyOptions(def.name.clone()));
            }
            // `validate_def` wants a default that is one of the values; the
            // real default (a list, for a multi-select) is checked below.
            let mut list = scalar_def(&def.name, ParamType::List);
            list.values = Some(def.options.clone());
            list.default = match (def.ty, &def.default) {
                (QueryParamType::Dropdown, d) if !d.is_null() => d.clone(),
                _ => def
                    .options
                    .first()
                    .map_or(Value::Null, |first| first.value.clone()),
            };
            let checked = validate_def(&list).map_err(|reason| {
                ParamError::BadDefinition(def.name.clone(), capitalise(&reason))
            })?;
            Some(checked)
        }
        _ => {
            if !def.options.is_empty() {
                return Err(bad("only a dropdown or a multi-select has options."));
            }
            None
        }
    };
    if !def.default.is_null() {
        check_raw(def, list.as_ref(), &def.default)?;
    }
    Ok(list)
}

/// `reason` with its first letter upper-cased, so it reads as a sentence
/// after "is not valid: ".
fn capitalise(reason: &str) -> String {
    let mut chars = reason.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Whether `part` fits the definition's type.
fn check_part(def: &QueryParamDef, part: Part) -> Result<(), ParamError> {
    match (def.ty == QueryParamType::DateRange, part) {
        (true, Part::Whole) => Err(ParamError::NotAParameter),
        (false, Part::Start | Part::End) => Err(ParamError::RangeOnly(def.name.clone())),
        _ => Ok(()),
    }
}

/// The value a run gives `def`: the request's, else the default.
fn resolve(
    def: &QueryParamDef,
    list: Option<&ParamDef>,
    supplied: Option<&Value>,
) -> Result<Checked, ParamError> {
    let raw = supplied
        .filter(|v| !v.is_null())
        .or_else(|| Some(&def.default).filter(|v| !v.is_null()))
        .ok_or_else(|| ParamError::Missing(def.name.clone()))?;
    check_raw(def, list, raw)
}

/// Check `raw` against `def`'s type.
fn check_raw(
    def: &QueryParamDef,
    list: Option<&ParamDef>,
    raw: &Value,
) -> Result<Checked, ParamError> {
    let name = &def.name;
    let invalid = |m: String| ParamError::Invalid(m);
    let one = |ty: ParamType, raw: &Value| check_value(&scalar_def(name, ty), raw).map_err(invalid);
    match def.ty {
        QueryParamType::Text => one(ParamType::Text, raw).map(Checked::One),
        QueryParamType::Number => one(ParamType::Number, raw).map(Checked::One),
        QueryParamType::Date => one(ParamType::Date, raw).map(Checked::One),
        QueryParamType::DateRange => {
            let range = raw
                .as_object()
                .filter(|o| o.len() == 2)
                .and_then(|o| Some((o.get("start")?, o.get("end")?)))
                .ok_or_else(|| {
                    ParamError::Invalid(format!(
                        "parameter '{name}' must be a date range with a start and an end."
                    ))
                })?;
            Ok(Checked::Range(
                one(ParamType::Date, range.0)?,
                one(ParamType::Date, range.1)?,
            ))
        }
        QueryParamType::Dropdown => {
            let list = list.ok_or(ParamError::BadName)?;
            check_value(list, raw).map(Checked::One).map_err(invalid)
        }
        QueryParamType::MultiSelect => {
            let list = list.ok_or(ParamError::BadName)?;
            let picked = raw.as_array().ok_or_else(|| {
                ParamError::Invalid(format!("parameter '{name}' must be a list."))
            })?;
            if picked.is_empty() {
                return Err(ParamError::EmptySelection(name.clone()));
            }
            if picked.len() > MAX_LIST_VALUES {
                return Err(ParamError::TooManyValues(name.clone()));
            }
            picked
                .iter()
                .map(|v| check_value(list, v).map_err(ParamError::Invalid))
                .collect::<Result<_, _>>()
                .map(Checked::Many)
        }
    }
}

// ── the printer ─────────────────────────────────────────────────────────

/// The literal for a checked value, from its parsed form: the only route
/// from a value into the statement.
fn print_checked(engine: Engine, value: &Checked, part: Part) -> Result<String, ParamError> {
    Ok(match (value, part) {
        (Checked::One(v), Part::Whole) => print_value(engine, v),
        (Checked::Many(vs), Part::Whole) => vs
            .iter()
            .map(|v| print_value(engine, v))
            .collect::<Vec<_>>()
            .join(", "),
        (Checked::Range(start, _), Part::Start) => print_value(engine, start),
        (Checked::Range(_, end), Part::End) => print_value(engine, end),
        _ => return Err(ParamError::NotAParameter),
    })
}

fn print_value(engine: Engine, value: &ParamValue) -> String {
    match value {
        ParamValue::Number(n) => {
            let plain = print_number(*n);
            // `-{{n}}` with -5 would read `--5`, a comment. Parentheses
            // keep a negative number one token in any position.
            if plain.starts_with('-') {
                format!("({plain})")
            } else {
                plain
            }
        }
        ParamValue::Text(t) => match engine {
            Engine::ClickHouse => SqlLiteral::from(t.as_str()).to_string(),
            // `Trino` has no backslash escape: only the quote is doubled.
            Engine::Trino => format!("'{}'", t.replace('\'', "''")),
        },
        ParamValue::Date(d) => match engine {
            Engine::ClickHouse => format!("toDate32('{d}')"),
            Engine::Trino => format!("DATE '{d}'"),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    const BOTH: [Engine; 2] = [Engine::ClickHouse, Engine::Trino];

    fn map(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            other => panic!("a test passed {other} where an object was meant"),
        }
    }

    fn defs(v: Value) -> Vec<QueryParamDef> {
        serde_json::from_value(v).unwrap()
    }

    fn run(engine: Engine, sql: &str, params: Value, definitions: Value) -> Result<String, String> {
        substitute(engine, sql, &map(params), &defs(definitions)).map_err(|e| e.to_string())
    }

    fn run_ok(engine: Engine, sql: &str, params: Value, definitions: Value) -> String {
        run(engine, sql, params, definitions).unwrap()
    }

    /// Reads the one string literal at the start of `printed` by the
    /// engine's own rule, independently of the module under test. Returns
    /// the text it stands for and the bytes it used.
    fn read_literal(engine: Engine, printed: &str) -> (String, usize) {
        let mut chars = printed.char_indices().peekable();
        assert_eq!(chars.next().map(|(_, c)| c), Some('\''), "{printed}");
        let mut text = String::new();
        while let Some((at, c)) = chars.next() {
            match c {
                '\\' if engine == Engine::ClickHouse => {
                    let (_, escaped) = chars.next().expect("a dangling escape");
                    assert!(
                        matches!(escaped, '\\' | '\''),
                        "unexpected escape in {printed}"
                    );
                    text.push(escaped);
                }
                '\'' => {
                    if chars.peek().map(|(_, n)| *n) == Some('\'') {
                        chars.next();
                        text.push('\'');
                    } else {
                        return (text, at + 1);
                    }
                }
                other => text.push(other),
            }
        }
        panic!("the literal is not closed: {printed}");
    }

    const NASTY: [&str; 10] = [
        "it's",
        "a\\b",
        "ends with a backslash\\",
        "'; DROP TABLE x; --",
        "x' OR '1'='1",
        "''",
        "\\'",
        "\\' OR 1=1 --",
        "'\\'",
        "{{x}}",
    ];

    // ── printing ────────────────────────────────────────────────────────

    #[test]
    fn a_text_value_prints_as_one_quoted_literal_on_both_engines() {
        for engine in BOTH {
            let got = run_ok(
                engine,
                "SELECT * FROM t WHERE r = {{region}}",
                json!({"region": "EU"}),
                json!([]),
            );
            assert_eq!(got, "SELECT * FROM t WHERE r = 'EU'", "{engine:?}");
        }
    }

    #[test]
    fn a_quote_a_backslash_and_an_injection_attempt_stay_inside_one_literal() {
        for engine in BOTH {
            for value in NASTY {
                let printed = run_ok(engine, "{{v}}", json!({ "v": value }), json!([]));
                let (decoded, used) = read_literal(engine, &printed);
                assert_eq!(used, printed.len(), "{engine:?} {value}: {printed}");
                assert_eq!(decoded, value, "{engine:?}: {printed}");
            }
        }
    }

    #[test]
    fn a_drop_table_value_is_one_literal_inside_the_statement() {
        for engine in BOTH {
            let got = run_ok(
                engine,
                "SELECT * FROM t WHERE n = {{n}} AND k = 1",
                json!({"n": "'; DROP TABLE x; --"}),
                json!([]),
            );
            assert_eq!(
                got, "SELECT * FROM t WHERE n = '''; DROP TABLE x; --' AND k = 1",
                "{engine:?}"
            );
        }
    }

    #[test]
    fn a_trino_backslash_stays_one_character_and_a_clickhouse_one_is_doubled() {
        let value = json!({"v": "a\\b"});
        assert_eq!(
            run_ok(Engine::ClickHouse, "{{v}}", value.clone(), json!([])),
            "'a\\\\b'"
        );
        assert_eq!(run_ok(Engine::Trino, "{{v}}", value, json!([])), "'a\\b'");
    }

    #[test]
    fn a_control_character_in_a_text_value_is_refused_without_repeating_it() {
        for engine in BOTH {
            for value in ["a\nb", "a\tb", "a\u{0}b", "a\rb"] {
                let err = run(engine, "{{v}}", json!({ "v": value }), json!([])).unwrap_err();
                assert!(err.contains("'v'"), "{err}");
                assert!(!err.contains(value), "{err:?}");
            }
        }
    }

    #[test]
    fn a_number_prints_from_its_value() {
        let d = json!([{"name": "n", "type": "number"}]);
        for engine in BOTH {
            assert_eq!(run_ok(engine, "{{n}}", json!({"n": 42}), d.clone()), "42");
            assert_eq!(run_ok(engine, "{{n}}", json!({"n": 1.5}), d.clone()), "1.5");
            // A number given as text is parsed and printed again.
            assert_eq!(run_ok(engine, "{{n}}", json!({"n": " 7 "}), d.clone()), "7");
        }
    }

    #[test]
    fn a_number_for_a_parameter_without_a_definition_is_refused_as_text() {
        let err = run(Engine::ClickHouse, "{{n}}", json!({"n": 42}), json!([])).unwrap_err();
        assert_eq!(err, "parameter 'n' must be text.");
    }

    #[test]
    fn a_negative_number_is_wrapped_so_it_cannot_start_a_comment() {
        for engine in BOTH {
            let got = run_ok(
                engine,
                "SELECT 1 -{{n}} FROM t",
                json!({"n": -5}),
                json!([{"name": "n", "type": "number"}]),
            );
            assert_eq!(got, "SELECT 1 -(-5) FROM t", "{engine:?}");
            assert!(!got.contains("--"));
        }
    }

    #[test]
    fn a_text_that_is_not_a_number_is_refused_for_a_number_parameter() {
        for engine in BOTH {
            let err = run(
                engine,
                "{{n}}",
                json!({"n": "12abc"}),
                json!([{"name": "n", "type": "number"}]),
            )
            .unwrap_err();
            assert_eq!(err, "parameter 'n' must be a number.");
        }
    }

    #[test]
    fn a_date_prints_as_a_date_literal_for_each_engine() {
        let d = json!([{"name": "d", "type": "date"}]);
        let v = json!({"d": "2024-02-29"});
        assert_eq!(
            run_ok(Engine::ClickHouse, "x >= {{d}}", v.clone(), d.clone()),
            "x >= toDate32('2024-02-29')"
        );
        assert_eq!(
            run_ok(Engine::Trino, "x >= {{d}}", v, d),
            "x >= DATE '2024-02-29'"
        );
    }

    #[test]
    fn a_date_that_is_not_a_real_day_is_refused() {
        for engine in BOTH {
            for bad in [
                "2023-02-29",
                "2024-13-01",
                "1800-01-01",
                "24-01-01",
                "2024-01-01'",
            ] {
                let err = run(
                    engine,
                    "{{d}}",
                    json!({ "d": bad }),
                    json!([{"name": "d", "type": "date"}]),
                )
                .unwrap_err();
                assert_eq!(
                    err, "parameter 'd' must be a date written YYYY-MM-DD.",
                    "{bad}"
                );
            }
        }
    }

    #[test]
    fn a_date_range_prints_start_and_end_as_two_dates() {
        let d = json!([{"name": "r", "type": "dateRange"}]);
        let v = json!({"r": {"start": "2024-01-01", "end": "2024-01-31"}});
        let sql = "d BETWEEN {{r.start}} AND {{ r.end }}";
        assert_eq!(
            run_ok(Engine::ClickHouse, sql, v.clone(), d.clone()),
            "d BETWEEN toDate32('2024-01-01') AND toDate32('2024-01-31')"
        );
        assert_eq!(
            run_ok(Engine::Trino, sql, v, d),
            "d BETWEEN DATE '2024-01-01' AND DATE '2024-01-31'"
        );
    }

    #[test]
    fn a_range_without_a_definition_is_read_from_its_start_and_end() {
        let got = run_ok(
            Engine::Trino,
            "d BETWEEN {{r.start}} AND {{r.end}}",
            json!({"r": {"start": "2024-01-01", "end": "2024-01-31"}}),
            json!([]),
        );
        assert_eq!(got, "d BETWEEN DATE '2024-01-01' AND DATE '2024-01-31'");
    }

    #[test]
    fn a_range_needs_a_real_start_and_end() {
        for bad in [
            json!("2024-01-01"),
            json!({"start": "2024-01-01"}),
            json!({"start": "2024-01-01", "end": "nope"}),
            json!({"start": "2024-01-01", "end": "2024-01-02", "extra": 1}),
        ] {
            let err = run(
                Engine::ClickHouse,
                "{{r.start}} {{r.end}}",
                json!({ "r": bad }),
                json!([{"name": "r", "type": "dateRange"}]),
            )
            .unwrap_err();
            assert!(err.contains("'r'"), "{err}");
        }
    }

    #[test]
    fn a_bare_range_name_is_the_malformed_placeholder_sentence() {
        let err = run(
            Engine::ClickHouse,
            "d = {{r}}",
            json!({"r": {"start": "2024-01-01", "end": "2024-01-31"}}),
            json!([{"name": "r", "type": "dateRange"}]),
        )
        .unwrap_err();
        assert_eq!(err, "This query has a {{ that is not a parameter.");
    }

    #[test]
    fn start_and_end_on_a_parameter_that_is_not_a_range_are_refused() {
        let err = run(
            Engine::ClickHouse,
            "d = {{d.start}}",
            json!({"d": "2024-01-01"}),
            json!([{"name": "d", "type": "date"}]),
        )
        .unwrap_err();
        assert_eq!(err, "{{d.start}} is for a date range, and d is not one.");
    }

    #[test]
    fn a_multi_select_prints_a_comma_separated_list_of_literals() {
        let text = json!([{"name": "ids", "type": "multiSelect",
            "options": [{"value": "a"}, {"value": "it's"}, {"value": "c"}]}]);
        for engine in BOTH {
            let got = run_ok(
                engine,
                "x IN ({{ids}})",
                json!({"ids": ["a", "it's"]}),
                text.clone(),
            );
            assert_eq!(got, "x IN ('a', 'it''s')", "{engine:?}");
        }
        let numbers = json!([{"name": "ids", "type": "multiSelect",
            "options": [{"value": 1}, {"value": 2.5}, {"value": 3}]}]);
        for engine in BOTH {
            let got = run_ok(
                engine,
                "x IN ({{ids}})",
                json!({"ids": [3, "2.50"]}),
                numbers.clone(),
            );
            assert_eq!(got, "x IN (3, 2.5)", "{engine:?}");
        }
    }

    #[test]
    fn an_empty_multi_select_is_refused() {
        let d = json!([{"name": "ids", "type": "multiSelect", "options": [{"value": "a"}]}]);
        for engine in BOTH {
            let err = run(engine, "x IN ({{ids}})", json!({"ids": []}), d.clone()).unwrap_err();
            assert_eq!(err, "Pick at least one value for {{ids}}.");
        }
    }

    #[test]
    fn a_multi_select_value_outside_the_options_is_refused() {
        let d = json!([{"name": "ids", "type": "multiSelect", "options": [{"value": "a"}]}]);
        let err = run(
            Engine::ClickHouse,
            "x IN ({{ids}})",
            json!({"ids": ["a", "b'); DROP TABLE x; --"]}),
            d,
        )
        .unwrap_err();
        assert_eq!(err, "parameter 'ids' is not one of its choices.");
    }

    #[test]
    fn a_dropdown_prints_the_stored_option_by_its_own_type() {
        let text = json!([{"name": "c", "type": "dropdown",
            "options": [{"value": "EU"}, {"value": "US"}]}]);
        let numbers = json!([{"name": "c", "type": "dropdown",
            "options": [{"value": 1}, {"value": 2.5}]}]);
        for engine in BOTH {
            assert_eq!(
                run_ok(engine, "r = {{c}}", json!({"c": "US"}), text.clone()),
                "r = 'US'"
            );
            // The stored 2.5 is printed, not the request's "2.50".
            assert_eq!(
                run_ok(engine, "r = {{c}}", json!({"c": "2.50"}), numbers.clone()),
                "r = 2.5"
            );
            let err = run(engine, "r = {{c}}", json!({"c": "us"}), text.clone()).unwrap_err();
            assert_eq!(err, "parameter 'c' is not one of its choices.");
        }
    }

    #[test]
    fn a_default_fills_in_for_a_missing_value() {
        let d = json!([
            {"name": "region", "type": "text", "default": "EU"},
            {"name": "n", "type": "number", "default": 3},
            {"name": "r", "type": "dateRange",
             "default": {"start": "2024-01-01", "end": "2024-01-31"}},
            {"name": "ids", "type": "multiSelect", "default": ["a"],
             "options": [{"value": "a"}, {"value": "b"}]},
        ]);
        let got = run_ok(
            Engine::Trino,
            "{{region}} {{n}} {{r.start}} {{r.end}} {{ids}}",
            json!({}),
            d.clone(),
        );
        assert_eq!(got, "'EU' 3 DATE '2024-01-01' DATE '2024-01-31' 'a'");
        // A value given beats the default.
        let got = run_ok(Engine::Trino, "{{region}}", json!({"region": "US"}), d);
        assert_eq!(got, "'US'");
    }

    #[test]
    fn a_missing_value_without_a_default_is_refused_with_the_fixed_sentence() {
        for engine in BOTH {
            let err = run(engine, "r = {{region}}", json!({}), json!([])).unwrap_err();
            assert_eq!(err, "Fill in {{region}} to run this query.");
            let err = run(
                engine,
                "r = {{region}}",
                json!({"region": null}),
                json!([{"name": "region", "type": "text"}]),
            )
            .unwrap_err();
            assert_eq!(err, "Fill in {{region}} to run this query.");
        }
    }

    #[test]
    fn a_placeholder_missing_from_the_supplied_definitions_is_read_as_text() {
        let got = run_ok(
            Engine::ClickHouse,
            "{{a}} {{b}}",
            json!({"a": "1", "b": 2}),
            json!([{"name": "b", "type": "number"}]),
        );
        assert_eq!(got, "'1' 2");
    }

    #[test]
    fn the_same_placeholder_twice_is_filled_twice() {
        let got = run_ok(
            Engine::ClickHouse,
            "{{a}} OR {{ a }}",
            json!({"a": "x"}),
            json!([]),
        );
        assert_eq!(got, "'x' OR 'x'");
    }

    // ── the scanner ─────────────────────────────────────────────────────

    #[test]
    fn a_placeholder_in_a_comment_is_left_alone() {
        for engine in BOTH {
            for sql in [
                "SELECT 1 -- {{x}}",
                "SELECT 1 # {{x}}",
                "SELECT 1 /* {{x}} */",
                "SELECT 1 /* {{x}}\n{{y}} */ FROM t",
                "SELECT 1 -- {{ not a parameter\nFROM t",
                "SELECT 1 /* {{ */",
            ] {
                assert_eq!(run_ok(engine, sql, json!({}), json!([])), sql, "{sql}");
            }
        }
    }

    #[test]
    fn a_placeholder_after_a_comment_line_is_still_filled() {
        let got = run_ok(
            Engine::ClickHouse,
            "-- {{x}}\nSELECT {{y}}",
            json!({"y": "v"}),
            json!([]),
        );
        assert_eq!(got, "-- {{x}}\nSELECT 'v'");
    }

    #[test]
    fn a_placeholder_inside_quotes_is_refused_with_the_fixed_sentence() {
        for engine in BOTH {
            for sql in [
                "r = '{{region}}'",
                "r = 'a {{ region }} b'",
                "SELECT \"{{region}}\" FROM t",
                "SELECT `{{region}}` FROM t",
            ] {
                let err = run(engine, sql, json!({"region": "EU"}), json!([])).unwrap_err();
                assert_eq!(
                    err,
                    "{{region}} is inside quotes: write it without them, for example = {{region}}.",
                    "{sql}"
                );
            }
        }
    }

    #[test]
    fn a_quoted_run_with_a_doubled_quote_stays_open() {
        for engine in BOTH {
            let err = run(engine, "r = 'it''s {{region}}'", json!({}), json!([])).unwrap_err();
            assert!(err.contains("is inside quotes"), "{err}");
        }
    }

    #[test]
    fn a_brace_pair_inside_quotes_that_is_not_a_placeholder_is_left_alone() {
        for engine in BOTH {
            for sql in [
                "r LIKE 'a{{2}}'",
                "r = '{{'",
                "r = '{{ a b }}'",
                "r = \"{{\"",
            ] {
                assert_eq!(run_ok(engine, sql, json!({}), json!([])), sql, "{sql}");
            }
        }
    }

    #[test]
    fn a_backslash_before_a_quote_depends_on_the_engine() {
        // ClickHouse: the quote is escaped, so the string is still open and
        // the placeholder is inside it.
        let err = run(
            Engine::ClickHouse,
            "SELECT 'a\\' {{x}}",
            json!({"x": "v"}),
            json!([]),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "{{x}} is inside quotes: write it without them, for example = {{x}}."
        );
        // Trino: the backslash is a plain character, the string ended, and
        // the placeholder is real.
        let got = run_ok(
            Engine::Trino,
            "SELECT 'a\\' {{x}}",
            json!({"x": "v"}),
            json!([]),
        );
        assert_eq!(got, "SELECT 'a\\' 'v'");
    }

    #[test]
    fn a_brace_pair_that_is_not_a_placeholder_is_refused() {
        for engine in BOTH {
            for sql in [
                "SELECT {{",
                "SELECT {{ }}",
                "SELECT {{1a}}",
                "SELECT {{a b}}",
                "SELECT {{a",
                "SELECT {{a.b}}",
                "SELECT {{a.start.end}}",
                "SELECT {{\"a\"}}",
                "SELECT {{{a}}}",
                "SELECT {{a-b}}",
                "SELECT {{a} }",
            ] {
                let err = run(engine, sql, json!({"a": "v"}), json!([])).unwrap_err();
                assert_eq!(err, "This query has a {{ that is not a parameter.", "{sql}");
            }
        }
    }

    #[test]
    fn a_name_longer_than_sixty_four_characters_is_not_a_placeholder() {
        let sql = format!("SELECT {{{{{}}}}}", "a".repeat(65));
        let err = run(Engine::ClickHouse, &sql, json!({}), json!([])).unwrap_err();
        assert_eq!(err, "This query has a {{ that is not a parameter.");
        let ok = format!("SELECT {{{{{}}}}}", "a".repeat(64));
        let got = run_ok(
            Engine::ClickHouse,
            &ok,
            json!({"a".repeat(64): "v"}),
            json!([]),
        );
        assert_eq!(got, "SELECT 'v'");
    }

    #[test]
    fn spaces_tabs_and_newlines_inside_the_braces_are_allowed() {
        for sql in ["{{ a }}", "{{\ta\t}}", "{{\n a \n}}", "{{a }}", "{{ a}}"] {
            assert_eq!(
                run_ok(Engine::ClickHouse, sql, json!({"a": "v"}), json!([])),
                "'v'",
                "{sql:?}"
            );
        }
    }

    #[test]
    fn single_braces_and_clickhouse_query_parameters_are_left_alone() {
        for sql in [
            "SELECT {a:String}",
            "SELECT '{a}'",
            "SELECT {a}",
            "SELECT }}",
        ] {
            assert_eq!(run_ok(Engine::ClickHouse, sql, json!({}), json!([])), sql);
        }
    }

    #[test]
    fn a_statement_without_a_placeholder_comes_back_byte_for_byte() {
        for engine in BOTH {
            for sql in [
                "",
                "SELECT 1",
                "  SELECT 'é', \"ü\" -- ünïcode \u{1F600}\n FROM t  ;  ",
                "SELECT '\\' /* x */ # y",
            ] {
                assert_eq!(run_ok(engine, sql, json!({}), json!([])), sql);
                assert_eq!(run_ok(engine, sql, json!({"unused": "v"}), json!([])), sql);
            }
        }
    }

    #[test]
    fn multibyte_text_around_a_placeholder_is_kept() {
        let got = run_ok(
            Engine::Trino,
            "SELECT 'é' AS é, {{n}} AS ü -- \u{1F600}",
            json!({"n": "ö"}),
            json!([]),
        );
        assert_eq!(got, "SELECT 'é' AS é, 'ö' AS ü -- \u{1F600}");
    }

    #[test]
    fn a_value_that_looks_like_a_placeholder_is_not_scanned_again() {
        for engine in BOTH {
            let got = run_ok(engine, "a = {{x}}", json!({"x": "{{y}}"}), json!([]));
            assert_eq!(got, "a = '{{y}}'", "{engine:?}");
            let got = run_ok(engine, "a = {{x}}", json!({"x": "{{x}}"}), json!([]));
            assert_eq!(got, "a = '{{x}}'", "{engine:?}");
        }
    }

    // ── limits ──────────────────────────────────────────────────────────

    fn numbered(n: usize) -> (String, Value) {
        let sql = (0..n)
            .map(|i| format!("{{{{p{i}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let params: Map<String, Value> = (0..n).map(|i| (format!("p{i}"), json!("v"))).collect();
        (sql, Value::Object(params))
    }

    #[test]
    fn twenty_parameters_run_and_twenty_one_are_refused() {
        let (sql, params) = numbered(20);
        assert!(run(Engine::ClickHouse, &sql, params, json!([])).is_ok());
        let (sql, params) = numbered(21);
        let err = run(Engine::ClickHouse, &sql, params, json!([])).unwrap_err();
        assert_eq!(err, "A query takes at most 20 parameters.");
        // Twenty-one values alone count too.
        let (_, params) = numbered(21);
        let err = run(Engine::ClickHouse, "SELECT 1", params, json!([])).unwrap_err();
        assert_eq!(err, "A query takes at most 20 parameters.");
    }

    #[test]
    fn twenty_one_definitions_are_refused_on_a_run() {
        let many: Vec<Value> = (0..21)
            .map(|i| json!({"name": format!("p{i}"), "type": "text"}))
            .collect();
        let err = run(
            Engine::ClickHouse,
            "SELECT 1",
            json!({}),
            Value::Array(many),
        )
        .unwrap_err();
        assert_eq!(err, "A query takes at most 20 parameters.");
    }

    #[test]
    fn a_text_value_of_two_hundred_characters_runs_and_two_hundred_and_one_are_refused() {
        for engine in BOTH {
            let ok = "a".repeat(200);
            assert!(run(engine, "{{v}}", json!({ "v": ok }), json!([])).is_ok());
            let err = run(engine, "{{v}}", json!({ "v": "a".repeat(201) }), json!([])).unwrap_err();
            assert_eq!(err, "parameter 'v' is longer than 200 characters.");
        }
    }

    fn options(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({"value": format!("o{i}")})).collect()
    }

    #[test]
    fn two_hundred_options_are_accepted_and_two_hundred_and_one_are_refused() {
        for (ty, value) in [("dropdown", json!("o1")), ("multiSelect", json!(["o1"]))] {
            let ok = json!([{"name": "c", "type": ty, "options": options(200)}]);
            assert!(
                run(Engine::ClickHouse, "{{c}}", json!({"c": value}), ok).is_ok(),
                "{ty}"
            );
            let too_many = json!([{"name": "c", "type": ty, "options": options(201)}]);
            let err = run(Engine::ClickHouse, "{{c}}", json!({"c": value}), too_many).unwrap_err();
            assert_eq!(
                err,
                "{{c}} has more than 200 options; a dropdown or multi-select takes at most 200.",
                "{ty}"
            );
        }
    }

    #[test]
    fn two_hundred_values_are_accepted_and_two_hundred_and_one_are_refused() {
        let d = json!([{"name": "ids", "type": "multiSelect", "options": options(200)}]);
        let pick = |n: usize| -> Value {
            json!({"ids": (0..n).map(|i| format!("o{}", i % 200)).collect::<Vec<_>>()})
        };
        assert!(run(Engine::ClickHouse, "{{ids}}", pick(200), d.clone()).is_ok());
        let err = run(Engine::ClickHouse, "{{ids}}", pick(201), d).unwrap_err();
        assert_eq!(err, "{{ids}} takes at most 200 values.");
    }

    // ── definitions ─────────────────────────────────────────────────────

    #[test]
    fn definitions_that_match_the_sql_are_accepted() {
        let d = defs(json!([
            {"name": "region", "type": "text", "default": "EU"},
            {"name": "r", "type": "dateRange"},
            {"name": "ids", "type": "multiSelect", "options": [{"value": "a"}]},
        ]));
        for engine in BOTH {
            assert_eq!(
                validate_definitions(engine, "{{region}} {{r.start}} {{r.end}} IN ({{ids}})", &d),
                Ok(())
            );
        }
    }

    #[test]
    fn a_definition_the_sql_does_not_use_is_refused() {
        let d = defs(json!([{"name": "gone", "type": "text"}]));
        let err = validate_definitions(Engine::ClickHouse, "SELECT 1 -- {{gone}}", &d).unwrap_err();
        assert_eq!(
            err.to_string(),
            "{{gone}} has a definition but the SQL does not use it."
        );
    }

    #[test]
    fn a_placeholder_with_no_definition_is_refused_on_save() {
        let d = defs(json!([{"name": "a", "type": "text"}]));
        let err = validate_definitions(Engine::ClickHouse, "{{a}} {{b}}", &d).unwrap_err();
        assert_eq!(
            err.to_string(),
            "{{b}} is used in the SQL but has no definition."
        );
    }

    #[test]
    fn two_definitions_with_one_name_are_refused() {
        let d = defs(json!([{"name": "a", "type": "text"}, {"name": "a", "type": "number"}]));
        let err = validate_definitions(Engine::ClickHouse, "{{a}}", &d).unwrap_err();
        assert_eq!(err.to_string(), "Two parameters are named {{a}}.");
    }

    #[test]
    fn a_definition_name_that_fails_the_name_rule_is_refused_without_repeating_it() {
        for bad in ["1a", "a b", "a'b", "", &"a".repeat(65)] {
            let d = defs(json!([{ "name": bad, "type": "text" }]));
            let err = validate_definitions(Engine::ClickHouse, "SELECT 1", &d).unwrap_err();
            assert_eq!(
                err.to_string(),
                "A parameter name is 1 to 64 letters, digits or _, and does not start with a digit."
            );
        }
    }

    #[test]
    fn a_default_that_does_not_fit_its_type_is_refused() {
        for (ty, default) in [
            ("text", json!("a\nb")),
            ("text", json!(7)),
            ("number", json!("x")),
            ("date", json!("2024-02-30")),
            ("dateRange", json!({"start": "2024-01-01"})),
        ] {
            let d = defs(json!([{"name": "a", "type": ty, "default": default}]));
            let sql = if ty == "dateRange" {
                "{{a.start}} {{a.end}}"
            } else {
                "{{a}}"
            };
            assert!(
                validate_definitions(Engine::ClickHouse, sql, &d).is_err(),
                "{ty} {default}"
            );
        }
    }

    #[test]
    fn a_dropdown_needs_options_and_a_text_parameter_may_not_have_any() {
        let d = defs(json!([{"name": "a", "type": "dropdown"}]));
        let err = validate_definitions(Engine::ClickHouse, "{{a}}", &d).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("The definition of {{a}} is not valid: "),
            "{err}"
        );

        let d = defs(json!([{"name": "a", "type": "text", "options": [{"value": "x"}]}]));
        let err = validate_definitions(Engine::ClickHouse, "{{a}}", &d).unwrap_err();
        assert_eq!(
            err.to_string(),
            "The definition of {{a}} is not valid: only a dropdown or a multi-select has options."
        );
    }

    #[test]
    fn a_dropdown_default_must_be_one_of_its_options() {
        let d = defs(json!([{"name": "a", "type": "dropdown", "default": "zz",
            "options": [{"value": "x"}]}]));
        assert!(validate_definitions(Engine::ClickHouse, "{{a}}", &d).is_err());
        let d = defs(json!([{"name": "a", "type": "dropdown", "default": "x",
            "options": [{"value": "x"}]}]));
        assert_eq!(
            validate_definitions(Engine::ClickHouse, "{{a}}", &d),
            Ok(())
        );
    }

    #[test]
    fn the_scanner_refuses_on_save_too() {
        let d = defs(json!([{"name": "a", "type": "text"}]));
        let err = validate_definitions(Engine::ClickHouse, "'{{a}}'", &d).unwrap_err();
        assert!(err.to_string().contains("is inside quotes"), "{err}");
    }

    #[test]
    fn a_definition_set_serialises_with_the_wire_names() {
        let d = defs(json!([{"name": "r", "type": "dateRange"},
            {"name": "m", "type": "multiSelect", "default": ["a"],
             "options": [{"value": "a", "label": "A"}]}]));
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!([{"name": "r", "type": "dateRange"},
                {"name": "m", "type": "multiSelect", "default": ["a"],
                 "options": [{"value": "a", "label": "A"}]}])
        );
        assert!(
            serde_json::from_value::<Vec<QueryParamDef>>(
                json!([{"name": "r", "type": "text", "surprise": 1}])
            )
            .is_err()
        );
    }

    #[test]
    fn a_value_for_a_name_the_sql_does_not_use_is_ignored() {
        let got = run_ok(
            Engine::ClickHouse,
            "SELECT {{a}}",
            json!({"a": "1", "zzz": {"deep": [1]}}),
            json!([]),
        );
        assert_eq!(got, "SELECT '1'");
    }
}
