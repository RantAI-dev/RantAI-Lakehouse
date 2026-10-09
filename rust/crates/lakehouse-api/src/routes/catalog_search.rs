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

use std::collections::HashMap;

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

    /// The `matchedOn.field` spelling; `None` for the name, which is never
    /// reported because the name is already on screen.
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

/// One piece of text of one asset a word can match.
struct Candidate {
    field: Field,
    lower: String,
    /// What `matchedOn.value` shows: the tag or the column name, empty for
    /// the rest.
    shown: String,
}

/// The best field one word found.
struct Hit {
    score: u32,
    field: Field,
    shown: String,
    approximate: bool,
}

fn text_of<'a>(asset: &'a Value, key: &str) -> &'a str {
    asset.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn candidates(asset: &Value, columns: &ColumnIndex) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut plain = |field: Field, text: &str| {
        if !text.is_empty() {
            out.push(Candidate {
                field,
                lower: text.to_lowercase(),
                shown: String::new(),
            });
        }
    };
    plain(Field::Name, text_of(asset, "name"));
    plain(Field::Id, text_of(asset, "id"));
    plain(Field::Namespace, text_of(asset, "namespace"));
    plain(Field::Owner, text_of(asset, "owner"));
    plain(Field::Description, text_of(asset, "description"));
    if let Some(tags) = asset.get("tags").and_then(Value::as_array) {
        for tag in tags.iter().filter_map(Value::as_str) {
            out.push(Candidate {
                field: Field::Tag,
                lower: tag.to_lowercase(),
                shown: tag.to_owned(),
            });
        }
    }
    if let Some(cols) = columns.get(text_of(asset, "id")) {
        for (name, description) in cols {
            out.push(Candidate {
                field: Field::Column,
                lower: name.to_lowercase(),
                shown: name.clone(),
            });
            if !description.is_empty() {
                out.push(Candidate {
                    field: Field::ColumnDescription,
                    lower: description.to_lowercase(),
                    shown: String::new(),
                });
            }
        }
    }
    out
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

/// Whether `word`, not contained in `field` as typed, matches it
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
fn approximately_in(word: &str, field: &str) -> Option<bool> {
    let mut any = false;
    let mut edited = false;
    for part in tokens(word) {
        any = true;
        if field.contains(part) {
            continue;
        }
        let part_chars: Vec<char> = part.chars().collect();
        if part_chars.len() < MIN_APPROX_CHARS
            || !tokens(field).any(|t| {
                let t: Vec<char> = t.chars().collect();
                within_one_edit(&t, &part_chars)
            })
        {
            return None;
        }
        edited = true;
    }
    any.then_some(edited)
}

fn best_hit(word: &str, candidates: &[Candidate]) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for c in candidates {
        let (score, approximate) = if c.lower.contains(word) {
            let whole = c.field == Field::Name && c.lower == word;
            (
                if whole {
                    SCORE_NAME_WHOLE
                } else {
                    c.field.weights().0
                },
                false,
            )
        } else if let Some(edited) = approximately_in(word, &c.lower) {
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
/// each with `matchedOn` set for the field that scored highest on its first
/// word (omitted when that field is the name). `usage` is `DATA-11` D4.
pub fn search(
    assets: &[Value],
    columns: &ColumnIndex,
    usage: &UsageIndex,
    term: &str,
) -> Vec<Value> {
    let lowered = term.to_lowercase();
    let words: Vec<&str> = lowered.split_whitespace().take(MAX_WORDS).collect();
    if words.is_empty() {
        return assets.to_vec();
    }

    let mut ranked: Vec<(u32, u32, &Value, Option<Hit>)> = Vec::new();
    for asset in assets {
        let cands = candidates(asset, columns);
        let mut total = 0_u32;
        let mut first = None;
        let mut all = true;
        for (i, word) in words.iter().enumerate() {
            let Some(hit) = best_hit(word, &cands) else {
                all = false;
                break;
            };
            total += hit.score;
            if i == 0 {
                first = Some(hit);
            }
        }
        if all {
            let used = usage.get(text_of(asset, "id")).copied().unwrap_or(0);
            ranked.push((total, used, asset, first));
        }
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then_with(|| text_of(a.2, "id").cmp(text_of(b.2, "id")))
    });
    ranked
        .into_iter()
        .map(|(_, _, asset, hit)| {
            let mut asset = asset.clone();
            let reported =
                hit.and_then(|h| h.field.reported().map(|f| (f, h.shown, h.approximate)));
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

    fn find(assets: &[Value], term: &str) -> Vec<Value> {
        search(assets, &ColumnIndex::new(), &UsageIndex::new(), term)
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
        assert!(find(&assets, "restoran zzzzzz").is_empty());
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
        assert!(find(&assets, "rvn").is_empty());
        assert!(find(&assets, "ravanue").is_empty());
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
        let hit = search(&assets, &columns, &UsageIndex::new(), "custmer_id");
        assert_eq!(ids(&hit), ["silver.other"]);
        assert_eq!(
            hit[0]["matchedOn"],
            json!({ "field": "column", "value": "customer_id", "approximate": true })
        );
        assert_eq!(find(&assets, "silver.ordrs").len(), 1);
        // `xx` is under 4 characters and is not in the column: no match.
        assert!(search(&assets, &columns, &UsageIndex::new(), "custmer_xx").is_empty());
        // No letter or digit: nothing to match.
        assert!(find(&assets, "___").is_empty());
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
        let hits = search(&assets, &columns, &UsageIndex::new(), "id_customer");
        assert_eq!(ids(&hits), ["silver.exact", "silver.swapped"]);
        assert_eq!(
            hits[1]["matchedOn"],
            json!({ "field": "column", "value": "customer_id", "approximate": false })
        );
        // A real typo is still flagged.
        let typo = search(&assets, &columns, &UsageIndex::new(), "custmer_id");
        assert!(
            typo.iter()
                .all(|h| h["matchedOn"]["approximate"] == json!(true))
        );
        assert!(!typo.is_empty());
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
        let hits = search(&assets, &columns, &UsageIndex::new(), "revenue_amount");
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
        let by = |term: &str| search(&assets, &columns, &UsageIndex::new(), term);
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
        let hits = search(&assets, &columns, &UsageIndex::new(), "sales");
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
        let hits = search(&assets, &ColumnIndex::new(), &usage, "sales");
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
        assert!(find(&assets, "nine").is_empty());
    }

    #[test]
    fn the_reason_comes_from_the_first_word() {
        let assets = vec![json!({
            "id": "a", "name": "Plain", "tags": ["finance"], "owner": "Ops"
        })];
        let hits = find(&assets, "finance ops");
        assert_eq!(hits[0]["matchedOn"]["field"], "tag");
        let hits = find(&assets, "ops finance");
        assert_eq!(hits[0]["matchedOn"]["field"], "owner");
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
}
