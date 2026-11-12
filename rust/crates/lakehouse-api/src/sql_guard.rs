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
//! # The rules the guard reads with
//!
//! The guard reads the statement the way the engine that will run it reads
//! it, and the rules are [`crate::sql_lex`]'s — one set, one lexer, one
//! place (`SEC-62`; the findings came out of the `#420` and `#437`
//! reviews). The guard takes the engine as an argument for that
//! reason: a `ClickHouse` statement and a `Trino` statement are not the
//! same text, and a copy of the rules that does not know the engine's is a
//! hole rather than a nuisance.
//!
//! What that buys, in the two directions:
//!
//! - A write the old rules hid is now refused. `ClickHouse`'s heredoc
//!   (`$$…$$`) is a quoted run, so the `'` inside it opens no string, and
//!   `Trino` reads a `\` as a plain character, so `'a\'` is a complete
//!   string there. The old guard read both the other way round and let a
//!   `DROP` hide behind them.
//! - A write is not invented either. A `#` line is a comment on `ClickHouse`
//!   only, and a `/* … */` comment nests on `ClickHouse` only: the guard
//!   follows the engine on both.
//!
//! ## The one case where `ClickHouse` is the more permissive engine
//!
//! `ClickHouse` nests block comments: `/* /* */ ; DROP TABLE t */` is one
//! comment there, and the write inside it cannot run. `Trino` ends the
//! comment at the first `*/`, so the same text is code there and the write
//! is refused. The `ClickHouse` verdict is the more permissive one, and it
//! is the correct one: the engine reads the whole run as a comment, so
//! there is nothing there to write. `sqlparser`'s `ClickHouseDialect`
//! nests block comments as well, so the rewrite that runs after this guard
//! reads the text the same way.
//!
//! The lexer it reads with lives in [`crate::sql_lex`], the module that
//! holds the engine's lexical rules (`SEC-62`).

use crate::sql_lex::{SpanKind, lex};
use crate::sql_rewrite::Engine;

/// Whether `sql` is a read-only statement `engine` may run from Query
/// Studio.
///
/// Public so the engine-corpus test (`tests/sql_lex_corpus.rs`) can
/// compare this verdict, statement by statement, with what a
/// real `ClickHouse` reads; apart from that test, every caller is inside this crate.
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
///
/// `engine` says which engine's lexical rules the statement is read with
/// ([`crate::sql_lex`]); the caller knows it, and the run path takes it
/// from the request.
#[must_use]
pub fn is_read_only(sql: &str, engine: Engine) -> bool {
    // A `$`-run no measured rule reads, or a Unicode quote on `ClickHouse`
    // (`SpanKind::Uncertain`), is a shape the lexer cannot follow. The
    // engine refuses every `$` shape the measurements cover with
    // `SYNTAX_ERROR` (code 62, 2026-11-10), and reads `‘ … ’` as a literal
    // that this lexer would misplace (`#452` review SHOULD-FIX,
    // 2026-11-12). Fail closed (AGENTS.md principle 3): the whole
    // statement is refused before any word in it is read.
    if lex(engine, sql)
        .iter()
        .any(|span| span.kind == SpanKind::Uncertain)
    {
        return false;
    }
    let code = strip_sql_noise(sql, engine);
    starts_with_allowed_keyword(&code) && !contains_denied_keyword(&code)
}

/// `sql` with comments and quoted literals blanked out, so the guard reads
/// only the parts of a statement that can actually do something.
///
/// Removed: the comments and the quoted runs `engine` reads — `-- line`
/// and `# line` comments, `/* block */` comments, and `'single'`,
/// `"double"` and `` `backtick` `` quoted runs, the heredoc `$$…$$`
/// included (the backtick run is blanked on `Trino` too, where such a
/// statement is invalid anyway). Each is replaced by a single space
/// rather than deleted, so words either side of it cannot be glued into
/// one.
///
/// Kept: everything the engine reads as code, and the three texts it does
/// not read as a comment. The run after a `#` the engine does not read as
/// a comment start: the engine refuses the statement there, so the text
/// cannot run. And the tail of a heredoc that is never closed: the engine
/// reads that tail as code (`ClickHouse` opens a heredoc only when the
/// closing delimiter is found again in the text), so a denied keyword in
/// it is a keyword the engine would run. And a `$`-run no measured rule
/// reads ([`SpanKind::Uncertain`]): [`is_read_only`] refuses the whole
/// statement when one is present, so keeping it as it stands here cannot
/// make a statement pass. The guard reads all three as they stand.
#[must_use]
pub(crate) fn strip_sql_noise(sql: &str, engine: Engine) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    for span in lex(engine, sql) {
        match span.kind {
            // An `Uncertain` span is kept as it stands, like code: the
            // guard refuses the whole statement when one is present, so
            // nothing here can make it pass.
            SpanKind::Code
            | SpanKind::NotCode
            | SpanKind::UnclosedHeredoc
            | SpanKind::Uncertain => {
                out.extend(chars[span.start..span.end].iter());
            }
            SpanKind::Comment | SpanKind::Literal => out.push(' '),
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
/// A dashboard SQL source runs on `ClickHouse`, so the guard reads it with
/// the `ClickHouse` rules ([`crate::sql_lex`]), as the rewrite behind it
/// does with `ClickHouse`'s dialect.
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
    let code = strip_sql_noise(trimmed, Engine::ClickHouse);
    let lower = code.trim_start().to_ascii_lowercase();
    let starts_select = ["select", "with"].iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    });
    // `is_read_only` also admits SHOW/DESCRIBE/EXPLAIN, which cannot be a
    // derived table, hence the narrower first-word check.
    if !starts_select || !is_read_only(trimmed, Engine::ClickHouse) {
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
mod engine_aware_rules {
    use super::{is_read_only, strip_sql_noise};
    use crate::sql_rewrite::Engine;

    /// A `ClickHouse` heredoc is a quoted run, so the `'` inside it opens
    /// no string and the write behind it is code the engine runs.
    ///
    /// `Trino` has no heredoc: the `'` opens a string there, and the write
    /// behind it is inside that string, so there is nothing to refuse.
    #[test]
    fn a_clickhouse_heredoc_hides_no_write_behind_it() {
        let sql = "SELECT $$'$$; DROP TABLE t; SELECT '";
        assert!(
            !is_read_only(sql, Engine::ClickHouse),
            "the write behind the heredoc is code on ClickHouse"
        );
        assert!(
            is_read_only(sql, Engine::Trino),
            "the write is inside a quoted run on Trino"
        );
    }

    /// A `ClickHouse` heredoc is a literal: its body is raw text the engine
    /// reads as it stands and never as SQL again, so a write inside it is
    /// a word, not a statement. `Trino` has no heredoc, so the same text
    /// is code there and the write is a write.
    #[test]
    fn a_write_inside_a_clickhouse_heredoc_is_a_literal() {
        let sql = "SELECT $$DROP TABLE t$$";
        assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
        assert!(!is_read_only(sql, Engine::Trino), "{sql}");
    }

    /// `Trino` reads a `\` as a plain character, so `'a\'` is a complete
    /// string there and the write behind it is code. `ClickHouse` reads
    /// `\'` as an escaped quote, so the string runs on and the write
    /// cannot run.
    #[test]
    fn a_trino_backslash_ends_the_string_and_a_clickhouse_one_does_not() {
        let sql = "SELECT 'a\\'; DROP TABLE t; SELECT '";
        assert!(!is_read_only(sql, Engine::Trino), "{sql}");
        assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
    }

    /// `ClickHouse` nests block comments, so the comment runs to the `*/`
    /// that closes the outermost one and the write inside it cannot run.
    /// `Trino` ends the comment at the first `*/`, so the write behind it
    /// is code.
    #[test]
    fn a_nested_clickhouse_comment_hides_no_write() {
        let sql = "SELECT 1 /* /* */ ; DROP TABLE t */";
        assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
        assert!(!is_read_only(sql, Engine::Trino), "{sql}");
    }

    // ── the rules measured 2026-11-10 on
    // `clickhouse/clickhouse-server:26.8` ────────────────────────────────

    /// The `#446` review shape: a `$$` written right after the closed
    /// heredoc `$$x$$`. When the backward scan crossed into that heredoc
    /// it read the run as part of one name, the `'` behind it opened a
    /// string, and the `DROP` hid from this guard. Measured on
    /// `clickhouse/clickhouse-server:26.8` (2026-11-10): the engine
    /// refuses every shape of the family with `SYNTAX_ERROR` (code 62) and
    /// runs nothing in it, so the guard refuses the whole statement — the
    /// adjacent `$`-run is a construct no measured rule reads.
    #[test]
    fn an_adjacent_dollar_run_after_a_closed_heredoc_is_refused() {
        let sql = "SELECT $$x$$$$ ' $$; DROP TABLE t; SELECT 'z'";
        assert!(
            !is_read_only(sql, Engine::ClickHouse),
            "the shape the engine refuses with code 62 must not pass: {sql}"
        );
        assert!(!is_read_only("SELECT $$x$$$$y$$", Engine::ClickHouse));
        assert!(!is_read_only("SELECT $$x$$$$ ' $$", Engine::ClickHouse));
    }

    /// Measured on `clickhouse/clickhouse-server:26.8` (2026-11-10): the
    /// engine refuses `$'$x$'$` with `SYNTAX_ERROR` at the first `$` and
    /// `$é$x$é$` with "Unrecognized token" at the non-ASCII character. No
    /// measured rule reads a `$` before a quote or a non-ASCII character,
    /// so both statements are refused here rather than guessed.
    #[test]
    fn a_dollar_no_measured_rule_reads_is_refused() {
        assert!(!is_read_only("SELECT $'$x$'$", Engine::ClickHouse));
        assert!(!is_read_only("SELECT $é$x$é$", Engine::ClickHouse));
    }

    /// The `#452` review SHOULD-FIX: measured on
    /// `clickhouse/clickhouse-server:26.8` (2026-11-12), `‘ … ’` is a string
    /// literal and `“ … ”` a quoted identifier, so the `'` inside them opens
    /// no string and the `DROP` behind it is code the engine runs. The
    /// lexer cannot follow them, so the whole statement is refused on
    /// `ClickHouse`. `Trino` reads them as ordinary text (`sqlparser` fails
    /// them later), so its verdict stays what it was.
    #[test]
    fn a_unicode_quote_hides_no_write_on_clickhouse() {
        let sql = "SELECT \u{2018} ' \u{2019}; DROP TABLE t; SELECT \u{2018} ' \u{2019}";
        assert!(!is_read_only(sql, Engine::ClickHouse), "{sql}");
        assert!(is_read_only(sql, Engine::Trino), "{sql}");
        let identifier = "SELECT \u{201C} ' \u{201D}; DROP TABLE t; SELECT \u{201C} ' \u{201D}";
        assert!(
            !is_read_only(identifier, Engine::ClickHouse),
            "{identifier}"
        );
    }

    /// Inside an ASCII literal, a quoted identifier or a comment the four
    /// characters are data, so a statement that only holds them there stays
    /// read-only on both engines.
    #[test]
    fn a_unicode_quote_inside_another_span_leaves_the_verdict_alone() {
        for sql in [
            "SELECT '\u{2018}'",
            "SELECT \"\u{2019}\" FROM t",
            "SELECT 1 -- \u{2018}",
            "SELECT 1 /* \u{201C} */",
        ] {
            assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
            assert!(is_read_only(sql, Engine::Trino), "{sql}");
        }
    }

    /// The measured shapes that keep their verdicts: `SELECT $1$x$1$` runs
    /// on the engine (a heredoc with a digit tag), `SELECT $a-b$x$a-b$`
    /// parses (a `$` beside name characters is one identifier), `SELECT
    /// $a b$x$a b$` opens no heredoc (the tag scan stops at the space),
    /// and `//` is a line comment the engine reads. The last three are
    /// refused by the engine at parse or analysis time, but the refusal
    /// happens there and not here, so the guard keeps them allowed.
    #[test]
    fn the_measured_heredoc_and_slash_shapes_stay_allowed() {
        assert!(is_read_only("SELECT $1$x$1$", Engine::ClickHouse));
        assert!(is_read_only("SELECT $a-b$x$a-b$", Engine::ClickHouse));
        assert!(is_read_only("SELECT $a b$x$a b$", Engine::ClickHouse));
        assert!(is_read_only("SELECT 1 // c", Engine::ClickHouse));
        assert!(is_read_only("SELECT 8 // 3", Engine::ClickHouse));
        assert!(is_read_only("SELECT 1 // '\n, 2", Engine::ClickHouse));
        assert!(is_read_only("SELECT 1 /\n-- c\n2", Engine::ClickHouse));
    }

    /// `#` starts a comment on `ClickHouse` only when a space or a `!`
    /// follows it. `Trino` has no `#` comment, so the text behind it is
    /// code.
    #[test]
    fn a_hash_is_a_comment_on_clickhouse_only() {
        let sql = "SELECT 1 # ; DROP TABLE t";
        assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
        assert!(!is_read_only(sql, Engine::Trino), "{sql}");
    }

    /// A heredoc that is never closed is no heredoc to the engine: the `$`
    /// opens nothing and the text behind it is ordinary SQL to run.
    #[test]
    fn an_unclosed_heredoc_is_code_on_clickhouse() {
        assert!(
            !is_read_only("SELECT 1$b$ ; DROP TABLE t", Engine::ClickHouse),
            "the tail of an unclosed heredoc is code on ClickHouse"
        );
    }

    /// A `#` line is a comment on `ClickHouse` and text the engine refuses
    /// on `Trino`, so the statement behind it is code there.
    #[test]
    fn a_hash_comment_line_is_refused_on_trino() {
        let sql = "# hash comment\nSELECT 1";
        assert!(is_read_only(sql, Engine::ClickHouse), "{sql}");
        assert!(!is_read_only(sql, Engine::Trino), "{sql}");
    }

    /// Where the two engines read the text the same way, the blanked text
    /// is the same too.
    #[test]
    fn the_blanked_text_is_the_same_where_the_engines_read_the_same() {
        assert_eq!(
            strip_sql_noise("SELECT /* x */ 1", Engine::Trino),
            "SELECT   1"
        );
        assert_eq!(
            strip_sql_noise("SELECT 'a' , \"b\" , `c`", Engine::Trino),
            "SELECT   ,   ,  "
        );
        assert_eq!(
            strip_sql_noise("SELECT 'unterminated", Engine::Trino),
            "SELECT  "
        );
    }
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
