//! The read-only guard compared with a real `ClickHouse`.
//!
//! The lexer follows what `ClickHouse` 26.8 is measured to do and refuses
//! what no measurement covers. This corpus is the primary
//! evidence that the two agree: every row goes to a real engine as
//! `EXPLAIN AST`, which parses the statement and never runs it (its root
//! node names the real statement kind, and a parse error answers error
//! code 62 — the same call the assistant's `dry_run_sql` makes), and the
//! guard's verdict is compared with the engine's own reading of the same
//! text.
//!
//! Measured facts behind the rows (2026-11-10,
//! `clickhouse/clickhouse-server:26.8`, version 26.8.15.10, image digest
//! `sha256:3043f691ec1a847f38b446ff43708893fcbf9815bd9bd88c0f84bfe064ce852f`):
//! the engine refuses every adjacent-`$`-run shape with `SYNTAX_ERROR`
//! (code 62), `$a b$` is no heredoc, `$1$x$1$` is, `$a-b$x$a-b$` parses
//! as identifiers, and `//` is a line comment. Each row's comment names
//! the measurement it pins.
//!
//! The container is stopped when the test ends (dropping it removes it),
//! like `tests/ops_workloads.rs`' harness, which this file is modeled on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_api::query_params::substitute;
use lakehouse_api::sql_guard::is_read_only;
use lakehouse_api::sql_rewrite::Engine;
use lakehouse_clickhouse::ChClient;
use serde_json::{Map, Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;

/// What the corpus claims about one statement, and what the test then
/// asserts on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The engine parses the statement as a SELECT and the guard answers
    /// read-only — no false refusals of measured-good SQL.
    SelectRead,
    /// The engine parses the statement, but not as a SELECT (the
    /// SHOW/DESCRIBE/EXPLAIN forms), and the guard answers read-only.
    NonSelectRead,
    /// The engine parses a write (its AST root is not a select), and the
    /// guard must refuse it.
    WriteRefused,
    /// The engine refuses the statement — a syntax error, or the HTTP
    /// interface's refusal of a second statement — and the guard refuses
    /// it too: the review and measurement shapes the plan pins.
    RefusedBoth,
    /// The engine parses the statement as a SELECT, and the guard refuses
    /// it anyway: the guard cannot follow what the engine reads there, so
    /// it fails closed (`#452` review SHOULD-FIX, the Unicode quotes).
    SelectGuardRefused,
    /// The engine refuses the statement and the guard's verdict is not
    /// asserted: the refusal happens at the engine, where the author sees
    /// it, and no measurement says what the guard should answer.
    RefusedEngineOnly,
}

/// The fixed statements: the guard cases of `routes/query.rs` and
/// `sql_lex.rs`, the two `#446` review shapes, and the shapes measured on
/// the engine (2026-11-10). The scanner's `NASTY` values join at run time
/// (see `nasty_rows` below), printed the way the scanner prints them.
const CORPUS: &[(&str, Expect)] = &[
    // ── the allowed keyword forms (`routes/query.rs` guard tests) ───────
    ("SELECT 1", Expect::SelectRead),
    ("with x as (select 1) select * from x", Expect::SelectRead),
    ("SeLeCt 1", Expect::SelectRead),
    ("   \n\t SELECT 1", Expect::SelectRead),
    ("SELECT createdAt FROM t", Expect::SelectRead),
    ("SELECT updated_at FROM t", Expect::SelectRead),
    ("-- Write SQL here\nSELECT 1", Expect::SelectRead),
    ("/* c */ SELECT 1", Expect::SelectRead),
    ("# hash comment\nSELECT 1", Expect::SelectRead),
    ("SELECT 1 -- drop this later", Expect::SelectRead),
    ("/* TODO: delete */ SELECT 1", Expect::SelectRead),
    ("SELECT * FROM t WHERE action = 'drop'", Expect::SelectRead),
    ("SELECT 'it''s an update' AS note", Expect::SelectRead),
    ("SELECT 'drop' AS a", Expect::SelectRead),
    // A write hidden behind a construct the engine itself reads as
    // harmless, and the engine agrees: these parse as SELECTs.
    ("SELECT 1 /* /* */ ; DROP TABLE t */", Expect::SelectRead),
    ("SELECT 1 # ; DROP TABLE t", Expect::SelectRead),
    ("SELECT $$DROP TABLE t$$", Expect::SelectRead),
    (
        // The backslash escapes the quote on `ClickHouse`, so the whole
        // rest is one string and nothing behind it runs — the engine
        // parses one SELECT over one literal.
        "SELECT 'a\\'; DROP TABLE t; SELECT '",
        Expect::SelectRead,
    ),
    // ── the SHOW/DESCRIBE/EXPLAIN forms the guard admits ────────────────
    ("SHOW TABLES", Expect::NonSelectRead),
    ("DESCRIBE t", Expect::NonSelectRead),
    ("DESC t", Expect::NonSelectRead),
    ("EXPLAIN SELECT 1", Expect::NonSelectRead),
    // ── writes the engine parses, and the guard refuses ─────────────────
    ("INSERT INTO x VALUES (1)", Expect::WriteRefused),
    ("DROP TABLE x", Expect::WriteRefused),
    ("TRUNCATE TABLE x", Expect::WriteRefused),
    ("CREATE TABLE x (id UInt8)", Expect::WriteRefused),
    ("ALTER TABLE x UPDATE y = 1 WHERE 1", Expect::WriteRefused),
    ("RENAME TABLE x TO y", Expect::WriteRefused),
    ("GRANT SELECT ON t TO u", Expect::WriteRefused),
    // ── second statements: the HTTP interface refuses them, and so does
    // the guard ──────────────────────────────────────────────────────────
    ("SELECT 1 AS a; DELETE FROM t", Expect::RefusedBoth),
    ("SELECT 'drop' AS a; DROP TABLE t", Expect::RefusedBoth),
    // The tail of an unclosed heredoc is code to the engine (it opens no
    // heredoc), so the `DROP` in it would run: the guard refuses, and the
    // HTTP interface refuses the two statements anyway.
    ("SELECT 1$b$ ; DROP TABLE t", Expect::RefusedBoth),
    // ── the two `#446` review shapes ─────────────────────────────────────
    (
        "SELECT $$x$$$$ ' $$; DROP TABLE t; SELECT 'z'",
        Expect::RefusedBoth,
    ),
    ("SELECT $$x$$$$ {{p}} $$", Expect::RefusedBoth),
    // ── the shapes measured on the engine ───────────────────────────────
    // (1) `SYNTAX_ERROR` at the second `$`-run.
    ("SELECT $$x$$$$y$$", Expect::RefusedBoth),
    // (2) `SYNTAX_ERROR` at the second `$`-run.
    ("SELECT $$x$$$$ ' $$", Expect::RefusedBoth),
    // (3) `$a b$` is no heredoc (the tag scan stops at the space); the
    // statement is all identifiers to the lexer and `SYNTAX_ERROR` to the
    // engine. No measurement says what the guard should answer.
    ("SELECT $a b$x$a b$", Expect::RefusedEngineOnly),
    // (4) `SYNTAX_ERROR` at the first `$`.
    ("SELECT $'$x$'$", Expect::RefusedBoth),
    // (5) "Unrecognized token" at the non-ASCII character.
    ("SELECT $é$x$é$", Expect::RefusedBoth),
    // (6) a heredoc with the digit tag `1`; runs and answers `x`.
    ("SELECT $1$x$1$", Expect::SelectRead),
    // (7) parses as `minus` over the identifiers `$a`, `b$x$a`, `b$`.
    ("SELECT $a-b$x$a-b$", Expect::SelectRead),
    // (8) `//` is a line comment; the operand is `1` alone.
    ("SELECT 1 // c", Expect::SelectRead),
    // (9) no space needed after `//`.
    ("SELECT 1 //c", Expect::SelectRead),
    // (10) not integer division: a comment.
    ("SELECT 8 // 3", Expect::SelectRead),
    // (11) the `'` is inside the line comment; the next line is code.
    ("SELECT 1 // '\n, 2", Expect::SelectRead),
    // (12) a lone `/` is an operator; `--` comments across newlines are
    // ordinary. Measured: `divide(1, 2)`.
    ("SELECT 1 /\n-- c\n2", Expect::SelectRead),
    // (13) `$$abc$$` runs; `abc` after the closing `$` is an alias.
    ("SELECT $$abc$$", Expect::SelectRead),
    ("SELECT $$x$$abc", Expect::SelectRead),
    // (14) the control of the review family: two adjacent literals are a
    // plain syntax error at the engine; the guard blanks both and allows.
    ("SELECT $$x$$ 'x'", Expect::RefusedEngineOnly),
    // A comment does not join two operands on `ClickHouse`: refused there,
    // allowed here — the refusal happens at the engine.
    ("SELECT 1 /* c */ 2", Expect::RefusedEngineOnly),
    // The `#437` shapes, still measured here: a `$` after a non-ASCII
    // character opens a heredoc the tag scan finds, but the engine refuses
    // the statement at that character before the heredoc decides anything.
    ("SELECT 'x' AS aé$$b", Expect::RefusedEngineOnly),
    ("SELECT 'x' AS aé$$b$$", Expect::RefusedEngineOnly),
    // An identifier that holds a `$` is one name; the engine answers.
    ("SELECT 1 AS a$$b", Expect::SelectRead),
    ("SELECT 1 AS a$tag$b", Expect::SelectRead),
    // The heredoc after a number is a heredoc to the engine (`#437`), so
    // the `concat` call is a syntax error whose token is the heredoc.
    ("SELECT concat('x', 1$$a$$)", Expect::RefusedEngineOnly),
    // A closed heredoc is a literal to the engine and blanked by the
    // guard; the placeholder inside it is refused by the scanner (pinned
    // in `query_params`' tests), not by this guard.
    ("SELECT $$ {{x}} $$", Expect::SelectRead),
    // ── the Unicode quotes (`#452` review SHOULD-FIX) ───────────────────
    // Measured 2026-11-12 on `clickhouse/clickhouse-server:26.8`: `‘ … ’`
    // is a string literal and `“ … ”` a quoted identifier, so the `'`
    // inside them opens no string. The lexer cannot follow them, so the
    // guard refuses these statements on ClickHouse although the engine
    // parses the first three as one SELECT each (`EXPLAIN AST` root
    // `SelectWithUnionQuery`).
    // The engine reads the literal `‘ ' ’` and then the comment `--'`.
    (
        "SELECT \u{2018} ' \u{2019} AS a, 2 AS b --'",
        Expect::SelectGuardRefused,
    ),
    // The engine reads the literals `‘ '’` and `‘' ’` around `3 AS c`.
    (
        "SELECT \u{2018} '\u{2019}, 3 AS c, \u{2018}' \u{2019}",
        Expect::SelectGuardRefused,
    ),
    // The engine reads the quoted identifier `“ ' ”` and then the comment
    // `--'`.
    (
        "SELECT \u{201C} ' \u{201D}, 2 AS b --'",
        Expect::SelectGuardRefused,
    ),
    // The review shape (2026-11-12, same image): the engine refuses the
    // statement. The lexer does not follow the quotes: it marks them
    // uncertain, so the guard refuses the statement. A lexer that read a
    // string around the `DROP` let it pass.
    (
        "SELECT \u{2018} ' \u{2019}; DROP TABLE t; SELECT \u{2018} ' \u{2019}",
        Expect::RefusedBoth,
    ),
    // ── the `NASTY` values are appended at run time, printed ────────────
];

/// The two templates the scanner's `NASTY` values are filled into: the
/// printed value is a quoted literal, so every printed statement parses
/// and the guard must answer read-only for all of them.
const NASTY_TEMPLATES: &[&str] = &["SELECT {{v}} AS v", "SELECT 1 AS one WHERE n = {{v}}"];

/// The scanner's own injection corpus (`query_params.rs`), printed the way
/// the printer prints it — the only route a value takes into a statement.
const NASTY: [&str; 16] = [
    "\\",
    "a\\",
    "\\a",
    "a\\\\b",
    "x\\' OR '1'='1",
    "'; DROP TABLE x; --\\",
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

/// How the engine reads one statement, from `EXPLAIN AST`.
#[derive(Debug)]
enum Verdict {
    /// The AST root is `SelectWithUnionQuery`.
    Select,
    /// The engine parsed the statement; the first line of the dump names
    /// the root node's kind.
    Parsed(String),
    /// The engine refused the statement (a parse error, or the HTTP
    /// interface's refusal of a second statement).
    Refused,
}

/// A disposable `ClickHouse` 26.8 — the compose file's own tag.
struct EngineServer {
    /// Kept alive for the test: dropping it removes the container.
    _container: testcontainers::ContainerAsync<ClickHouse>,
    /// The HTTP client the corpus speaks to the engine with.
    client: ChClient,
}

impl EngineServer {
    async fn start() -> Self {
        // The compose service's env trio (the compose file's own local-dev
        // default), as `tests/ops_workloads.rs` starts it.
        let container = ClickHouse::default()
            .with_env_var("CLICKHOUSE_USER", "default")
            .with_env_var("CLICKHOUSE_PASSWORD", "")
            .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
            .with_tag("26.8")
            .start()
            .await
            .expect("start the ClickHouse testcontainer");
        let host = container.get_host().await.expect("container host");
        let port = container
            .get_host_port_ipv4(8123)
            .await
            .expect("container HTTP port");
        let client = ChClient::new(
            format!("http://{host}:{port}"),
            "default".to_owned(),
            String::new(),
        );
        Self {
            _container: container,
            client,
        }
    }
}

/// `EXPLAIN AST` never runs the statement; a parse error is a transport or
/// server error from the client, and the dump's first line names the root.
async fn explain(client: &ChClient, sql: &str) -> Verdict {
    match client.raw_bytes(&format!("EXPLAIN AST {sql}"), None).await {
        Err(_) => Verdict::Refused,
        Ok(body) => {
            let text = String::from_utf8_lossy(&body);
            let first = text.lines().next().unwrap_or("").trim().to_owned();
            if first.starts_with("SelectWithUnionQuery") {
                Verdict::Select
            } else {
                Verdict::Parsed(first)
            }
        }
    }
}

fn nasty_rows() -> Vec<(String, Expect)> {
    let mut rows = Vec::new();
    for value in NASTY {
        let params: Map<String, Value> = [("v".to_owned(), json!(value))].into_iter().collect();
        for template in NASTY_TEMPLATES {
            let printed = substitute(Engine::ClickHouse, template, &params, &[])
                .unwrap_or_else(|e| panic!("the printer refused its own corpus value: {e}"));
            rows.push((printed, Expect::SelectRead));
        }
    }
    rows
}

/// The corpus: for every row, the engine's own reading of the statement
/// (from `EXPLAIN AST` on a real `ClickHouse` 26.8) must agree with the
/// guard's verdict, per the row's expectation.
#[tokio::test]
async fn the_guard_agrees_with_a_real_clickhouse_on_every_corpus_row() {
    let server = EngineServer::start().await;
    let mut rows: Vec<(String, Expect)> = CORPUS
        .iter()
        .map(|(sql, expect)| ((*sql).to_owned(), *expect))
        .collect();
    rows.extend(nasty_rows());

    let mut failures: Vec<String> = Vec::new();
    for (sql, expect) in &rows {
        let verdict = explain(&server.client, sql).await;
        let guard = is_read_only(sql, Engine::ClickHouse);
        let engine = match &verdict {
            Verdict::Select => "select".to_owned(),
            // The dump's first line names the root node's kind.
            Verdict::Parsed(root) => format!("parsed: {root}"),
            Verdict::Refused => "refused".to_owned(),
        };
        let agreed = match expect {
            Expect::SelectRead => matches!(verdict, Verdict::Select) && guard,
            Expect::NonSelectRead => matches!(verdict, Verdict::Parsed(_)) && guard,
            Expect::WriteRefused => matches!(verdict, Verdict::Parsed(_)) && !guard,
            Expect::RefusedBoth => matches!(verdict, Verdict::Refused) && !guard,
            Expect::SelectGuardRefused => matches!(verdict, Verdict::Select) && !guard,
            Expect::RefusedEngineOnly => matches!(verdict, Verdict::Refused),
        };
        if !agreed {
            failures.push(format!(
                "{expect:?} | guard={guard} | engine={engine} | {sql}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus rows disagree with the engine:\n{}",
        failures.len(),
        rows.len(),
        failures.join("\n")
    );
}
