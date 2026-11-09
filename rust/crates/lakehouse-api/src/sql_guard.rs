//! Lexical guards for user-authored `SQL`: the read-only check every ad hoc
//! surface applies before a statement reaches an engine, and the extra
//! `Trino` vocabulary check.
//!
//! Moved here verbatim from `routes/query.rs` so Query Studio and dashboard
//! SQL sources share one guard instead of two copies (AGENTS.md rule 4).
//! These are word-level checks, not a parser: they run first and cheaply;
//! `sql_rewrite` (a real `sqlparser` pass) still decides what a statement
//! may touch.
//!
//! The lexical helpers it uses live in [`crate::sql_lex`], the module
//! that holds the engine's lexical rules (`SEC-62`).

use crate::sql_lex::{skip_block_comment, skip_quoted, skip_to_line_end};

/// Whether `sql` is a read-only statement `ClickHouse` may run from Query
/// Studio.
///
/// Both halves of the check run against [`strip_sql_noise`]'s output
/// rather than the raw text, which is the difference from the
/// `query/run/route.ts` guard this was ported from. That guard tested the
/// raw string, and so rejected two things it had no reason to:
///
/// - Anything written under a leading comment — including the editor's own
///   starter line, `-- Write SQL here…`, which made the very first query a
///   new user typed fail with "only read queries are allowed".
/// - Any query merely *mentioning* a DML word, e.g.
///   `SELECT * FROM t WHERE action = 'drop'`.
///
/// What it still rejects is what matters: a statement that does not start
/// with a read keyword, and smuggled DML such as `SELECT 1; DELETE FROM t`
/// — the semicolon trick is why the denied-word test looks at the whole
/// statement rather than just its first word.
#[must_use]
pub(crate) fn is_read_only(sql: &str) -> bool {
    let code = strip_sql_noise(sql);
    starts_with_allowed_keyword(&code) && !contains_denied_keyword(&code)
}

/// `sql` with comments and quoted literals blanked out, so the guard reads
/// only the parts of a statement that can actually do something.
///
/// Removed: `-- line` and `# line` comments, `/* block */` comments, and
/// `'single'`, `"double"` and `` `backtick` `` quoted runs (doubled quotes
/// and backslash escapes inside them included). Each is replaced by a
/// single space rather than deleted, so words either side of it cannot be
/// glued into one.
#[must_use]
pub(crate) fn strip_sql_noise(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                i = skip_to_line_end(&chars, i);
                out.push(' ');
            }
            '#' => {
                i = skip_to_line_end(&chars, i);
                out.push(' ');
            }
            '/' if next == Some('*') => {
                i = skip_block_comment(&chars, i);
                out.push(' ');
            }
            '\'' | '"' | '`' => {
                i = skip_quoted(&chars, i, c, true);
                out.push(' ');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `/^\s*(with|select|show|describe|desc|explain)\b/i.test(sql)` — leading
/// whitespace, then one of the allowed keywords, then a word boundary.
fn starts_with_allowed_keyword(sql: &str) -> bool {
    const ALLOWED: [&str; 6] = ["with", "select", "show", "describe", "desc", "explain"];
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    ALLOWED.iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    })
}

/// `/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)`
/// — the whole string, any position, word-boundary delimited.
fn contains_denied_keyword(sql: &str) -> bool {
    const DENIED: [&str; 12] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename", "attach",
        "detach", "grant", "revoke",
    ];
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    DENIED.iter().any(|kw| word_occurs(&chars, kw))
}

/// Whether `chars` (already lowercased) contains `word` at a position
/// bounded by non-word characters (or the string's edges) on both sides —
/// `\b<word>\b` in a case-insensitive regex.
fn word_occurs(chars: &[char], word: &str) -> bool {
    let word_chars: Vec<char> = word.chars().collect();
    let n = word_chars.len();
    if n == 0 || chars.len() < n {
        return false;
    }
    for start in 0..=(chars.len() - n) {
        if chars[start..start + n] == word_chars[..] {
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let after_ok = start + n == chars.len() || !is_word_char(chars[start + n]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// `\w` in a JavaScript regex: `[A-Za-z0-9_]`.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `sql` is additionally safe to send to `Trino`, on top of
/// [`is_read_only`] (checked first, unconditionally, for every engine —
/// this function is never the only gate). Applied only when the request
/// names `engine: "trino"`.
///
/// [`is_read_only`]'s denylist was written for `ClickHouse` and is not
/// sufficient here, for two reasons specific to `Trino`:
///
/// - `Trino`'s `EXPLAIN ANALYZE` *executes* the statement it explains,
///   unlike `ClickHouse`'s `EXPLAIN`, which never runs anything. A
///   statement starting `EXPLAIN ANALYZE <write>` passes
///   [`is_read_only`]'s "starts with an allowed keyword" test and, unless
///   `<write>` happens to use one of the 12 `ClickHouse`-oriented denied
///   words, sails through it entirely — see [`contains_explain_analyze`].
/// - `Trino` speaks `SQL` statements `ClickHouse` doesn't have, several of
///   which write: `MERGE`, `REFRESH MATERIALIZED VIEW`, `COMMENT ON`,
///   `CALL` (procedures, including maintenance ones), `EXECUTE`/`PREPARE`/
///   `DEALLOCATE` (prepared statements, a smuggling vector for anything
///   above), `SET`/`RESET` (session state), and `DENY` (access control
///   itself). None of these are in [`is_read_only`]'s `ClickHouse`-shaped
///   denylist, so they are refused here instead.
///
/// This guard is the console's OWN boundary, not `Trino`'s. `Trino`'s
/// file-based access control (the compose `trino` service's
/// `rules.json`) restricts the `iceberg` catalog to `SELECT` for every
/// user except `trino-maintenance`, but it does NOT block `ALTER TABLE …
/// EXECUTE optimize` for a plain reader — Trino's
/// `checkCanExecuteTableProcedure` requires only table `SELECT`, with no
/// separate privilege for maintenance procedures (see the compose file's
/// block comment on `rules.json` for the measured proof). So this guard
/// is what actually stops a `query:read` principal from running `ALTER
/// TABLE … EXECUTE optimize` through the console, not `Trino` itself.
#[must_use]
pub(crate) fn is_trino_safe(sql: &str) -> bool {
    !contains_explain_analyze(sql) && !contains_trino_denied_keyword(sql)
}

/// Whether `sql` contains `explain`, followed — modulo any amount of
/// whitespace, and case-insensitively — by `analyze`, as adjacent words.
/// `Trino` accepts arbitrary whitespace between the two keywords
/// (`EXPLAIN   ANALYZE`, `explain\nanalyze`, ...), so the comparison
/// normalizes every whitespace run to a single space before matching.
fn contains_explain_analyze(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    // Sentinel spaces at both ends turn the substring check into a
    // word-bounded match with no separate boundary logic needed.
    format!(" {collapsed} ").contains(" explain analyze ")
}

/// The `Trino`-specific write/session/procedure vocabulary
/// [`is_trino_safe`] refuses, checked as whole words with [`word_occurs`]
/// — the same helper [`contains_denied_keyword`] uses, rather than a
/// second matcher.
fn contains_trino_denied_keyword(sql: &str) -> bool {
    const TRINO_DENIED: [&str; 10] = [
        "merge",
        "refresh",
        "comment",
        "call",
        "execute",
        "deny",
        "set",
        "reset",
        "prepare",
        "deallocate",
    ];
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    TRINO_DENIED.iter().any(|kw| word_occurs(&chars, kw))
}

/// Longest SQL source text accepted. Generous for a hand-written join; the
/// bound only keeps a pasted dump from being stored and re-sent per tile.
const SQL_SOURCE_MAX_CHARS: usize = 20_000;

/// Why a statement cannot become a dashboard SQL source. Every message is
/// fixed text written here, never parser or engine output, so it is safe to
/// return to the caller as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlSourceRefusal {
    /// Blank after trimming.
    Empty,
    /// Over [`SQL_SOURCE_MAX_CHARS`].
    TooLong,
    /// Not a `SELECT`/`WITH` query, or [`is_read_only`] refused it.
    NotASelect,
    /// More than one statement.
    MultipleStatements,
    /// A `SETTINGS`/`FORMAT` clause, which would break the wrapper the
    /// builder puts around the source (its own `SETTINGS` must be last).
    SettingsOrFormat,
    /// `sqlparser` could not parse it, so the tables it reads are unknown.
    Unparseable,
    /// It reads a table outside `serving` (including `console.*` and
    /// `system.*`), or a bare table name.
    OutsideServing,
    /// A well-formed `{{name}}` placeholder (`QS-5`). A dashboard source cannot
    /// take parameters until part 2, and the engine would answer with a
    /// message about `{`.
    ParametersLater,
}

impl SqlSourceRefusal {
    /// The caller-facing message.
    #[must_use]
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Empty => "SQL is required.",
            Self::TooLong => "SQL source is too long (max 20000 characters).",
            Self::NotASelect => "a SQL source must be a single read-only SELECT or WITH query.",
            Self::MultipleStatements => "a SQL source must be a single statement.",
            Self::SettingsOrFormat => {
                "remove the SETTINGS/FORMAT clause; dashboard limits are applied automatically."
            }
            Self::Unparseable => "the SQL could not be parsed, so the tables it reads are unknown.",
            Self::OutsideServing => {
                "a SQL source may only read Gold tables, written as serving.<table>."
            }
            Self::ParametersLater => {
                "Parameters in a dashboard's SQL source come later: give the value in the SQL \
                 for now."
            }
        }
    }
}

/// Decide whether `sql` may be stored as a dashboard SQL source, returning
/// the normalized text (trimmed, one trailing `;` dropped) when it may.
///
/// Fail closed at every step (AGENTS.md principle 3): a statement this
/// function cannot fully account for is refused, not passed along. It is
/// the lexical gate only; table functions, sensitive `system.*` reads and
/// governed tables are then handled by the same
/// `policy_engine::rewrite_sql_for_roles` pass every execution goes through,
/// which runs before the source is probed or stored.
///
/// # Errors
///
/// The [`SqlSourceRefusal`] naming the first rule `sql` breaks.
pub(crate) fn check_sql_source(sql: &str) -> Result<String, SqlSourceRefusal> {
    let trimmed = sql.trim();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    if trimmed.is_empty() {
        return Err(SqlSourceRefusal::Empty);
    }
    if trimmed.chars().count() > SQL_SOURCE_MAX_CHARS {
        return Err(SqlSourceRefusal::TooLong);
    }
    // `query-parameters.md` decision 15: before anything reads the statement as ClickHouse SQL. The
    // scanner is the one that knows comments and quotes, so a `{{` in a
    // comment, or text that is not a well-formed placeholder, changes
    // nothing here.
    if crate::query_params::holds_placeholder(trimmed) {
        return Err(SqlSourceRefusal::ParametersLater);
    }
    let code = strip_sql_noise(trimmed);
    let lower = code.trim_start().to_ascii_lowercase();
    let starts_select = ["select", "with"].iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    });
    // `is_read_only` also admits SHOW/DESCRIBE/EXPLAIN, which cannot be a
    // derived table, hence the narrower first-word check.
    if !starts_select || !is_read_only(trimmed) {
        return Err(SqlSourceRefusal::NotASelect);
    }
    if code.contains(';') {
        return Err(SqlSourceRefusal::MultipleStatements);
    }
    let chars: Vec<char> = lower.chars().collect();
    if word_occurs(&chars, "settings") || word_occurs(&chars, "format") {
        return Err(SqlSourceRefusal::SettingsOrFormat);
    }
    let tables =
        crate::sql_rewrite::referenced_tables(trimmed, &sqlparser::dialect::ClickHouseDialect {})
            .ok_or(SqlSourceRefusal::Unparseable)?;
    let all_serving = tables.iter().all(|t| {
        t.split_once('.')
            .is_some_and(|(schema, table)| schema == "serving" && !table.is_empty())
    });
    if !all_serving {
        return Err(SqlSourceRefusal::OutsideServing);
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod sql_source_rules {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{SqlSourceRefusal, check_sql_source};

    #[test]
    fn a_join_across_two_serving_marts_is_accepted_and_normalized() {
        let sql = "  SELECT g.material_group, t.material_type\n\
                   FROM serving.mart_material_by_group AS g\n\
                   JOIN serving.mart_material_by_type AS t USING (material_group);  ";
        let ok = check_sql_source(sql).expect("a serving join is a valid source");
        assert!(ok.starts_with("SELECT"));
        assert!(!ok.ends_with(';'));
    }

    #[test]
    fn a_cte_over_serving_is_accepted() {
        let sql = "WITH x AS (SELECT * FROM serving.mart_a) SELECT * FROM x";
        assert!(check_sql_source(sql).is_ok());
    }

    #[test]
    fn a_trailing_comment_is_allowed() {
        assert!(check_sql_source("SELECT * FROM serving.mart_a -- note").is_ok());
    }

    #[test]
    fn tables_outside_serving_are_refused() {
        for sql in [
            "SELECT * FROM console.bi_chart",
            "SELECT * FROM system.users",
            "SELECT * FROM silver.customers",
            "SELECT * FROM mart_a",
            "SELECT * FROM serving.mart_a JOIN console.bi_board USING (id)",
            "SELECT (SELECT count() FROM system.tables) AS n FROM serving.mart_a",
        ] {
            assert_eq!(
                check_sql_source(sql),
                Err(SqlSourceRefusal::OutsideServing),
                "{sql}"
            );
        }
    }

    #[test]
    fn writes_and_non_select_statements_are_refused() {
        for sql in [
            "INSERT INTO serving.mart_a VALUES (1)",
            "DROP TABLE serving.mart_a",
            "SHOW TABLES",
            "DESCRIBE serving.mart_a",
            "EXPLAIN SELECT 1",
        ] {
            assert_eq!(
                check_sql_source(sql),
                Err(SqlSourceRefusal::NotASelect),
                "{sql}"
            );
        }
    }

    #[test]
    fn a_second_statement_is_refused() {
        let err = check_sql_source("SELECT 1 FROM serving.a; SELECT 2 FROM serving.b").unwrap_err();
        assert_eq!(err, SqlSourceRefusal::MultipleStatements);
    }

    #[test]
    fn settings_and_format_clauses_are_refused() {
        for sql in [
            "SELECT * FROM serving.a SETTINGS max_threads = 1",
            "SELECT * FROM serving.a FORMAT JSON",
        ] {
            assert_eq!(
                check_sql_source(sql),
                Err(SqlSourceRefusal::SettingsOrFormat),
                "{sql}"
            );
        }
    }

    #[test]
    fn empty_and_oversized_text_is_refused() {
        assert_eq!(check_sql_source("  ;  "), Err(SqlSourceRefusal::Empty));
        let long = format!("SELECT * FROM serving.a WHERE x = '{}'", "a".repeat(20_001));
        assert_eq!(check_sql_source(&long), Err(SqlSourceRefusal::TooLong));
    }

    #[test]
    fn unparseable_sql_is_refused_not_passed_through() {
        assert_eq!(
            check_sql_source("SELECT * FROM serving.a WHERE ((("),
            Err(SqlSourceRefusal::Unparseable)
        );
    }

    /// `query-parameters.md` decision 15: parameters in a source come later, so a well-formed
    /// placeholder is refused with the page's sentence instead of reaching
    /// the engine parser, which would answer with a message about `{`.
    #[test]
    fn a_placeholder_in_a_source_is_refused_with_the_part_two_sentence() {
        for sql in [
            "SELECT * FROM serving.a WHERE region = {{region}}",
            "SELECT * FROM serving.a WHERE region = {{ region }}",
            "SELECT * FROM serving.a WHERE day >= {{days.start}} AND day <= {{days.end}}",
            "SELECT * FROM serving.a WHERE region IN ({{regions}})",
            // The same words in a string are a placeholder the person meant.
            "SELECT * FROM serving.a WHERE region = '{{region}}'",
        ] {
            let refusal = check_sql_source(sql).unwrap_err();
            assert_eq!(refusal, SqlSourceRefusal::ParametersLater, "{sql}");
            assert_eq!(
                refusal.message(),
                "Parameters in a dashboard's SQL source come later: give the value in the \
                 SQL for now.",
                "{sql}"
            );
        }
    }

    /// The refusal comes before the engine parse: a source that holds a
    /// placeholder and would also fail the parse says the placeholder first.
    #[test]
    fn the_placeholder_refusal_comes_before_the_engine_parse() {
        assert_eq!(
            check_sql_source("SELECT * FROM serving.a WHERE x = {{x}} AND ((("),
            Err(SqlSourceRefusal::ParametersLater)
        );
    }

    /// A `{{` that is not a well-formed placeholder, or that sits in a
    /// comment, changes nothing: the source is judged as it was before.
    #[test]
    fn braces_that_are_not_a_placeholder_leave_the_verdict_as_it_was() {
        for sql in [
            "SELECT * FROM serving.a -- {{region}}",
            "SELECT * FROM serving.a /* {{region}} */",
            "SELECT '{{ not closed' AS note FROM serving.a",
            "SELECT '{{9x}}' AS note FROM serving.a",
        ] {
            assert!(check_sql_source(sql).is_ok(), "{sql}");
        }
        // A stray `{{` outside quotes and comments is not a well-formed
        // placeholder either: it keeps failing the way it did (a parse
        // failure), not with the part-two sentence.
        assert_eq!(
            check_sql_source("SELECT * FROM serving.a WHERE x = {{"),
            Err(SqlSourceRefusal::Unparseable)
        );
    }
}
