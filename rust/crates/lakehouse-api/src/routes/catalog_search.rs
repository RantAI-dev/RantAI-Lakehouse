//! The one matcher behind catalog search: `GET /api/catalog?q=` and
//! `GET /api/catalog/query?search=` both rank through [`search`].
//!
//! # Why there is one
//!
//! `DATA-11` F1: the two routes used to carry two matchers
//! (`filter_assets_by_query` and `apply_search`) over different fields, so a
//! tag found a table in the ⌘K box and not in the Data Explorer, and an
//! owner did the reverse. A second copy of a guard is a finding (rule 4);
//! this module replaces both.
//!
//! # Why this is pure
//!
//! Like [`super::catalog_query`], it works on assets that are already
//! assembled and takes the column names and the use counts as plain maps,
//! never a `ChClient` or a Postgres pool. The weight table, the one-edit
//! rule and the ordering are therefore all reachable from unit tests, and
//! the cached copy that feeds it ([`super::catalog`]'s search snapshot) can
//! be tested apart from the ranking.
//!
//! # How a term is read
//!
//! The term is lower-cased and split on whitespace; at most [`MAX_WORDS`]
//! words are used. Every word must match the asset somewhere, in any order.
//! A word matches a field when the field contains it (exact) or, for a word
//! of [`MIN_APPROX_CHARS`] or more characters, when one token of the field
//! is one edit away from it (approximate; `DATA-11` D3). A typed word with
//! punctuation (`custmer_id`) is split like a field, and each part must
//! match (`DATA-11` review `SHOULD-FIX 1`). The best field per
//! word counts, and an asset's score is the sum over its words.
//!
//! The reason an asset is shown (`matchedOn`) comes from the first word, in
//! typed order, whose best field is not the name (`DATA-11` review
//! `SHOULD-FIX 8`): `northwind customer` finds `Northwind Orders` through
//! its column `customer_id`, and the first word alone (the name) would leave
//! the reader with no reason. When every word's best field is the name there
//! is no `matchedOn`: the name is already on screen.
//!
//! `DATA-32` D1 is the one exception to that last rule. When no field
//! explains the hit and a word matched the name only approximately (`revnue`
//! for `Revenue`), the name is the reason, with `approximate: true`: a
//! reader who typed a typo must be told that a typo was forgiven, or the
//! result looks like a false positive. An exact name match still carries no
//! reason.
//!
//! # The prepared index (`DATA-41` D2)
//!
//! The index ([`SearchIndex`]) is built once, with the search copy
//! (`DATA-11` D2), and holds every asset's candidates already lower-cased
//! and tokenised. A search reads it and allocates nothing per candidate.
//!
//! Before this, every search rebuilt the candidates of every asset: about
//! 450,000 lower-cased strings and a `Vec<char>` per token on a catalog of
//! 10,000 tables, which was most of the time a search took
//! (`docs/plans/DATA-11-RESULT.md`, 2026-11-01). The matching rules are
//! unchanged — the tests below are the proof — and the same text on many
//! assets (a column name on thousands of tables) is prepared once and
//! shared, so the index costs memory in proportion to the catalog's
//! distinct texts, not to its assets.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

/// Words past this many are ignored: a pasted sentence must not turn one
/// search into dozens of passes over every column of the catalog.
const MAX_WORDS: usize = 8;

/// A word shorter than this never matches approximately (`DATA-11` D3):
/// one edit on three letters reaches too many unrelated words.
const MIN_APPROX_CHARS: usize = 4;

/// Weight of a word that equals the whole name.
const SCORE_NAME_WHOLE: u32 = 120;
/// Exact / approximate weight of a hit in the name.
const SCORE_NAME: (u32, u32) = (100, 50);
/// Exact / approximate weight of a hit in the id or the namespace.
const SCORE_ID_NAMESPACE: (u32, u32) = (80, 40);
/// Exact / approximate weight of a hit in a tag.
const SCORE_TAG: (u32, u32) = (70, 35);
/// Exact / approximate weight of a hit in a column name.
const SCORE_COLUMN: (u32, u32) = (60, 30);
/// Exact / approximate weight of a hit in the owner.
const SCORE_OWNER: (u32, u32) = (40, 20);
/// Exact / approximate weight of a hit in the description.
const SCORE_DESCRIPTION: (u32, u32) = (30, 15);
/// Exact / approximate weight of a hit in a column description.
const SCORE_COLUMN_DESCRIPTION: (u32, u32) = (20, 10);

/// Column names and descriptions per asset id, as the search copy holds
/// them.
pub type ColumnIndex = HashMap<String, Vec<(String, String)>>;

/// Queries per asset id over the use window; an asset not in the map has 0.
pub type UsageIndex = HashMap<String, u32>;

/// Where a word was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Id,
    Namespace,
    Tag,
    Column,
    Owner,
    Description,
    ColumnDescription,
}

impl Field {
    /// `(exact, approximate)` weights of the section-3 table.
    const fn weights(self) -> (u32, u32) {
        match self {
            Self::Name => SCORE_NAME,
            Self::Id | Self::Namespace => SCORE_ID_NAMESPACE,
            Self::Tag => SCORE_TAG,
            Self::Column => SCORE_COLUMN,
            Self::Owner => SCORE_OWNER,
            Self::Description => SCORE_DESCRIPTION,
            Self::ColumnDescription => SCORE_COLUMN_DESCRIPTION,
        }
    }

    /// The `matchedOn.field` spelling; `None` for the name, which is not
    /// reported as a reason of its own because the name is already on
    /// screen. A name that forgave a typo is reported by the search itself,
    /// as `"name"` (`DATA-32` D1).
    const fn reported(self) -> Option<&'static str> {
        match self {
            Self::Name => None,
            Self::Id => Some("id"),
            Self::Namespace => Some("namespace"),
            Self::Tag => Some("tag"),
            Self::Column => Some("column"),
            Self::Owner => Some("owner"),
            Self::Description => Some("description"),
            Self::ColumnDescription => Some("columnDescription"),
        }
    }
}

/// One text, prepared once (`DATA-41` D2): its lower-cased form and its
/// tokens (the runs of letters and digits).
///
/// The tokens are what the approximate match compares against, so they are
/// computed here rather than per candidate per search.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedText {
    /// The text, lower-cased: what a word is looked up in.
    lower: String,
    /// The runs of letters and digits in `lower`.
    tokens: Vec<Vec<char>>,
}

/// One piece of text of one asset a word can match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    field: Field,
    /// The text, prepared: lower-cased and tokenised (`DATA-41` D2).
    text: Arc<PreparedText>,
    /// What `matchedOn.value` shows: the tag or the column name, empty for
    /// the rest.
    shown: String,
}

/// The texts one index build prepared, keyed by their lower-cased form.
///
/// The same column name is on thousands of tables and the same word is in
/// thousands of descriptions, so a text is prepared once and shared
/// (`DATA-41` D2): the index costs memory in proportion to the catalog's
/// distinct texts, not to its assets.
type PreparedTexts = HashMap<String, Arc<PreparedText>>;

/// The lower-cased, tokenised form of `text`, prepared once per build.
fn prepare(text: &str, prepared: &mut PreparedTexts) -> Arc<PreparedText> {
    let lower = text.to_lowercase();
    if let Some(found) = prepared.get(&lower) {
        return Arc::clone(found);
    }
    let tokens = tokens(&lower).map(|t| t.chars().collect()).collect();
    let prepared_text = Arc::new(PreparedText { lower, tokens });
    prepared.insert(prepared_text.lower.clone(), Arc::clone(&prepared_text));
    prepared_text
}

/// One asset's candidates, in the order [`best_hit`] prefers them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    /// The asset's id: the usage lookup and the tie-break of the ranking.
    id: String,
    /// Every text of this asset a word can match.
    candidates: Vec<Candidate>,
}

/// The prepared search index (`DATA-41` D2): the assets of the search copy
/// and, per asset, the texts a word can match, already lower-cased and
/// tokenised.
///
/// Built once, with the copy ([`crate::catalog_search_cache`]), and read by
/// every search. The matching rules are the ones this module has always
/// had; only where the work happens changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchIndex {
    /// The asset rows, in the order the copy holds them.
    assets: Vec<Value>,
    /// Per asset: its id and its candidates, in the same order.
    entries: Vec<IndexEntry>,
}

impl SearchIndex {
    /// Prepares the index over `assets`, which it takes over.
    ///
    /// `columns` are the column names and descriptions per asset id, as
    /// the search copy holds them.
    #[must_use]
    pub fn build(assets: Vec<Value>, columns: &ColumnIndex) -> Self {
        let mut prepared: PreparedTexts = HashMap::new();
        let entries = assets
            .iter()
            .map(|asset| IndexEntry {
                id: text_of(asset, "id").to_owned(),
                candidates: candidates(asset, columns, &mut prepared),
            })
            .collect();
        Self { assets, entries }
    }

    /// The asset rows, in the order the copy holds them.
    #[must_use]
    pub fn assets(&self) -> &[Value] {
        &self.assets
    }
}

/// The candidates of one asset, in the order [`best_hit`] prefers them.
///
/// The order is the one this module has always used: name, id, namespace,
/// owner, description, the tags, the governed pairs, then the columns
/// (each name, then its description). Of equal hits the first in this order
/// stays, so the order is part of the answer.
fn candidates(
    asset: &Value,
    columns: &ColumnIndex,
    prepared: &mut PreparedTexts,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (field, key) in [
        (Field::Name, "name"),
        (Field::Id, "id"),
        (Field::Namespace, "namespace"),
        (Field::Owner, "owner"),
        (Field::Description, "description"),
    ] {
        let text = text_of(asset, key);
        if !text.is_empty() {
            out.push(Candidate {
                field,
                text: prepare(text, prepared),
                shown: String::new(),
            });
        }
    }
    if let Some(tags) = asset.get("tags").and_then(Value::as_array) {
        for tag in tags.iter().filter_map(Value::as_str) {
            out.push(Candidate {
                field: Field::Tag,
                text: prepare(tag, prepared),
                shown: tag.to_owned(),
            });
        }
    }
    // `DATA-12` part 2: a governed pair is indexed as the text `key: value`,
    // so a word finds it by the key, the value or the pair, and the reason
    // line reads `data_domain: finance`. It is a tag for ranking and for
    // `matchedOn.field`, which stays `"tag"`.
    for (key, value) in governed_pairs(asset) {
        let pair = format!("{key}: {value}");
        out.push(Candidate {
            field: Field::Tag,
            text: prepare(&pair, prepared),
            shown: pair,
        });
    }
    if let Some(cols) = columns.get(text_of(asset, "id")) {
        for (name, description) in cols {
            out.push(Candidate {
                field: Field::Column,
                text: prepare(name, prepared),
                shown: name.clone(),
            });
            if !description.is_empty() {
                out.push(Candidate {
                    field: Field::ColumnDescription,
                    text: prepare(description, prepared),
                    shown: String::new(),
                });
            }
        }
    }
    out
}

/// The best field one word found.
struct Hit {
    score: u32,
    field: Field,
    shown: String,
    approximate: bool,
}

/// Why an asset matched, as the caller is told.
enum Reason {
    /// A field that explains the hit (`DATA-11`): the reader sees which
    /// field matched and whether a typo was forgiven.
    Field(Hit),
    /// The name forgave a typo and no field explains the hit (`DATA-32` D1):
    /// the name is on screen already, so the reader is told only that the
    /// match was approximate.
    NameTypo,
}

/// The `(key, value)` pairs of an asset's `governedTags` object
/// (`DATA-12` part 2); empty for an asset without it.
pub fn governed_pairs(asset: &Value) -> Vec<(&str, &str)> {
    asset
        .get("governedTags")
        .and_then(Value::as_object)
        .map(|tags| {
            tags.iter()
                .filter_map(|(k, v)| Some((k.as_str(), v.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn text_of<'a>(asset: &'a Value, key: &str) -> &'a str {
    asset.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The field's tokens: runs of letters and digits.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// Whether `a` and `b` differ by at most one substitution, insertion,
/// deletion or swap of two adjacent characters. A single pass, not a
/// distance matrix: the answer is only ever "one edit or not".
fn within_one_edit(a: &[char], b: &[char]) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let Some(first) = short.iter().zip(long).position(|(x, y)| x != y) else {
        // One is a prefix of the other: equal, or one trailing character.
        return true;
    };
    if long.len() > short.len() {
        // An insertion in `long`: the rest must line up after skipping it.
        return short[first..] == long[first + 1..];
    }
    // Same length: one substitution, or two adjacent characters swapped.
    if short[first + 1..] == long[first + 1..] {
        return true;
    }
    first + 1 < short.len()
        && short[first] == long[first + 1]
        && short[first + 1] == long[first]
        && short[first + 2..] == long[first + 2..]
}

/// One word of the term, prepared once per search: the word itself and its
/// parts (the runs of letters and digits), each with its characters.
///
/// The characters are prepared here so the approximate match allocates
/// nothing per candidate (`DATA-41` D2).
struct Word<'a> {
    /// The word, lower-cased: what the fields are looked up in.
    text: &'a str,
    /// The word's parts, each with its characters.
    parts: Vec<WordPart<'a>>,
}

/// One part of a word: its text and its characters.
struct WordPart<'a> {
    text: &'a str,
    chars: Vec<char>,
}

impl<'a> Word<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            parts: tokens(text)
                .map(|t| WordPart {
                    text: t,
                    chars: t.chars().collect(),
                })
                .collect(),
        }
    }
}

/// Whether `word`, not contained in `text` as typed, matches it
/// approximately.
///
/// `DATA-11` review `SHOULD-FIX 1`: the typed word is split the way fields are
/// (`tokens`), because a field's tokens never hold punctuation, so
/// `custmer_id` could never be one edit from a token of `customer_id`. Every
/// token of the word must be contained in the field or, at
/// `MIN_APPROX_CHARS` characters or more, one edit from one of its tokens. A
/// word with no letter or digit has no token and matches nothing.
///
/// `DATA-11` review `SHOULD-FIX 5`: the answer says whether any part needed an
/// edit. `None` is no match; `Some(false)` is every part contained in the
/// field (`id_customer` in `customer_id`: right letters, wrong order), which
/// is not a typo and must not be shown as one; `Some(true)` is a typo.
///
/// `DATA-41` D2: the field's tokens are already prepared in the index, so
/// this allocates nothing.
fn approximately_in(word: &Word, text: &PreparedText) -> Option<bool> {
    let mut any = false;
    let mut edited = false;
    for part in &word.parts {
        any = true;
        if text.lower.contains(part.text) {
            continue;
        }
        if part.chars.len() < MIN_APPROX_CHARS
            || !text.tokens.iter().any(|t| within_one_edit(t, &part.chars))
        {
            return None;
        }
        edited = true;
    }
    any.then_some(edited)
}

fn best_hit(word: &Word, candidates: &[Candidate]) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for c in candidates {
        let (score, approximate) = if c.text.lower.contains(word.text) {
            let whole = c.field == Field::Name && c.text.lower == word.text;
            (
                if whole {
                    SCORE_NAME_WHOLE
                } else {
                    c.field.weights().0
                },
                false,
            )
        } else if let Some(edited) = approximately_in(word, &c.text) {
            // The approximate weight either way (the order was wrong or a
            // letter was); only a letter makes the hit "approximate".
            (c.field.weights().1, edited)
        } else {
            continue;
        };
        // Strictly greater: of equal hits the first in table order stays.
        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(Hit {
                score,
                field: c.field,
                shown: c.shown.clone(),
                approximate,
            });
        }
    }
    best
}

/// Rank `assets` for `term`; see the module doc for how the term is read.
///
/// An empty or all-whitespace term returns every asset in input order with
/// no `matchedOn`. Otherwise only assets matching every word are returned,
/// ordered by score descending, then use descending, then `id` ascending,
/// each with `matchedOn` set for the best field of the first word, in typed
/// order, whose best field is not the name (omitted when every word's best
/// field is the name; `DATA-11` review `SHOULD-FIX 8`). `usage` is
/// `DATA-11` D4.
///
/// `DATA-32` D1: when no field explains the hit and a word matched the name
/// only approximately, `matchedOn` is the name with `approximate: true`.
///
/// `DATA-41` D2: the index is the prepared one, so a search allocates
/// nothing per candidate.
pub fn search(index: &SearchIndex, usage: &UsageIndex, term: &str) -> Vec<Value> {
    let lowered = term.to_lowercase();
    let words: Vec<Word> = lowered
        .split_whitespace()
        .take(MAX_WORDS)
        .map(Word::new)
        .collect();
    if words.is_empty() {
        return index.assets.clone();
    }

    // `(score, use, id, position in the index, reason)`: the id is the
    // tie-break of the ranking, the position is where the asset is.
    let mut ranked: Vec<(u32, u32, &str, usize, Option<Reason>)> = Vec::new();
    for (position, entry) in index.entries.iter().enumerate() {
        let mut total = 0_u32;
        let mut reason = None;
        let mut name_typo = false;
        let mut all = true;
        for word in &words {
            let Some(hit) = best_hit(word, &entry.candidates) else {
                all = false;
                break;
            };
            total += hit.score;
            // `DATA-11` review `SHOULD-FIX 8`: not the first word as such but
            // the first one the name does not already explain.
            if reason.is_none() && hit.field.reported().is_some() {
                reason = Some(Reason::Field(hit));
            } else if !name_typo && hit.field == Field::Name && hit.approximate {
                // `DATA-32` D1: the name forgave a typo. It is the reason
                // only when no field explains the hit, so it is kept aside;
                // the name has no reported field, so the branch above never
                // takes it.
                name_typo = true;
            }
        }
        if all {
            let used = usage.get(&entry.id).copied().unwrap_or(0);
            // `DATA-32` D1: a field that explains the hit wins; the name that
            // forgave a typo is the reason only when there is none.
            let reason = reason.or(name_typo.then_some(Reason::NameTypo));
            ranked.push((total, used, entry.id.as_str(), position, reason));
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(b.2)));
    ranked
        .into_iter()
        .map(|(_, _, _, position, reason)| {
            let mut asset = index.assets[position].clone();
            let reported = reason.and_then(|r| match r {
                Reason::Field(hit) => hit
                    .field
                    .reported()
                    .map(|f| (f, hit.shown, hit.approximate)),
                // `DATA-32` D1: the name forgave a typo and nothing else
                // explains the hit, so the name is the reason.
                Reason::NameTypo => Some(("name", String::new(), true)),
            });
            if let (Some((field, value, approximate)), Some(o)) = (reported, asset.as_object_mut())
            {
                o.insert(
                    "matchedOn".to_owned(),
                    json!({ "field": field, "value": value, "approximate": approximate }),
                );
            }
            asset
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn ids(assets: &[Value]) -> Vec<&str> {
        assets.iter().filter_map(|a| a["id"].as_str()).collect()
    }

    /// The prepared index of `assets` and `columns` (`DATA-41` D2): what a
    /// search reads.
    fn index(assets: &[Value], columns: &ColumnIndex) -> SearchIndex {
        SearchIndex::build(assets.to_vec(), columns)
    }

    fn find(assets: &[Value], term: &str) -> Vec<Value> {
        search(
            &index(assets, &ColumnIndex::new()),
            &UsageIndex::new(),
            term,
        )
    }

    fn orders_and_users() -> Vec<Value> {
        vec![
            json!({ "id": "bronze.orders", "name": "Orders", "description": "Order events" }),
            json!({ "id": "bronze.users", "name": "Users", "description": "Signup ledger" }),
        ]
    }

    /// The four rows the Data Explorer's `search_*` tests used.
    fn explorer_fixture() -> Vec<Value> {
        vec![
            json!({
                "id": "silver.mart_wisman", "name": "Mart Wisman",
                "namespace": "silver", "owner": "Dinas Pariwisata",
                "description": "Kunjungan wisatawan mancanegara",
            }),
            json!({
                "id": "bronze.event_2026", "name": "Event 2026",
                "namespace": "sdi-primer", "owner": "",
                "description": "Jumlah pengunjung event",
            }),
            json!({
                "id": "silver.dim_negara", "name": "dim negara",
                "namespace": "silver", "description": "Dimensi negara",
            }),
            json!({
                "id": "gold.restoran", "name": "Restoran",
                "namespace": "serving", "owner": "Dinas Ekraf",
                "description": "Agregat restoran",
            }),
        ]
    }

    /// `DATA-12` part 2: a governed tag is found by its value alone, its key
    /// alone and the whole pair, and the reason line shows the pair.
    #[test]
    fn a_governed_tag_finds_its_table_by_value_key_or_pair() {
        let mut assets = orders_and_users();
        assets[1]["governedTags"] = json!({ "data_domain": "finance" });
        for term in ["finance", "data_domain", "data_domain: finance"] {
            let hits = find(&assets, term);
            assert_eq!(ids(&hits), vec!["bronze.users"], "{term}");
            assert_eq!(
                hits[0]["matchedOn"],
                json!({ "field": "tag", "value": "data_domain: finance", "approximate": false }),
                "{term}"
            );
        }
        assert_eq!(find(&assets, "revenue"), [] as [serde_json::Value; 0]);
    }

    // Carried over from `filter_assets_by_query_*` in `catalog.rs`
    // (DATA-11 F1).
    #[test]
    fn a_term_matches_name_description_or_id_case_insensitively() {
        let assets = orders_and_users();
        assert_eq!(ids(&find(&assets, "ORDER")), vec!["bronze.orders"]);
        assert_eq!(ids(&find(&assets, "bronze.users")), vec!["bronze.users"]);
    }

    // Carried over from `filter_assets_by_query_matches_via_an_annotation_tag`.
    // The old matcher took the annotation rows as a second argument; the
    // search copy now has `apply_annotation` put the tags on the asset
    // itself, so the tag is read from the asset (DATA-11 F1).
    #[test]
    fn a_tag_finds_its_table() {
        let mut assets = orders_and_users();
        assets[1]["tags"] = json!(["pii"]);
        let hits = find(&assets, "pii");
        assert_eq!(ids(&hits), vec!["bronze.users"]);
        assert_eq!(
            hits[0]["matchedOn"],
            json!({ "field": "tag", "value": "pii", "approximate": false })
        );
    }

    // Carried over from `filter_assets_by_query_empty_q_returns_everything`
    // and `blank_search_returns_everything`.
    #[test]
    fn an_empty_or_blank_term_returns_every_asset_in_input_order_without_a_reason() {
        let assets = explorer_fixture();
        for term in ["", "   ", "\t \n"] {
            let hits = find(&assets, term);
            assert_eq!(ids(&hits), ids(&assets));
            assert!(hits.iter().all(|h| h.get("matchedOn").is_none()));
        }
    }

    // Carried over from `filter_assets_by_query_matches_non_ascii_case_unicode_aware`.
    #[test]
    fn a_non_ascii_term_folds_case_like_the_browser_does() {
        let assets = vec![json!({
            "id": "bronze.orders", "name": "Orders", "description": "Umsatz nach Übersee"
        })];
        assert_eq!(find(&assets, "übersee").len(), 1);
        assert_eq!(find(&assets, "ÜBERSEE").len(), 1);
    }

    // Carried over from `search_is_case_insensitive_and_spans_several_fields`.
    #[test]
    fn a_term_spans_name_description_and_owner_whatever_its_case() {
        let assets = explorer_fixture();
        assert_eq!(ids(&find(&assets, "WISMAN")), vec!["silver.mart_wisman"]);
        assert_eq!(ids(&find(&assets, "restoran")), vec!["gold.restoran"]);
        assert_eq!(ids(&find(&assets, "kunjungan")), vec!["silver.mart_wisman"]);
        assert_eq!(
            ids(&find(&assets, "pariwisata")),
            vec!["silver.mart_wisman"]
        );
    }

    // Carried over from `search_matches_on_id_as_well_as_name`. The answer
    // changes in order only: the old matcher kept input order
    // (`mart_wisman`, `dim_negara`); a ranked result orders equal scores by
    // id, so `dim_negara` comes first (DATA-11 F4).
    #[test]
    fn a_namespace_qualified_term_finds_rows_by_id_and_equal_scores_order_by_id() {
        let hits = find(&explorer_fixture(), "silver.");
        assert_eq!(ids(&hits), vec!["silver.dim_negara", "silver.mart_wisman"]);
    }

    // Carried over from `search_does_not_match_unlisted_fields`: the layer
    // and the tier are not searched.
    #[test]
    fn the_layer_and_the_tier_are_not_searched() {
        let mut assets = explorer_fixture();
        assets[2]["tier"] = json!("bronze");
        assets[2]["layer"] = json!("Bronze");
        assert_eq!(ids(&find(&assets, "bronze")), vec!["bronze.event_2026"]);
    }

    // The one intended change from the old matcher: it searched the term as
    // one phrase, so "negara dimensi" found nothing.
    #[test]
    fn two_words_match_in_the_opposite_order_of_the_description() {
        let assets = explorer_fixture();
        assert_eq!(
            ids(&find(&assets, "negara dimensi")),
            vec!["silver.dim_negara"]
        );
        assert_eq!(
            ids(&find(&assets, "mancanegara kunjungan")),
            vec!["silver.mart_wisman"]
        );
    }

    #[test]
    fn a_word_that_matches_nothing_excludes_the_asset_even_when_another_word_matches() {
        let assets = explorer_fixture();
        assert_eq!(
            find(&assets, "restoran zzzzzz"),
            [] as [serde_json::Value; 0]
        );
        assert_eq!(
            ids(&find(&assets, "restoran agregat")),
            vec!["gold.restoran"]
        );
    }

    fn revenue() -> Vec<Value> {
        vec![json!({ "id": "silver.rev", "name": "Rev", "description": "revenue by month" })]
    }

    #[test]
    fn one_edit_finds_a_word_by_deletion_swap_substitution_and_insertion() {
        let assets = revenue();
        // Deletion in the typed word.
        assert_eq!(find(&assets, "revnue").len(), 1);
        // Swap of two adjacent characters.
        assert_eq!(find(&assets, "revneue").len(), 1);
        // Substitution.
        assert_eq!(find(&assets, "revenua").len(), 1);
        // Insertion in the typed word.
        assert_eq!(find(&assets, "reveenue").len(), 1);
        let hit = &find(&assets, "revnue")[0];
        assert_eq!(
            hit["matchedOn"],
            json!({ "field": "description", "value": "", "approximate": true })
        );
    }

    #[test]
    fn a_short_word_or_two_edits_do_not_match_approximately() {
        let assets = revenue();
        // Under 4 characters, and `rev` is the name besides: the typed word
        // must not be matched approximately against "rev".
        assert_eq!(find(&assets, "rvn"), [] as [serde_json::Value; 0]);
        assert_eq!(find(&assets, "ravanue"), [] as [serde_json::Value; 0]);
    }

    #[test]
    fn a_typo_in_a_word_with_punctuation_finds_the_column_or_table() {
        // DATA-11 review SHOULD-FIX 1: the word is split like the field.
        let assets = vec![
            json!({ "id": "silver.orders", "name": "Orders" }),
            json!({ "id": "silver.other", "name": "Other" }),
        ];
        let columns: ColumnIndex = [(
            "silver.other".to_owned(),
            vec![("customer_id".to_owned(), String::new())],
        )]
        .into();
        let hit = search(&index(&assets, &columns), &UsageIndex::new(), "custmer_id");
        assert_eq!(ids(&hit), ["silver.other"]);
        assert_eq!(
            hit[0]["matchedOn"],
            json!({ "field": "column", "value": "customer_id", "approximate": true })
        );
        assert_eq!(find(&assets, "silver.ordrs").len(), 1);
        // `xx` is under 4 characters and is not in the column: no match.
        assert_eq!(
            search(&index(&assets, &columns), &UsageIndex::new(), "custmer_xx"),
            [] as [serde_json::Value; 0]
        );
        // No letter or digit: nothing to match.
        assert_eq!(find(&assets, "___"), [] as [serde_json::Value; 0]);
    }

    #[test]
    fn a_word_whose_parts_all_match_exactly_is_not_marked_approximate() {
        // DATA-11 review SHOULD-FIX 5: `id_customer` has no wrong letter in
        // `customer_id`; the order is wrong, so the hit has the approximate
        // weight (below the table whose column is `id_customer`) but is not
        // flagged as a typo.
        let assets = vec![
            json!({ "id": "silver.swapped", "name": "Swapped" }),
            json!({ "id": "silver.exact", "name": "Exact" }),
        ];
        let columns: ColumnIndex = [
            (
                "silver.swapped".to_owned(),
                vec![("customer_id".to_owned(), String::new())],
            ),
            (
                "silver.exact".to_owned(),
                vec![("id_customer".to_owned(), String::new())],
            ),
        ]
        .into();
        let hits = search(&index(&assets, &columns), &UsageIndex::new(), "id_customer");
        assert_eq!(ids(&hits), ["silver.exact", "silver.swapped"]);
        assert_eq!(
            hits[1]["matchedOn"],
            json!({ "field": "column", "value": "customer_id", "approximate": false })
        );
        // A real typo is still flagged.
        let typo = search(&index(&assets, &columns), &UsageIndex::new(), "custmer_id");
        assert!(
            typo.iter()
                .all(|h| h["matchedOn"]["approximate"] == json!(true))
        );
        assert_ne!(typo, [] as [serde_json::Value; 0]);
    }

    #[test]
    fn a_column_name_finds_its_table_and_says_which_column() {
        let assets = vec![
            json!({ "id": "silver.sales", "name": "Sales" }),
            json!({ "id": "silver.hr", "name": "Hr" }),
        ];
        let columns: ColumnIndex = HashMap::from([(
            "silver.sales".to_owned(),
            vec![("revenue_amount".to_owned(), String::new())],
        )]);
        let hits = search(
            &index(&assets, &columns),
            &UsageIndex::new(),
            "revenue_amount",
        );
        assert_eq!(ids(&hits), vec!["silver.sales"]);
        assert_eq!(
            hits[0]["matchedOn"],
            json!({ "field": "column", "value": "revenue_amount", "approximate": false })
        );
    }

    #[test]
    fn an_owner_a_namespace_and_a_column_description_each_find_their_table() {
        let assets = vec![
            json!({ "id": "a.one", "name": "One", "owner": "Finance Team" }),
            json!({ "id": "b.two", "name": "Two", "namespace": "sdi-primer" }),
            json!({ "id": "c.three", "name": "Three" }),
        ];
        let columns: ColumnIndex = HashMap::from([(
            "c.three".to_owned(),
            vec![("c1".to_owned(), "Amount invoiced".to_owned())],
        )]);
        let by = |term: &str| search(&index(&assets, &columns), &UsageIndex::new(), term);
        let owner = by("finance");
        assert_eq!(ids(&owner), vec!["a.one"]);
        assert_eq!(owner[0]["matchedOn"]["field"], "owner");
        let namespace = by("sdi-primer");
        assert_eq!(ids(&namespace), vec!["b.two"]);
        assert_eq!(namespace[0]["matchedOn"]["field"], "namespace");
        let described = by("invoiced");
        assert_eq!(ids(&described), vec!["c.three"]);
        assert_eq!(
            described[0]["matchedOn"],
            json!({ "field": "columnDescription", "value": "", "approximate": false })
        );
    }

    #[test]
    fn a_name_beats_a_tag_beats_a_column_beats_a_description_for_the_same_word() {
        let assets = vec![
            json!({ "id": "d", "name": "D", "description": "about sales" }),
            json!({ "id": "c", "name": "C" }),
            json!({ "id": "t", "name": "T", "tags": ["sales"] }),
            json!({ "id": "n", "name": "Sales ledger" }),
        ];
        let columns: ColumnIndex =
            HashMap::from([("c".to_owned(), vec![("sales".to_owned(), String::new())])]);
        let hits = search(&index(&assets, &columns), &UsageIndex::new(), "sales");
        assert_eq!(ids(&hits), vec!["n", "t", "c", "d"]);
        // The name is on screen already, so it is never given as the reason.
        assert!(hits[0].get("matchedOn").is_none());
    }

    #[test]
    fn a_name_equal_to_the_word_beats_a_name_that_only_contains_it() {
        let assets = vec![
            json!({ "id": "a", "name": "Sales ledger" }),
            json!({ "id": "b", "name": "Sales" }),
        ];
        assert_eq!(ids(&find(&assets, "sales")), vec!["b", "a"]);
    }

    #[test]
    fn an_exact_hit_beats_an_approximate_one_in_the_same_field() {
        let assets = vec![
            json!({ "id": "a", "name": "A", "tags": ["revnue"] }),
            json!({ "id": "b", "name": "B", "tags": ["revenue"] }),
        ];
        // `a` is listed first and would win a tie; `b` must win on the word.
        assert_eq!(ids(&find(&assets, "revenue")), vec!["b", "a"]);
    }

    #[test]
    fn equal_scores_order_by_use_then_by_id() {
        let assets = vec![
            json!({ "id": "x.a", "name": "A", "tags": ["sales"] }),
            json!({ "id": "x.b", "name": "B", "tags": ["sales"] }),
            json!({ "id": "x.c", "name": "C", "tags": ["sales"] }),
        ];
        let usage: UsageIndex = HashMap::from([("x.c".to_owned(), 9), ("x.b".to_owned(), 2)]);
        let hits = search(&index(&assets, &ColumnIndex::new()), &usage, "sales");
        assert_eq!(ids(&hits), vec!["x.c", "x.b", "x.a"]);
    }

    #[test]
    fn a_ninth_word_is_ignored() {
        let assets = vec![json!({
            "id": "a", "name": "one two three four five six seven eight"
        })];
        assert_eq!(
            find(&assets, "one two three four five six seven eight nine").len(),
            1
        );
        assert_eq!(find(&assets, "nine"), [] as [serde_json::Value; 0]);
    }

    // `DATA-11` review `SHOULD-FIX 8` changed this test (rule 2: the rule it
    // pinned was wrong, not the code). The reason used to come from the first
    // word even when that word matched the name, which left a result found
    // through its second word with no reason. It now comes from the first
    // word whose best field is not the name; with two non-name words the
    // typed order still decides, as before.
    #[test]
    fn the_reason_comes_from_the_first_word_the_name_does_not_explain() {
        let assets = vec![json!({
            "id": "a", "name": "Plain", "tags": ["finance"], "owner": "Ops"
        })];
        let hits = find(&assets, "finance ops");
        assert_eq!(hits[0]["matchedOn"]["field"], "tag");
        let hits = find(&assets, "ops finance");
        assert_eq!(hits[0]["matchedOn"]["field"], "owner");
    }

    // `DATA-11` review `SHOULD-FIX 8`.
    #[test]
    fn a_result_found_through_its_second_word_shows_that_word_as_the_reason() {
        let mut columns = ColumnIndex::new();
        columns.insert(
            "bronze.northwind_orders".to_owned(),
            vec![("customer_id".to_owned(), String::new())],
        );
        let assets = vec![json!({
            "id": "bronze.northwind_orders", "name": "Northwind Orders"
        })];
        let hits = search(
            &index(&assets, &columns),
            &UsageIndex::new(),
            "northwind customer",
        );
        assert_eq!(
            hits[0]["matchedOn"],
            json!({ "field": "column", "value": "customer_id", "approximate": false })
        );
    }

    // `DATA-11` review `SHOULD-FIX 8`: every word explained by the name.
    #[test]
    fn no_reason_is_shown_when_every_word_matches_the_name() {
        let assets = vec![json!({
            "id": "bronze.northwind_customers", "name": "Northwind Customers"
        })];
        let hits = find(&assets, "northwind customer");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].get("matchedOn").is_none());
    }

    // `DATA-32` D1: the name forgave the typo and nothing else explains the
    // hit, so the name is the reason and it says the match was approximate.
    #[test]
    fn a_typo_that_only_the_name_forgives_is_reported_as_an_approximate_name_match() {
        let assets = vec![json!({ "id": "silver.revenue", "name": "Revenue" })];
        let hits = find(&assets, "revnue");
        assert_eq!(ids(&hits), vec!["silver.revenue"]);
        assert_eq!(
            hits[0]["matchedOn"],
            json!({ "field": "name", "value": "", "approximate": true })
        );
    }

    // `DATA-32` D1: an exact name match is still no reason; the name is on
    // screen already.
    #[test]
    fn an_exact_name_match_still_has_no_reason() {
        let assets = vec![json!({ "id": "silver.revenue", "name": "Revenue" })];
        let hits = find(&assets, "revenue");
        assert_eq!(ids(&hits), vec!["silver.revenue"]);
        assert!(hits[0].get("matchedOn").is_none());
    }

    #[test]
    fn the_one_edit_check_covers_each_kind_of_edit_and_nothing_more() {
        let edit = |a: &str, b: &str| {
            within_one_edit(
                &a.chars().collect::<Vec<_>>(),
                &b.chars().collect::<Vec<_>>(),
            )
        };
        assert!(edit("abcd", "abcd"));
        assert!(edit("abcd", "abxd"));
        assert!(edit("abcd", "abd"));
        assert!(edit("abd", "abcd"));
        assert!(edit("abcd", "abc"));
        assert!(edit("abcd", "bacd"));
        assert!(edit("abcd", "abdc"));
        assert!(!edit("abcd", "badc"));
        assert!(!edit("abcd", "ab"));
        assert!(!edit("abcd", "axyd"));
    }

    // `DATA-41` D2: the index is prepared once, with the copy, so a search
    // allocates nothing per candidate. The two facts that make that true
    // are pinned here: a text is prepared once and shared by every asset
    // that carries it (the same column name is on thousands of tables),
    // and the prepared text is already lower-cased and tokenised.
    #[test]
    fn the_index_prepares_each_text_once_and_shares_it() {
        let assets = vec![
            json!({ "id": "silver.a", "name": "Revenue" }),
            json!({ "id": "silver.b", "name": "REVENUE" }),
        ];
        let index = index(&assets, &ColumnIndex::new());
        let first = &index.entries[0].candidates[0];
        let second = &index.entries[1].candidates[0];
        assert!(Arc::ptr_eq(&first.text, &second.text));
        assert_eq!(first.text.lower, "revenue");
        assert_eq!(
            first.text.tokens,
            vec![vec!['r', 'e', 'v', 'e', 'n', 'u', 'e']]
        );
    }

    // `DATA-41` D2: the index is built from the assets and the columns, and
    // a search over it answers exactly as the same search over the assets
    // did before the index existed.
    #[test]
    fn the_index_answers_the_same_as_the_assets_it_was_built_from() {
        let assets = explorer_fixture();
        let columns: ColumnIndex = HashMap::from([(
            "silver.mart_wisman".to_owned(),
            vec![("revenue_amount".to_owned(), "Amount invoiced".to_owned())],
        )]);
        let hits = search(
            &index(&assets, &columns),
            &UsageIndex::new(),
            "revenue_amount",
        );
        assert_eq!(ids(&hits), vec!["silver.mart_wisman"]);
        assert_eq!(
            hits[0]["matchedOn"],
            json!({ "field": "column", "value": "revenue_amount", "approximate": false })
        );
    }
}
