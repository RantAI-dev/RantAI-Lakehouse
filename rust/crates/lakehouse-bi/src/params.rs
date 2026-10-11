//! Parameters (`BI-11`, with `AI-6`): a named value a viewer can change on a
//! dashboard and a formula, a top-N, a reference line or a column switch can
//! read.
//!
//! # What a parameter is
//!
//! A deployment-wide definition in `console.bi_param` (versioned-insert shape
//! of `bi_field`: a save is an `INSERT`, the newest `created_at` wins under
//! `FINAL`, a delete is an `is_deleted = 1` tombstone): a `name` (an
//! identifier, immutable, unique), a `label`, a `type` (immutable) and by type
//! a `default`, a `min` / `max`, a `maxLength` or a list of `values`. The whole
//! definition is one JSON document, so a new attribute is not a new column.
//!
//! # Why a value can never be SQL
//!
//! A value is checked against its definition before anything else sees it
//! ([`check_value`]): a number is finite and inside `min` / `max`, a date is a
//! real calendar day written `YYYY-MM-DD`, text is at most the definition's
//! length with no control character, a list value is one of the listed
//! entries. What the rest of the crate gets is a [`ParamValue`], and the only
//! way it reaches a statement is [`ParamValue::sql`]: a number re-printed by
//! Rust from its parsed value, a date re-printed from its parsed year, month
//! and day, text through [`SqlLiteral`]. The request's own text is never
//! spliced.
//!
//! # A column switch is a lookup, not a pass-through
//!
//! A chart's `dimension`, `breakdown` or a measure's column may be
//! `{ "param": <list parameter> }`. The value the viewer sends is looked up in
//! the parameter's list (so it is one of the entries the editor wrote), and
//! the entry is then checked again as an [`Ident`] that names a column or
//! field of the chart's source ([`resolve_chart`]). Only a value that passed
//! both is written; anything else is the tile's plain sentence.
//!
//! In a stored chart the reference is kept as the text `{name}` (never a valid
//! identifier, so an unresolved one cannot reach a statement: every builder
//! path refuses it) and written to the wire as `{ "param": "name" }`.
//!
//! # Where values ride
//!
//! In [`crate::grain::TimeContext`], the one context every compile and build
//! path already receives (formulas, fields, the builder, the stored statement
//! of a chart), so no signature had to grow.

use std::collections::{BTreeMap, BTreeSet};

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::formula::compile::{FType, print_number};
use crate::shape::MeasureList;
use crate::specs::ChartKind;
use crate::store::{ChartInput, StoredChartSpec, ensure_bi_table, random_hex};

/// Longest parameter name, in characters.
pub const MAX_NAME_LEN: usize = 64;
/// Longest label, in characters.
pub const MAX_LABEL_LEN: usize = 100;
/// Longest text value, in characters.
pub const MAX_TEXT_LEN: usize = 200;
/// Most entries a list may hold.
pub const MAX_LIST_VALUES: usize = 200;
/// Most parameters a deployment may define.
pub const MAX_PARAMS: usize = 200;
/// Most reference lines on one chart.
pub const MAX_REFERENCE_LINES: usize = 5;
/// Longest reference line label, in characters.
pub const MAX_LINE_LABEL_LEN: usize = 60;
/// Earliest and latest year a date parameter may hold: the range `Date32`
/// prints without clipping.
const YEARS: std::ops::RangeInclusive<i32> = 1900..=2299;
/// Numbers at or above this magnitude are refused, as in a formula.
const MAX_NUMBER: f64 = 1e15;

// ── the definition ──────────────────────────────────────────────────────

/// The type of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    /// A finite number, optionally within `min` and `max`.
    Number,
    /// Text of at most `maxLength` characters.
    Text,
    /// A calendar day, optionally within `min` and `max`.
    Date,
    /// One of a fixed list of values.
    List,
}

impl ParamType {
    /// The wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Text => "text",
            Self::Date => "date",
            Self::List => "list",
        }
    }
}

/// One entry of a list parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEntry {
    /// Text or a number; all entries of a list are one or the other.
    pub value: Value,
    /// What the control shows. Defaults to the value.
    #[serde(default)]
    pub label: String,
}

/// A saved parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDef {
    /// `p_<8 hex>`.
    #[serde(default)]
    pub id: String,
    /// The identifier a formula and a chart refer to it by.
    pub name: String,
    /// What people call it.
    #[serde(default)]
    pub label: String,
    /// Its type.
    #[serde(rename = "type")]
    pub ty: ParamType,
    /// The value it has when nobody chose one.
    #[serde(default)]
    pub default: Value,
    /// Smallest value (number, date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    /// Largest value (number, date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    /// Longest text, in characters (text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// The choices (list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<ListEntry>>,
    /// Display name of whoever last saved it.
    #[serde(default)]
    pub created_by: String,
    /// When this version was saved, `ClickHouse`-formatted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A fresh parameter id.
#[must_use]
pub fn new_id() -> String {
    format!("p_{}", random_hex(4))
}

/// Whether `name` is a parameter name: an identifier of at most
/// [`MAX_NAME_LEN`] characters.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    name.chars().count() <= MAX_NAME_LEN && Ident::new(name).is_ok()
}

fn number_of(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n.abs() < MAX_NUMBER).then_some(n)
}

fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < MAX_NUMBER {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "n is whole and below 1e15 in magnitude, well inside i64"
        )]
        let whole = n as i64;
        return Value::from(whole);
    }
    serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
}

/// A calendar day written exactly `YYYY-MM-DD`, in [`YEARS`].
fn parse_date(raw: &str) -> Option<(i32, u32, u32)> {
    let b = raw.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if ![0, 1, 2, 3, 5, 6, 8, 9]
        .iter()
        .all(|i| b[*i].is_ascii_digit())
    {
        return None;
    }
    let year: i32 = raw[0..4].parse().ok()?;
    let month: u32 = raw[5..7].parse().ok()?;
    let day: u32 = raw[8..10].parse().ok()?;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (YEARS.contains(&year) && (1..=days).contains(&day)).then_some((year, month, day))
}

fn date_text(d: (i32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

fn plain_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

fn clean_name(raw: &str) -> String {
    raw.trim().to_owned()
}

/// Check a definition and return it in canonical form (numbers as numbers,
/// dates as `YYYY-MM-DD`, list labels filled in).
///
/// # Errors
///
/// The sentence to show the person who defined it.
pub fn validate_def(def: &ParamDef) -> Result<ParamDef, String> {
    let mut out = def.clone();
    out.name = clean_name(&def.name);
    if !valid_name(&out.name) {
        return Err(format!(
            "a parameter name is 1 to {MAX_NAME_LEN} letters, digits or _, and does not start with a digit."
        ));
    }
    def.label.trim().clone_into(&mut out.label);
    if out.label.is_empty() {
        out.label.clone_from(&out.name);
    }
    if out.label.chars().count() > MAX_LABEL_LEN || out.label.chars().any(char::is_control) {
        return Err(format!("a label is at most {MAX_LABEL_LEN} characters."));
    }
    match def.ty {
        ParamType::Number => number_def(&mut out)?,
        ParamType::Date => date_def(&mut out)?,
        ParamType::Text => text_def(&mut out)?,
        ParamType::List => list_def(&mut out)?,
    }
    Ok(out)
}

fn only_bounds_for_ranged(def: &ParamDef) -> Result<(), String> {
    if def.max_length.is_some() && def.ty != ParamType::Text {
        return Err("a maximum length is for a text parameter.".to_owned());
    }
    if def.values.is_some() && def.ty != ParamType::List {
        return Err("a list of values is for a list parameter.".to_owned());
    }
    if (def.min.is_some() || def.max.is_some())
        && !matches!(def.ty, ParamType::Number | ParamType::Date)
    {
        return Err("a minimum and a maximum are for a number or a date.".to_owned());
    }
    Ok(())
}

fn number_def(out: &mut ParamDef) -> Result<(), String> {
    only_bounds_for_ranged(out)?;
    let bound = |v: &Option<Value>, word: &str| -> Result<Option<f64>, String> {
        v.as_ref()
            .filter(|v| !v.is_null())
            .map(|v| number_of(v).ok_or_else(|| format!("the {word} must be a number.")))
            .transpose()
    };
    let min = bound(&out.min, "minimum")?;
    let max = bound(&out.max, "maximum")?;
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err("the minimum is above the maximum.".to_owned());
    }
    let default = number_of(&out.default).ok_or("the default must be a number.")?;
    if min.is_some_and(|lo| default < lo) || max.is_some_and(|hi| default > hi) {
        return Err("the default is outside the minimum and maximum.".to_owned());
    }
    out.min = min.map(number_value);
    out.max = max.map(number_value);
    out.default = number_value(default);
    Ok(())
}

fn date_def(out: &mut ParamDef) -> Result<(), String> {
    only_bounds_for_ranged(out)?;
    let bound = |v: &Option<Value>, word: &str| -> Result<Option<(i32, u32, u32)>, String> {
        v.as_ref()
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .and_then(parse_date)
                    .ok_or_else(|| format!("the {word} must be a date written YYYY-MM-DD."))
            })
            .transpose()
    };
    let min = bound(&out.min, "minimum")?;
    let max = bound(&out.max, "maximum")?;
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err("the minimum is above the maximum.".to_owned());
    }
    let default = out
        .default
        .as_str()
        .and_then(parse_date)
        .ok_or("the default must be a date written YYYY-MM-DD.")?;
    if min.is_some_and(|lo| default < lo) || max.is_some_and(|hi| default > hi) {
        return Err("the default is outside the minimum and maximum.".to_owned());
    }
    out.min = min.map(|d| Value::String(date_text(d)));
    out.max = max.map(|d| Value::String(date_text(d)));
    out.default = Value::String(date_text(default));
    Ok(())
}

fn text_def(out: &mut ParamDef) -> Result<(), String> {
    only_bounds_for_ranged(out)?;
    let max = out
        .max_length
        .unwrap_or(u32::try_from(MAX_TEXT_LEN).unwrap_or(200));
    if max == 0 || max as usize > MAX_TEXT_LEN {
        return Err(format!(
            "the longest text is 1 to {MAX_TEXT_LEN} characters."
        ));
    }
    out.max_length = Some(max);
    let default = out.default.as_str().ok_or("the default must be text.")?;
    if default.chars().count() > max as usize || default.chars().any(char::is_control) {
        return Err(
            "the default is longer than the longest text or has control characters.".to_owned(),
        );
    }
    Ok(())
}

fn list_def(out: &mut ParamDef) -> Result<(), String> {
    only_bounds_for_ranged(out)?;
    let entries = out.values.take().unwrap_or_default();
    if entries.is_empty() || entries.len() > MAX_LIST_VALUES {
        return Err(format!("a list has 1 to {MAX_LIST_VALUES} values."));
    }
    let numbers = entries.iter().all(|e| e.value.is_number());
    let texts = entries.iter().all(|e| e.value.is_string());
    if !(numbers || texts) {
        return Err("the values of a list are all text or all numbers.".to_owned());
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut clean = Vec::with_capacity(entries.len());
    for e in entries {
        let (value, key) = if numbers {
            let n = number_of(&e.value).ok_or("a list value must be a finite number.")?;
            (number_value(n), print_number(n))
        } else {
            let s = e.value.as_str().unwrap_or_default();
            if !plain_text(s, MAX_TEXT_LEN) {
                return Err(format!(
                    "a list value is 1 to {MAX_TEXT_LEN} characters without control characters."
                ));
            }
            (e.value.clone(), s.to_owned())
        };
        if !seen.insert(key.clone()) {
            return Err("a value is listed twice.".to_owned());
        }
        let label = e.label.trim().to_owned();
        let label = if label.is_empty() { key } else { label };
        if label.chars().count() > MAX_LABEL_LEN || label.chars().any(char::is_control) {
            return Err(format!(
                "a list label is at most {MAX_LABEL_LEN} characters."
            ));
        }
        clean.push(ListEntry { value, label });
    }
    let default_key = if numbers {
        number_of(&out.default).map(print_number)
    } else {
        out.default.as_str().map(str::to_owned)
    };
    let Some(default_key) = default_key.filter(|k| seen.contains(k)) else {
        return Err("the default must be one of the values.".to_owned());
    };
    out.default = clean
        .iter()
        .find(|e| {
            if numbers {
                number_of(&e.value).map(print_number).as_deref() == Some(default_key.as_str())
            } else {
                e.value.as_str() == Some(default_key.as_str())
            }
        })
        .map_or(Value::Null, |e| e.value.clone());
    out.values = Some(clean);
    Ok(())
}

// ── a value ─────────────────────────────────────────────────────────────

/// A checked value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// A finite number below 1e15 in magnitude.
    Number(f64),
    /// Text with no control character.
    Text(String),
    /// A real calendar day, `YYYY-MM-DD`.
    Date(String),
}

// Eq is sound: a `Number` is finite by construction (`number_of`).
impl Eq for ParamValue {}

impl ParamValue {
    /// The formula type this value has.
    #[must_use]
    pub fn ftype(&self) -> FType {
        match self {
            Self::Number(_) => FType::Number,
            Self::Text(_) => FType::Text,
            Self::Date(_) => FType::Date,
        }
    }

    /// The value as a `ClickHouse` literal: a number re-printed from its
    /// value, a date re-printed from its parsed parts, text escaped by
    /// [`SqlLiteral`].
    #[must_use]
    pub fn sql(&self) -> String {
        match self {
            Self::Number(n) => print_number(*n),
            Self::Text(t) => SqlLiteral::from(t.as_str()).to_string(),
            Self::Date(d) => match parse_date(d) {
                Some(parts) => format!("toDate32('{}')", date_text(parts)),
                // Unreachable for a value built by `check_value`; an empty
                // value rather than the text if it ever were.
                None => "NULL".to_owned(),
            },
        }
    }

    /// The value as JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Number(n) => number_value(*n),
            Self::Text(t) | Self::Date(t) => Value::String(t.clone()),
        }
    }
}

fn bound_number(v: Option<&Value>) -> Option<f64> {
    v.and_then(number_of)
}

fn bound_date(v: Option<&Value>) -> Option<(i32, u32, u32)> {
    v.and_then(Value::as_str).and_then(parse_date)
}

/// Check `raw` against `def`. The sentence names the parameter and what is
/// wrong with the value; it never repeats the value.
///
/// # Errors
///
/// A fixed sentence when the value does not fit the definition.
pub fn check_value(def: &ParamDef, raw: &Value) -> Result<ParamValue, String> {
    let name = &def.name;
    match def.ty {
        ParamType::Number => {
            let n =
                number_of(raw).ok_or_else(|| format!("parameter '{name}' must be a number."))?;
            if let Some(lo) = bound_number(def.min.as_ref())
                && n < lo
            {
                return Err(format!(
                    "parameter '{name}' must be at least {}.",
                    print_number(lo)
                ));
            }
            if let Some(hi) = bound_number(def.max.as_ref())
                && n > hi
            {
                return Err(format!(
                    "parameter '{name}' must be at most {}.",
                    print_number(hi)
                ));
            }
            Ok(ParamValue::Number(n))
        }
        ParamType::Date => {
            let d = raw
                .as_str()
                .and_then(parse_date)
                .ok_or_else(|| format!("parameter '{name}' must be a date written YYYY-MM-DD."))?;
            if let Some(lo) = bound_date(def.min.as_ref())
                && d < lo
            {
                return Err(format!(
                    "parameter '{name}' must be on or after {}.",
                    date_text(lo)
                ));
            }
            if let Some(hi) = bound_date(def.max.as_ref())
                && d > hi
            {
                return Err(format!(
                    "parameter '{name}' must be on or before {}.",
                    date_text(hi)
                ));
            }
            Ok(ParamValue::Date(date_text(d)))
        }
        ParamType::Text => {
            let max = def.max_length.map_or(MAX_TEXT_LEN, |m| m as usize);
            let s = raw
                .as_str()
                .ok_or_else(|| format!("parameter '{name}' must be text."))?;
            if s.chars().count() > max {
                return Err(format!(
                    "parameter '{name}' is longer than {max} characters."
                ));
            }
            if s.chars().any(char::is_control) {
                return Err(format!(
                    "parameter '{name}' has characters that are not allowed."
                ));
            }
            Ok(ParamValue::Text(s.to_owned()))
        }
        ParamType::List => list_choice(def, raw)
            .ok_or_else(|| format!("parameter '{name}' is not one of its choices.")),
    }
}

/// The listed entry `raw` stands for: the entry's own value is returned, never
/// the request's.
fn list_choice(def: &ParamDef, raw: &Value) -> Option<ParamValue> {
    let entries = def.values.as_deref()?;
    let numeric = entries.first().is_some_and(|e| e.value.is_number());
    if numeric {
        let wanted = number_of(raw)?;
        entries
            .iter()
            .find(|e| number_of(&e.value) == Some(wanted))
            .and_then(|e| number_of(&e.value))
            .map(ParamValue::Number)
    } else {
        let wanted = raw.as_str()?;
        entries
            .iter()
            .find(|e| e.value.as_str() == Some(wanted))
            .and_then(|e| e.value.as_str())
            .map(|s| ParamValue::Text(s.to_owned()))
    }
}

impl ParamDef {
    /// The default as a checked value; `None` when the stored default no
    /// longer fits (data corruption only).
    #[must_use]
    pub fn default_value(&self) -> Option<ParamValue> {
        check_value(self, &self.default).ok()
    }

    /// The listed text values of a list parameter that holds text.
    #[must_use]
    pub fn text_choices(&self) -> Vec<&str> {
        if self.ty != ParamType::List {
            return Vec::new();
        }
        self.values
            .iter()
            .flatten()
            .filter_map(|e| e.value.as_str())
            .collect()
    }
}

// ── the set a read uses ─────────────────────────────────────────────────

/// A parameter and the value this read gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The definition.
    pub def: ParamDef,
    /// The checked value: the request's, else the board's saved one, else the
    /// default.
    pub value: ParamValue,
}

/// A name saved on a board with the value its editor saved for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardParam {
    /// The parameter.
    pub name: String,
    /// The saved value, absent when the board shows the control with the
    /// parameter's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// The parameters of one read, with the value each has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamValues {
    by_name: BTreeMap<String, Resolved>,
}

static NO_PARAMS: ParamValues = ParamValues {
    by_name: BTreeMap::new(),
};

impl ParamValues {
    /// No parameters: a read that does not use them.
    #[must_use]
    pub fn none() -> &'static Self {
        &NO_PARAMS
    }

    /// Every definition at its default (a definition whose default no longer
    /// fits is left out, so a use of it is an unknown parameter).
    #[must_use]
    pub fn from_defaults(defs: &[ParamDef]) -> Self {
        let mut by_name = BTreeMap::new();
        for def in defs {
            if let Some(value) = def.default_value() {
                by_name.insert(
                    def.name.clone(),
                    Resolved {
                        def: def.clone(),
                        value,
                    },
                );
            }
        }
        Self { by_name }
    }

    /// The values of one request: each definition takes the value the request
    /// sent, else the board's saved one (checked again; one that no longer
    /// fits is ignored), else its default.
    ///
    /// # Errors
    ///
    /// A fixed sentence naming the parameter when a value the request sent
    /// does not fit its definition, or a sentence without the name when the
    /// request names a parameter that does not exist (the name is the
    /// caller's text and is not repeated).
    pub fn resolve(
        defs: &[ParamDef],
        saved: &[BoardParam],
        request: Option<&Map<String, Value>>,
    ) -> Result<Self, String> {
        if let Some(sent) = request
            && sent.keys().any(|k| !defs.iter().any(|d| d.name == *k))
        {
            return Err("the request names a parameter that does not exist.".to_owned());
        }
        let mut by_name = BTreeMap::new();
        for def in defs {
            let chosen = match request.and_then(|r| r.get(&def.name)) {
                Some(raw) => Some(check_value(def, raw)?),
                None => saved
                    .iter()
                    .find(|b| b.name == def.name)
                    .and_then(|b| b.value.as_ref())
                    .and_then(|raw| check_value(def, raw).ok()),
            };
            if let Some(value) = chosen.or_else(|| def.default_value()) {
                by_name.insert(
                    def.name.clone(),
                    Resolved {
                        def: def.clone(),
                        value,
                    },
                );
            }
        }
        Ok(Self { by_name })
    }

    /// One parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Resolved> {
        self.by_name.get(name)
    }

    /// Every parameter, by name.
    pub fn iter(&self) -> impl Iterator<Item = &Resolved> {
        self.by_name.values()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The value of every parameter as JSON, by name.
    #[must_use]
    pub fn values_json(&self) -> Value {
        Value::Object(
            self.by_name
                .iter()
                .map(|(k, r)| (k.clone(), r.value.to_json()))
                .collect(),
        )
    }
}

/// The `params` query / body value: a JSON object, name to value.
///
/// # Errors
///
/// A fixed sentence when the text is not a JSON object.
pub fn parse_request(raw: Option<&str>) -> Result<Option<Map<String, Value>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        _ => Err("params must be a JSON object of parameter names and values.".to_owned()),
    }
}

// ── storage ─────────────────────────────────────────────────────────────

const COLS: &str = "id, name, def_json, created_by, toString(created_at) AS updated_at";

fn row_str<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_to_def(row: &Map<String, Value>) -> Option<ParamDef> {
    let mut def: ParamDef = serde_json::from_str(row_str(row, "def_json")).ok()?;
    row_str(row, "id").clone_into(&mut def.id);
    row_str(row, "name").clone_into(&mut def.name);
    row_str(row, "created_by").clone_into(&mut def.created_by);
    def.updated_at = Some(row_str(row, "updated_at").to_owned());
    Some(def)
}

/// Every live parameter, by name.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_params(ch: &ChClient) -> Result<Vec<ParamDef>, ChError> {
    ensure_bi_table(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {COLS} FROM console.bi_param FINAL WHERE is_deleted = 0 ORDER BY name LIMIT {MAX_PARAMS}"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().filter_map(row_to_def).collect())
}

/// Save (create or replace) a parameter. The caller has validated it.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn save_param(ch: &ChClient, def: &ParamDef) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let json = serde_json::to_string(def).unwrap_or_else(|_| "{}".to_owned());
    let sql = format!(
        "INSERT INTO console.bi_param (id, name, def_json, created_by) VALUES ({}, {}, {}, {})",
        SqlLiteral::from(def.id.as_str()),
        SqlLiteral::from(def.name.as_str()),
        SqlLiteral::from(json),
        SqlLiteral::from(def.created_by.as_str()),
    );
    ch.exec(&sql, None).await
}

/// Tombstone a parameter.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_param(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure_bi_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.bi_param (id, is_deleted) VALUES ({}, 1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

// ── references in a chart ───────────────────────────────────────────────

/// `{ "param": <name> }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamRef {
    /// A parameter name.
    pub param: String,
}

/// A chart's row limit: a number, or the value of a number parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Limit {
    /// A fixed limit.
    Fixed(u32),
    /// The value of a number parameter, a whole number within the chart
    /// kind's limit range at read time.
    Param(ParamRef),
}

impl From<u32> for Limit {
    fn from(n: u32) -> Self {
        Self::Fixed(n)
    }
}

/// Where a reference line stands: a number, or a number parameter's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LineValue {
    /// A fixed value.
    Fixed(f64),
    /// A number parameter.
    Param(ParamRef),
}

/// A reference line drawn on the value axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceLine {
    /// Where.
    pub value: LineValue,
    /// Its label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Kinds that can draw reference lines: the ones with a value axis.
#[must_use]
pub fn takes_reference_lines(kind: ChartKind) -> bool {
    matches!(
        kind,
        ChartKind::Bar
            | ChartKind::Hbar
            | ChartKind::Line
            | ChartKind::Area
            | ChartKind::Stacked
            | ChartKind::Combo
    )
}

/// Kinds that can switch a column or take their limit from a parameter:
/// the ones that list groups along one dimension, drawn from the measure
/// names of the stored chart (not a pivot, a table, a map, or a chart with
/// steps or groupings, whose result columns are named by other rules).
fn takes_param_slots(def: &ChartInput) -> bool {
    !matches!(
        def.kind,
        ChartKind::Pivot
            | ChartKind::Table
            | ChartKind::Pointmap
            | ChartKind::Geoheat
            | ChartKind::Boxplot
            | ChartKind::Text
    ) && def.shape.steps.is_none()
        && def.shape.groupings.is_none()
        && !def.tables.is_rows_mode(def.kind)
}

/// Check the parameter-related fields of a chart (the kind and shape rules,
/// before any parameter is looked up).
///
/// # Errors
///
/// The sentence to show.
pub fn check_chart_shape(def: &ChartInput) -> Result<(), String> {
    let lines = def.shape.reference_lines.as_deref().unwrap_or_default();
    if !lines.is_empty() {
        if !takes_reference_lines(def.kind) {
            return Err(
                "reference lines are for a bar, horizontal bar, line, area, stacked or combo chart."
                    .to_owned(),
            );
        }
        if lines.len() > MAX_REFERENCE_LINES {
            return Err(format!(
                "a chart has at most {MAX_REFERENCE_LINES} reference lines."
            ));
        }
        for line in lines {
            match &line.value {
                LineValue::Fixed(v) if !(v.is_finite() && v.abs() < MAX_NUMBER) => {
                    return Err("a reference line value must be a finite number.".to_owned());
                }
                LineValue::Param(p) if !valid_name(&p.param) => {
                    return Err("a reference line names an invalid parameter.".to_owned());
                }
                _ => {}
            }
            if let Some(label) = &line.label
                && (label.chars().count() > MAX_LINE_LABEL_LEN
                    || label.chars().any(char::is_control))
            {
                return Err(format!(
                    "a reference line label is at most {MAX_LINE_LABEL_LEN} characters."
                ));
            }
        }
    }
    if (!slots(def).is_empty() || matches!(def.limit, Some(Limit::Param(_))))
        && !takes_param_slots(def)
    {
        return Err(
            "a parameter can stand for a column or the limit of a chart that lists groups along one dimension (not a pivot, table, map, box plot, or a chart with steps or groupings)."
                .to_owned(),
        );
    }
    Ok(())
}

// ── the stored reference ────────────────────────────────────────────────

/// The text a stored chart keeps for "the column named by parameter `name`":
/// never a valid identifier, so an unresolved one cannot reach a statement.
#[must_use]
pub fn slot_text(name: &str) -> String {
    format!("{{{name}}}")
}

/// The parameter a stored slot refers to, if it is one.
#[must_use]
pub fn slot_param(text: &str) -> Option<&str> {
    text.strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .filter(|n| valid_name(n))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawSlot {
    Text(String),
    Param(ParamRef),
}

fn slot_from_raw<E: serde::de::Error>(raw: RawSlot) -> Result<String, E> {
    match raw {
        RawSlot::Text(t) => Ok(t),
        RawSlot::Param(p) if valid_name(&p.param) => Ok(slot_text(&p.param)),
        RawSlot::Param(_) => Err(E::custom("a parameter name is letters, digits and _")),
    }
}

/// Serde for a column slot that may be a parameter: a string, or
/// `{ "param": name }` (kept as the text `{name}`).
pub mod slot {
    use super::{Deserialize, Deserializer, ParamRef, RawSlot, Serialize, Serializer};

    /// # Errors
    ///
    /// The deserializer's error.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        super::slot_from_raw(RawSlot::deserialize(d)?)
    }

    /// # Errors
    ///
    /// The serializer's error.
    pub fn serialize<S: Serializer>(value: &str, s: S) -> Result<S::Ok, S::Error> {
        match super::slot_param(value) {
            Some(name) => ParamRef {
                param: name.to_owned(),
            }
            .serialize(s),
            None => s.serialize_str(value),
        }
    }
}

/// [`slot`] for an optional column.
pub mod slot_opt {
    use super::{Deserialize, Deserializer, ParamRef, RawSlot, Serialize, Serializer};

    /// # Errors
    ///
    /// The deserializer's error.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Option::<RawSlot>::deserialize(d)?
            .map(super::slot_from_raw)
            .transpose()
    }

    /// # Errors
    ///
    /// The serializer's error.
    #[allow(
        clippy::ref_option,
        reason = "serde's `serialize_with` contract requires `&Option<String>`"
    )]
    pub fn serialize<S: Serializer>(value: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        match value.as_deref() {
            None => s.serialize_none(),
            Some(v) => match super::slot_param(v) {
                Some(name) => ParamRef {
                    param: name.to_owned(),
                }
                .serialize(s),
                None => s.serialize_str(v),
            },
        }
    }
}

/// A column slot of a chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// `dimension`.
    Dimension,
    /// `breakdown`.
    Breakdown,
    /// The measure at this position.
    Measure(usize),
}

/// The slots of `def` that stand for a parameter, with its name.
#[must_use]
pub fn slots(def: &ChartInput) -> Vec<(Slot, String)> {
    let mut out = Vec::new();
    if let Some(name) = slot_param(&def.dimension) {
        out.push((Slot::Dimension, name.to_owned()));
    }
    if let Some(name) = def.breakdown.as_deref().and_then(slot_param) {
        out.push((Slot::Breakdown, name.to_owned()));
    }
    for (i, (column, _)) in def.measures.entries().enumerate() {
        if let Some(name) = slot_param(column) {
            out.push((Slot::Measure(i), name.to_owned()));
        }
    }
    out
}

/// Every parameter a chart's definition names: its limit, column slots and
/// reference lines. (A chart that uses a field that uses a parameter names
/// only the field.)
#[must_use]
pub fn chart_param_names(def: &ChartInput) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = slots(def).into_iter().map(|(_, n)| n).collect();
    if let Some(Limit::Param(p)) = &def.limit {
        out.insert(p.param.clone());
    }
    for line in def.shape.reference_lines.iter().flatten() {
        if let LineValue::Param(p) = &line.value {
            out.insert(p.param.clone());
        }
    }
    out
}

/// Whether the statement of `def` depends on a parameter's value: a limit or
/// a column slot (a reference line changes no statement).
#[must_use]
pub fn changes_statement(def: &ChartInput) -> bool {
    matches!(def.limit, Some(Limit::Param(_))) || !slots(def).is_empty()
}

/// Ids of the stored charts whose definition names the parameter `name`.
#[must_use]
pub fn charts_using<'a>(charts: &'a [StoredChartSpec], name: &str) -> Vec<&'a str> {
    charts
        .iter()
        .filter(|c| chart_param_names(&c.def).contains(name))
        .map(|c| c.spec.id.as_str())
        .collect()
}

/// The largest whole limit a chart of this definition may take from a
/// parameter: what the chart would be saved with.
fn limit_max(def: &ChartInput) -> u32 {
    if def.grain.is_some() {
        crate::grain::MAX_GRAIN_LIMIT
    } else if def.kind == ChartKind::Calendar {
        366
    } else {
        100
    }
}

/// `def` with every parameter slot replaced by a concrete value, for a read
/// or for a save-time check. `pick` gives the column text a slot takes; the
/// limit is taken from the number parameter's value.
///
/// # Errors
///
/// A plain sentence for the tile when a parameter is missing, of the wrong
/// type, or its value does not fit the slot.
pub fn substitute(
    def: &ChartInput,
    params: &ParamValues,
    pick: &dyn Fn(Slot, &str) -> Option<String>,
) -> Result<ChartInput, String> {
    let mut out = def.clone();
    for (slot, name) in slots(def) {
        let r = params
            .get(&name)
            .ok_or_else(|| format!("the parameter '{name}' does not exist."))?;
        if r.def.ty != ParamType::List || !matches!(r.value, ParamValue::Text(_)) {
            return Err(format!(
                "the parameter '{name}' must be a list of column names."
            ));
        }
        let ParamValue::Text(chosen) = &r.value else {
            return Err(format!(
                "the parameter '{name}' must be a list of column names."
            ));
        };
        // The viewer's value was looked up in the list when the parameter was
        // resolved; the entry is checked here once more as an identifier and,
        // by `pick`, as a column or field of this chart's source.
        let in_list = r.def.text_choices().contains(&chosen.as_str());
        let column = pick(slot, chosen).filter(|c| in_list && Ident::new(c.as_str()).is_ok());
        let Some(column) = column else {
            return Err(format!(
                "the value of parameter '{name}' is not a column of this chart's source."
            ));
        };
        match slot {
            Slot::Dimension => out.dimension = column,
            Slot::Breakdown => out.breakdown = Some(column),
            Slot::Measure(i) => {
                out.measures = MeasureList::from_pairs(
                    out.measures
                        .entries()
                        .enumerate()
                        .map(|(j, (c, a))| {
                            (
                                if j == i { column.clone() } else { c.to_owned() },
                                a.map(str::to_owned),
                            )
                        })
                        .collect(),
                );
            }
        }
    }
    if let Some(Limit::Param(p)) = &def.limit {
        let r = params
            .get(&p.param)
            .ok_or_else(|| format!("the parameter '{}' does not exist.", p.param))?;
        let max = limit_max(def);
        let whole = match (&r.def.ty, &r.value) {
            (ParamType::Number, ParamValue::Number(n))
                if n.fract() == 0.0 && *n >= 1.0 && *n <= f64::from(max) =>
            {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "n is whole and checked to lie in 1..=max, a u32"
                )]
                Some(*n as u32)
            }
            _ => None,
        };
        let Some(n) = whole else {
            return Err(format!(
                "the value of parameter '{}' must be a whole number from 1 to {max} to be the number of rows of this chart.",
                p.param
            ));
        };
        out.limit = Some(Limit::Fixed(n));
    }
    Ok(out)
}

/// `chart` for this read: every parameter slot of its definition replaced by
/// the value the read gives it, the render spec's columns (`x`, `y`,
/// `series`) following, and marked so the builder rebuilds the statement
/// instead of serving the stored one. `known` says whether a name is a column
/// or a field of the chart's source.
///
/// # Errors
///
/// The tile's plain sentence (see [`substitute`]).
pub fn resolve_chart(
    chart: &StoredChartSpec,
    params: &ParamValues,
    known: &dyn Fn(&str) -> bool,
) -> Result<StoredChartSpec, String> {
    if !changes_statement(&chart.def) {
        return Ok(chart.clone());
    }
    let def = substitute(&chart.def, params, &|_slot, column| {
        known(column).then(|| column.to_owned())
    })?;
    let mut out = chart.clone();
    let measure_names: Vec<String> = def.measures.to_vec();
    out.spec.x.clone_from(&def.dimension);
    out.spec.series.clone_from(&def.breakdown);
    out.spec.y = if measure_names.len() == 1 {
        crate::specs::ChartY::Single(measure_names.into_iter().next().unwrap_or_default())
    } else {
        crate::specs::ChartY::Multi(measure_names)
    };
    out.def = def;
    out.def.shape.resolved_params = true;
    Ok(out)
}

/// A reference line with its value resolved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedLine {
    /// Where.
    pub value: f64,
    /// Its label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The reference lines of `def` with the values of this read. A line whose
/// parameter is missing or is not a number is left out.
#[must_use]
pub fn reference_lines(def: &ChartInput, params: &ParamValues) -> Vec<ResolvedLine> {
    def.shape
        .reference_lines
        .iter()
        .flatten()
        .filter_map(|line| {
            let value = match &line.value {
                LineValue::Fixed(v) => *v,
                LineValue::Param(p) => match params.get(&p.param).map(|r| &r.value) {
                    Some(ParamValue::Number(n)) => *n,
                    _ => return None,
                },
            };
            Some(ResolvedLine {
                value,
                label: line.label.clone(),
            })
        })
        .collect()
}

/// A control the console draws for a parameter of a board: the definition and
/// the value of this read.
#[must_use]
pub fn control_json(r: &Resolved) -> Value {
    serde_json::json!({
        "param": r.def,
        "value": r.value.to_json(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;
    use crate::filters::ColumnKind;
    use crate::specs::ChartSource;
    use crate::store::ChartSpec;

    fn base(name: &str, ty: ParamType, default: Value) -> ParamDef {
        ParamDef {
            id: "p_1".to_owned(),
            name: name.to_owned(),
            label: String::new(),
            ty,
            default,
            min: None,
            max: None,
            max_length: None,
            values: None,
            created_by: String::new(),
            updated_at: None,
        }
    }

    fn list(name: &str, values: &[&str], default: &str) -> ParamDef {
        let mut d = base(name, ParamType::List, json!(default));
        d.values = Some(
            values
                .iter()
                .map(|v| ListEntry {
                    value: json!(v),
                    label: String::new(),
                })
                .collect(),
        );
        d
    }

    #[test]
    fn each_type_is_validated_and_canonical() {
        let mut n = base("target", ParamType::Number, json!("1000"));
        n.min = Some(json!(0));
        n.max = Some(json!("5000"));
        let n = validate_def(&n).unwrap();
        assert_eq!(
            (n.default, n.min, n.max),
            (json!(1000), Some(json!(0)), Some(json!(5000)))
        );
        assert_eq!(n.label, "target", "the label defaults to the name");

        let mut d = base("since", ParamType::Date, json!("2026-02-28"));
        d.max = Some(json!("2026-12-31"));
        assert!(validate_def(&d).is_ok());

        let mut t = base("who", ParamType::Text, json!("x"));
        t.max_length = Some(10);
        assert_eq!(validate_def(&t).unwrap().max_length, Some(10));
        assert_eq!(
            validate_def(&base("who", ParamType::Text, json!("x")))
                .unwrap()
                .max_length,
            Some(200)
        );

        let l = validate_def(&list("pick", &["visitors", "rooms"], "rooms")).unwrap();
        assert_eq!(l.values.unwrap()[0].label, "visitors");
    }

    #[test]
    fn each_refusal_is_a_sentence() {
        let bad = |d: ParamDef| validate_def(&d).unwrap_err();
        assert!(bad(base("1x", ParamType::Number, json!(1))).contains("name"));
        assert!(bad(base("a b", ParamType::Number, json!(1))).contains("name"));
        assert!(bad(base(&"a".repeat(65), ParamType::Number, json!(1))).contains("name"));
        assert!(bad(base("n", ParamType::Number, json!("abc"))).contains("number"));
        assert!(bad(base("n", ParamType::Number, json!(1e300))).contains("number"));
        let mut n = base("n", ParamType::Number, json!(10));
        n.min = Some(json!(20));
        assert!(bad(n.clone()).contains("outside"));
        n.min = Some(json!(5));
        n.max = Some(json!(1));
        assert!(bad(n).contains("above"));
        assert!(bad(base("d", ParamType::Date, json!("2026-02-30"))).contains("date"));
        assert!(bad(base("d", ParamType::Date, json!("26-02-03"))).contains("date"));
        assert!(bad(base("d", ParamType::Date, json!("1800-01-01"))).contains("date"));
        let mut t = base("t", ParamType::Text, json!("x"));
        t.max_length = Some(201);
        assert!(bad(t).contains("longest"));
        assert!(bad(base("t", ParamType::Text, json!("a\nb"))).contains("control"));
        assert!(bad(list("l", &[], "")).contains("1 to 200"));
        assert!(bad(list("l", &["a", "a"], "a")).contains("twice"));
        assert!(bad(list("l", &["a"], "b")).contains("default"));
        let many: Vec<String> = (0..201).map(|i| format!("v{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(bad(list("l", &refs, "v0")).contains("1 to 200"));
        let mut mixed = list("l", &["a"], "a");
        mixed.values.as_mut().unwrap().push(ListEntry {
            value: json!(2),
            label: String::new(),
        });
        assert!(bad(mixed).contains("all text or all numbers"));
        let mut stray = base("n", ParamType::Number, json!(1));
        stray.max_length = Some(4);
        assert!(bad(stray).contains("text parameter"));
    }

    #[test]
    fn a_value_outside_its_definition_is_refused_naming_the_parameter_only() {
        let mut n = validate_def(&{
            let mut n = base("target", ParamType::Number, json!(10));
            n.min = Some(json!(5));
            n.max = Some(json!(50));
            n
        })
        .unwrap();
        n.id = "p_1".to_owned();
        assert_eq!(check_value(&n, &json!(7)).unwrap(), ParamValue::Number(7.0));
        assert_eq!(
            check_value(&n, &json!("7.5")).unwrap(),
            ParamValue::Number(7.5)
        );
        for (raw, word) in [
            (json!(4), "at least 5"),
            (json!(51), "at most 50"),
            (json!("zz'; DROP"), "must be a number"),
            (json!(null), "must be a number"),
            (json!(true), "must be a number"),
        ] {
            let e = check_value(&n, &raw).unwrap_err();
            assert!(e.contains("'target'") && e.contains(word), "{e}");
            assert!(
                !e.contains("DROP"),
                "the sentence never repeats the value: {e}"
            );
        }
        let d = validate_def(&base("since", ParamType::Date, json!("2026-01-01"))).unwrap();
        assert!(check_value(&d, &json!("2026-02-29")).is_err());
        assert!(check_value(&d, &json!("2024-02-29")).is_ok());
        assert!(check_value(&d, &json!("2026-1-1")).is_err());
        assert!(check_value(&d, &json!(20_260_101)).is_err());
        let t = validate_def(&{
            let mut t = base("who", ParamType::Text, json!("x"));
            t.max_length = Some(5);
            t
        })
        .unwrap();
        assert!(check_value(&t, &json!("abcde")).is_ok());
        assert!(
            check_value(&t, &json!("abcdef"))
                .unwrap_err()
                .contains("longer than 5")
        );
        assert!(
            check_value(&t, &json!("a\u{0}b"))
                .unwrap_err()
                .contains("not allowed")
        );
        let l = validate_def(&list("pick", &["visitors", "rooms"], "rooms")).unwrap();
        assert!(check_value(&l, &json!("visitors")).is_ok());
        assert!(check_value(&l, &json!("visitors ")).is_err());
        assert!(
            check_value(&l, &json!("lat"))
                .unwrap_err()
                .contains("choices")
        );
        let nums = validate_def(&{
            let mut d = base("n", ParamType::List, json!(2));
            d.values = Some(vec![
                ListEntry {
                    value: json!(1),
                    label: String::new(),
                },
                ListEntry {
                    value: json!(2),
                    label: String::new(),
                },
            ]);
            d
        })
        .unwrap();
        assert_eq!(
            check_value(&nums, &json!("2")).unwrap(),
            ParamValue::Number(2.0)
        );
        assert!(check_value(&nums, &json!(3)).is_err());
    }

    #[test]
    fn a_value_prints_as_a_literal_never_as_the_request_text() {
        assert_eq!(ParamValue::Number(1000.0).sql(), "1000");
        assert_eq!(ParamValue::Number(0.25).sql(), "0.25");
        assert_eq!(
            ParamValue::Text("O'Brien\\".to_owned()).sql(),
            "'O''Brien\\\\'"
        );
        assert_eq!(
            ParamValue::Date("2026-03-05".to_owned()).sql(),
            "toDate32('2026-03-05')"
        );
        assert_eq!(
            ParamValue::Date("x'); DROP".to_owned()).sql(),
            "NULL",
            "never the text"
        );
    }

    #[test]
    fn a_request_names_only_known_parameters_and_an_unfit_saved_value_falls_back() {
        let defs = [
            validate_def(&{
                let mut n = base("target", ParamType::Number, json!(10));
                n.max = Some(json!(50));
                n
            })
            .unwrap(),
            validate_def(&list("pick", &["a", "b"], "a")).unwrap(),
        ];
        let req = |v: Value| v.as_object().cloned();
        let set = ParamValues::resolve(&defs, &[], req(json!({ "target": 30 })).as_ref()).unwrap();
        assert_eq!(set.get("target").unwrap().value, ParamValue::Number(30.0));
        assert_eq!(
            set.get("pick").unwrap().value,
            ParamValue::Text("a".to_owned())
        );
        let saved = [
            BoardParam {
                name: "target".to_owned(),
                value: Some(json!(99)),
            },
            BoardParam {
                name: "pick".to_owned(),
                value: Some(json!("b")),
            },
        ];
        let set = ParamValues::resolve(&defs, &saved, None).unwrap();
        assert_eq!(
            set.get("target").unwrap().value,
            ParamValue::Number(10.0),
            "stale saved value -> default"
        );
        assert_eq!(
            set.get("pick").unwrap().value,
            ParamValue::Text("b".to_owned())
        );
        let e =
            ParamValues::resolve(&defs, &saved, req(json!({ "target": 51 })).as_ref()).unwrap_err();
        assert!(e.contains("'target'") && e.contains("at most 50"), "{e}");
        let e =
            ParamValues::resolve(&defs, &[], req(json!({ "evil'; --": 1 })).as_ref()).unwrap_err();
        assert!(!e.contains("evil"), "an unknown name is not echoed: {e}");
        assert!(parse_request(Some("[1]")).is_err() && parse_request(Some("")).unwrap().is_none());
    }

    fn chart(extra: &Value) -> StoredChartSpec {
        let mut def = json!({
            "title": "t", "kind": "bar", "mart": "m",
            "dimension": "provinsi", "measures": ["visitors"], "aggregate": "sum",
            "limit": 20, "order": "desc",
        });
        def.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let def: ChartInput = serde_json::from_value(def).unwrap();
        let spec: ChartSpec = serde_json::from_value(json!({
            "id": "u_1", "title": "t", "kind": def.kind, "mart": "m", "sql": "SELECT 1",
            "x": "provinsi", "y": "visitors",
        }))
        .unwrap();
        StoredChartSpec {
            spec,
            source: ChartSource::Ui,
            board: "default".to_owned(),
            def,
            has_year: false,
            created_by: None,
            created_at: None,
        }
    }

    fn set(defs: &[ParamDef], request: &Value) -> ParamValues {
        ParamValues::resolve(defs, &[], request.as_object()).unwrap()
    }

    #[test]
    fn the_wire_forms_round_trip_and_an_unresolved_slot_is_not_an_identifier() {
        let c = chart(&json!({
            "dimension": { "param": "dim" }, "breakdown": { "param": "brk" },
            "measures": [{ "column": { "param": "m" } }, { "column": { "param": "m2" }, "aggregate": "avg" }, "lat"],
            "limit": { "param": "n" },
            "referenceLines": [{ "value": { "param": "target" }, "label": "Goal" }, { "value": 5 }],
        }));
        assert_eq!(c.def.dimension, "{dim}");
        assert!(
            Ident::new(c.def.dimension.as_str()).is_err(),
            "never an identifier"
        );
        let back = serde_json::to_value(&c.def).unwrap();
        assert_eq!(back["dimension"], json!({ "param": "dim" }));
        assert_eq!(back["breakdown"], json!({ "param": "brk" }));
        assert_eq!(back["measures"][0], json!({ "column": { "param": "m" } }));
        assert_eq!(
            back["measures"][1],
            json!({ "column": { "param": "m2" }, "aggregate": "avg" })
        );
        assert_eq!(back["measures"][2], json!("lat"));
        assert_eq!(back["limit"], json!({ "param": "n" }));
        assert_eq!(
            back["referenceLines"][1],
            json!({ "value": 5.0 })
                .as_object()
                .map(|_| json!({ "value": 5.0 }))
                .unwrap()
        );
        assert_eq!(
            chart_param_names(&c.def).into_iter().collect::<Vec<_>>(),
            ["brk", "dim", "m", "m2", "n", "target"]
        );
        // A plain chart round-trips exactly as before.
        let plain = serde_json::to_value(&chart(&json!({})).def).unwrap();
        assert_eq!(plain["dimension"], json!("provinsi"));
        assert_eq!(plain["limit"], json!(20));
        assert!(plain.get("referenceLines").is_none());
        assert!(
            serde_json::from_value::<ChartInput>(
                json!({ "title": "t", "kind": "bar", "dimension": { "param": "a b" } })
            )
            .is_err()
        );
    }

    #[test]
    fn a_column_switch_takes_only_a_listed_value_that_is_also_a_column_of_the_source() {
        let defs = [validate_def(&list(
            "pick",
            &["visitors", "lat", "evil col", "ghost"],
            "visitors",
        ))
        .unwrap()];
        let c = chart(&json!({ "measures": [{ "column": { "param": "pick" } }] }));
        let known = |n: &str| ["visitors", "lat", "provinsi"].contains(&n);
        let ok = resolve_chart(&c, &set(&defs, &json!({ "pick": "lat" })), &known).unwrap();
        assert_eq!(ok.def.measures.to_vec(), ["lat"]);
        assert_eq!(ok.spec.y, crate::specs::ChartY::Single("lat".to_owned()));
        assert!(ok.def.shape.resolved_params, "the builder rebuilds it");
        // In the list, not a column of the source.
        let e = resolve_chart(&c, &set(&defs, &json!({ "pick": "ghost" })), &known).unwrap_err();
        assert!(e.contains("not a column"), "{e}");
        // In the list, not an identifier.
        assert!(resolve_chart(&c, &set(&defs, &json!({ "pick": "evil col" })), &known).is_err());
        // Not in the list: refused when the request is resolved, never reaching a chart.
        let e = ParamValues::resolve(&defs, &[], json!({ "pick": "serving.mart_x" }).as_object())
            .unwrap_err();
        assert!(e.contains("choices"));
        assert!(
            ParamValues::resolve(
                &defs,
                &[],
                json!({ "pick": "lat; DROP TABLE x" }).as_object()
            )
            .is_err()
        );
        // A parameter that is not a list of text.
        let numeric = [validate_def(&base("pick", ParamType::Number, json!(1))).unwrap()];
        assert!(resolve_chart(&c, &set(&numeric, &json!({})), &known).is_err());
        // A missing parameter.
        assert!(
            resolve_chart(&c, &ParamValues::default(), &known)
                .unwrap_err()
                .contains("does not exist")
        );
        // A chart that names none is returned untouched.
        let plain = chart(&json!({}));
        assert_eq!(
            resolve_chart(&plain, &ParamValues::default(), &known).unwrap(),
            plain
        );
    }

    #[test]
    fn a_limit_parameter_must_be_a_whole_number_within_the_kinds_range() {
        let defs = [validate_def(&base("n", ParamType::Number, json!(5))).unwrap()];
        let c = chart(&json!({ "limit": { "param": "n" } }));
        let always = |_: &str| true;
        for (value, expect) in [
            (5.0, Some(5)),
            (100.0, Some(100)),
            (1.0, Some(1)),
            (0.0, None),
            (101.0, None),
            (2.5, None),
            (-3.0, None),
        ] {
            let resolved = resolve_chart(&c, &set(&defs, &json!({ "n": value })), &always);
            match expect {
                Some(n) => assert_eq!(resolved.unwrap().def.limit_value(), Some(n)),
                None => assert!(resolved.unwrap_err().contains("whole number from 1 to 100")),
            }
        }
        let calendar = chart(&json!({ "kind": "calendar", "limit": { "param": "n" } }));
        assert_eq!(
            resolve_chart(&calendar, &set(&defs, &json!({ "n": 366 })), &always)
                .unwrap()
                .def
                .limit_value(),
            Some(366)
        );
        let grained = chart(&json!({ "grain": "day", "limit": { "param": "n" } }));
        assert_eq!(
            resolve_chart(&grained, &set(&defs, &json!({ "n": 1000 })), &always)
                .unwrap()
                .def
                .limit_value(),
            Some(1000)
        );
    }

    #[test]
    fn reference_lines_resolve_their_parameter_and_drop_one_that_is_not_a_number() {
        let defs = [
            validate_def(&base("target", ParamType::Number, json!(20000))).unwrap(),
            validate_def(&list("pick", &["a"], "a")).unwrap(),
        ];
        let c = chart(&json!({ "referenceLines": [
            { "value": { "param": "target" }, "label": "Goal" },
            { "value": 7.5 },
            { "value": { "param": "pick" } },
            { "value": { "param": "gone" } },
        ]}));
        let lines = reference_lines(&c.def, &set(&defs, &json!({ "target": 250 })));
        assert_eq!(
            lines,
            vec![
                ResolvedLine {
                    value: 250.0,
                    label: Some("Goal".to_owned())
                },
                ResolvedLine {
                    value: 7.5,
                    label: None
                },
            ]
        );
    }

    #[test]
    fn chart_shape_rules_name_the_kinds_that_take_each_form() {
        let lines = json!({ "referenceLines": [{ "value": 1 }] });
        assert!(check_chart_shape(&chart(&lines).def).is_ok());
        assert!(
            check_chart_shape(
                &chart(&json!({ "kind": "pie", "referenceLines": [{ "value": 1 }] })).def
            )
            .is_err()
        );
        let six: Vec<Value> = (0..6).map(|i| json!({ "value": i })).collect();
        assert!(
            check_chart_shape(&chart(&json!({ "referenceLines": six })).def)
                .unwrap_err()
                .contains("at most 5")
        );
        assert!(
            check_chart_shape(
                &chart(&json!({ "referenceLines": [{ "value": 1, "label": "a\nb" }] })).def
            )
            .is_err()
        );
        assert!(
            check_chart_shape(
                &chart(
                    &json!({ "dimension": { "param": "p" }, "kind": "table", "tableMode": "rows" })
                )
                .def
            )
            .is_err()
        );
        assert!(
            check_chart_shape(
                &chart(
                    &json!({ "limit": { "param": "n" }, "groupings": [{ "column": "provinsi" }] })
                )
                .def
            )
            .is_err()
        );
        assert!(check_chart_shape(&chart(&json!({ "limit": { "param": "n" } })).def).is_ok());
        assert!(check_chart_shape(&chart(&json!({ "dimension": { "param": "p" } })).def).is_ok());
    }

    #[test]
    fn deleting_a_parameter_finds_the_charts_that_name_it() {
        let a = chart(&json!({ "limit": { "param": "n" } }));
        let mut b = chart(&json!({}));
        b.spec.id = "u_2".to_owned();
        assert_eq!(charts_using(&[a.clone(), b], "n"), ["u_1"]);
        assert!(charts_using(&[a], "other").is_empty());
        let _ = ColumnKind::Text;
    }
}
