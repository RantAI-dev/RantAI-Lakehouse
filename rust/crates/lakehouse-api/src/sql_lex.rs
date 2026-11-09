//! The one lexer the console reads SQL with (`SEC-62`).
//!
//! # Why one lexer, and not one per reader
//!
//! Two scanners read the SQL a person wrote: the read-only guard
//! ([`crate::sql_guard`]) and the parameter scanner
//! ([`crate::query_params`]). Each had its own copy of the lexical rules,
//! and neither copy was the engine's. A copy that disagrees with the
//! engine is a hole and not a nuisance: text the scanner believes is code
//! can be code the engine reads.
//!
//! - The guard read a `\` as an escape on `Trino` and knew no `ClickHouse`
//!   heredoc, so a statement could hide a write from its keyword scan.
//! - The scanner refused to open a heredoc after a digit, so `SELECT 1$$
//!   {{x}} $$` was a heredoc to the engine and ordinary text to the
//!   scanner: the placeholder was filled, and a value holding `$$` closed
//!   the heredoc and ran as SQL.
//!
//! Both are `SEC-62`, and both came out of the planner's post-merge
//! reviews of PR #420 and PR #437. So the rules live here once, and
//! both readers take them from here (AGENTS.md rule 4: a duplicated guard
//! is a finding).
//!
//! # The rules, per engine
//!
//! ## `ClickHouse`
//!
//! The rules this module implements, measured on
//! `clickhouse/clickhouse-server:26.8.15.10` and read out of `ClickHouse`'s
//! own lexer (`src/Parsers/Lexer.cpp`):
//!
//! - `'…'`, `"…"` and `` `…` `` are quoted runs. A `\` escapes the next
//!   character, and a doubled quote is an escaped quote.
//! - `--` starts a line comment. `#` starts one only when the next
//!   character is a space or `!`; every other `#` is an "Unrecognized
//!   token" error. `Lexer.cpp` says so: "For less ambiguity, we will
//!   recognize a comment only if # is followed by whitespace. or #! as a
//!   special case for "shebang"."
//! - `/* … */` comments nest: the comment runs to the `*/` that closes the
//!   outermost one. The `#437` review read the nesting rule out of
//!   `ClickHouse`'s own lexer.
//! - `$tag$ … $tag$` is a heredoc: a literal whose body is raw text, read
//!   as it stands and not read as SQL again. Measured: `SELECT $$'; SELECT
//!   42; SELECT '$$` answers the literal `'; SELECT 42; SELECT '` and does
//!   not run the `SELECT 42`.
//! - A heredoc opens only when its closing delimiter is found again in the
//!   text: `src/Parsers/Lexer.cpp` (the `$` branch of `nextTokenImpl`)
//!   emits a heredoc token only then, and when the delimiter is not found
//!   again the `$` opens nothing — it is a standalone `$` or the start of
//!   one bareword, and the text after it is ordinary SQL to the engine.
//!   Measured: `SELECT 1$b$` answers `1` (the run is an alias), and
//!   `SELECT 1$b$ ; DROP TABLE t` is two statements to the engine.
//! - A `$` opens a heredoc only where it starts a token. The run of
//!   `[A-Za-z0-9_$]` before it is a name when it starts with a letter, `_`
//!   or `$` — the engine's bareword reader takes those — and a number
//!   otherwise, so the `$` opens the heredoc. Measured: `SELECT 'x' AS
//!   a$$b` answers `x` and `SELECT 'x' AS _a$$b$$` is the identifier
//!   `_a$$b$$`, while `1$$`, `1.5$$`, `0x1F$$` and `1a$$` each open a
//!   heredoc. (`#437` review)
//!
//! ## `Trino`
//!
//! From `Trino`'s own grammar
//! (`core/trino-parser/src/main/antlr4/io/trino/sql/parser/SqlBase.g4`)
//! and <https://trino.io/docs/current/language/comments.html>:
//!
//! - `STRING_`: `'…'`, where the only escape is a doubled quote. A `\` is
//!   a plain character.
//! - `QUOTED_IDENTIFIER_`: `"…"`, with a doubled quote as the only escape.
//! - `SIMPLE_COMMENT_`: `-- …` to the end of the line.
//! - `BRACKETED_COMMENT_`: `/* … */`, which does not nest.
//! - No heredoc, and no `#` comment: a `#` is a character the engine does
//!   not read as a comment start, so the text after it cannot run.
//! - No backtick quoting. A backtick run is read as a quoted run here all
//!   the same, so a placeholder inside one is refused rather than filled:
//!   such a statement is invalid `Trino` anyway, and refusing is the safe
//!   reading.
//!
//! ## What this module does not implement
//!
//! `ClickHouse`'s lexer reads two forms this module does not, and reading
//! them as ordinary code is the safe direction for both readers — the
//! guard sees more text than the engine runs, and a value the scanner
//! fills into what the engine reads as a comment or a quoted run is
//! printed as a quoted literal, so it cannot escape the comment or the
//! quoted run:
//!
//! - `//` starts a line comment there; this module reads it as ordinary
//!   code. A statement that uses `//` does not parse in the rewrite —
//!   the console's rewrite runs `sqlparser`, whose tokenizer reads `//`
//!   as two division operators for `ClickHouseDialect` (only
//!   `SnowflakeDialect` reads it as a comment) — and the rewrite refuses
//!   it, so it cannot run. The assistant's dry run does send the
//!   statement to the engine as `EXPLAIN AST <sql>`, which explains a
//!   statement and never executes it.
//! - The Unicode quotes `‘ … ’` are a string literal and `“ … ”` a quoted
//!   identifier there; this module reads them as ordinary code.
//!   `sqlparser` has no such rule either, so a statement that uses them
//!   does not parse in the rewrite and is refused before it reaches the
//!   engine.
//!
//! # What the spans say
//!
//! [`lex`] yields the statement's spans in order: the code between two of
//! them is one [`SpanKind::Code`] span, and each comment, quoted run and
//! heredoc is one span of its own kind.
//!
//! - [`SpanKind::Comment`]: a comment the engine reads.
//! - [`SpanKind::Literal`]: a quoted run, or a heredoc that is closed. The
//!   body of a heredoc is raw text, so a placeholder inside one is a
//!   placeholder inside quotes.
//! - [`SpanKind::NotCode`]: text the engine reads as neither code nor a
//!   comment — the run after a `#` that the engine does not read as a
//!   comment start. The engine refuses the statement there, so the text
//!   cannot run: the parameter scanner leaves it alone, and the guard
//!   reads it as it stands.
//! - [`SpanKind::UnclosedHeredoc`]: the tail of a heredoc that is never
//!   closed. It is the one text the two readers read differently, and it
//!   has a kind of its own so that neither reader has to guess.
//!
//! An unclosed literal or comment runs to the end of the text, and so does
//! an unclosed heredoc — but the engine reads that tail as code, and the
//! scanner reads it as raw text.
//!
//! ## The one text the two readers read differently
//!
//! `ClickHouse` opens a heredoc only when the closing delimiter is found
//! again in the text (the rule above, read out of `src/Parsers/Lexer.cpp`).
//! When it is not found again, the `$` opens nothing and the text after it
//! is ordinary SQL to the engine, so a denied keyword in it is a keyword
//! the engine would run.
//!
//! The scanner reads that tail as raw text, as it reads the body of a
//! closed heredoc: a placeholder in it is refused like one inside quotes.
//! The scanner cannot know that the author meant a heredoc, and refusing
//! is the safe reading. The guard must read it as code, as the engine
//! does, or a denied keyword hides in it.

use crate::sql_rewrite::Engine;

/// What the engine reads in one span of a statement.
///
/// One kind is the one text the engine and the scanner read differently:
/// [`SpanKind::UnclosedHeredoc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpanKind {
    /// Text the engine reads as code.
    Code,
    /// A comment the engine reads.
    Comment,
    /// A quoted run: a string literal, a quoted identifier, or a heredoc
    /// that is closed.
    Literal,
    /// Text the engine reads as neither code nor a comment.
    NotCode,
    /// Text the engine reads as code, but that the scanner reads as raw
    /// text: the tail of a heredoc that is never closed.
    ///
    /// `ClickHouse`'s lexer opens a heredoc only when the closing
    /// delimiter is found again in the text (`src/Parsers/Lexer.cpp`, the
    /// `$` branch of `nextTokenImpl`). When it is not found again the `$`
    /// opens nothing: it is a standalone `$` or the start of one bareword,
    /// and the text after it is ordinary SQL to the engine.
    ///
    /// The scanner cannot know that the author meant a heredoc, so it
    /// reads the tail as raw text and refuses a placeholder there as it
    /// refuses one inside quotes. The guard must read it as code, as the
    /// engine does: a denied keyword in it is a keyword the engine would
    /// run.
    UnclosedHeredoc,
}

/// One span of a statement, in character positions.
///
/// The positions are character indices and not byte offsets: the readers
/// work on a `Vec<char>`, and the one that needs byte offsets maps them
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    /// The first character of the span.
    pub(crate) start: usize,
    /// The index just past the last character of the span.
    pub(crate) end: usize,
    /// What the engine reads there.
    pub(crate) kind: SpanKind,
}

/// The spans of `sql`, read the way `engine` reads it.
///
/// The spans are in order and cover the whole text: the code between two
/// of them is one [`SpanKind::Code`] span.
#[must_use]
pub(crate) fn lex(engine: Engine, sql: &str) -> Vec<Span> {
    let chars: Vec<char> = sql.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut code_from = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let span = match c {
            '-' if next == Some('-') => Some((skip_to_line_end(&chars, i), SpanKind::Comment)),
            '/' if next == Some('*') => Some((
                if nests_block_comments(engine) {
                    skip_nested_block_comment(&chars, i)
                } else {
                    skip_block_comment(&chars, i)
                },
                SpanKind::Comment,
            )),
            '#' => Some((skip_to_line_end(&chars, i), hash_kind(engine, next))),
            '\'' | '"' | '`' => Some((
                skip_quoted(&chars, i, c, backslash_escapes(engine)),
                SpanKind::Literal,
            )),
            '$' if has_heredoc(engine) => heredoc_opener(&chars, i).map(|(open, close)| {
                let (end, closed) = skip_heredoc(&chars, open, close);
                let kind = if closed {
                    SpanKind::Literal
                } else {
                    SpanKind::UnclosedHeredoc
                };
                (end, kind)
            }),
            _ => None,
        };
        let Some((end, kind)) = span else {
            i += 1;
            continue;
        };
        if code_from < i {
            spans.push(Span {
                start: code_from,
                end: i,
                kind: SpanKind::Code,
            });
        }
        spans.push(Span {
            start: i,
            end,
            kind,
        });
        code_from = end;
        i = end;
    }
    if code_from < chars.len() {
        spans.push(Span {
            start: code_from,
            end: chars.len(),
            kind: SpanKind::Code,
        });
    }
    spans
}

/// What a `#` and the rest of its line are on `engine`.
///
/// `ClickHouse` reads `#` as the start of a line comment when the next
/// character is a space or `!`, and every other `#` as an "Unrecognized
/// token" error. `Trino` has no `#` comment at all.
fn hash_kind(engine: Engine, next: Option<char>) -> SpanKind {
    match engine {
        Engine::ClickHouse if matches!(next, Some(' ' | '!')) => SpanKind::Comment,
        _ => SpanKind::NotCode,
    }
}

/// Whether a backslash escapes the next character inside a quoted run.
///
/// `ClickHouse` does, so `'a\'` is still open there. `Trino` does not: `'a\'`
/// is a complete string, and the `\` in it is a plain character.
pub(crate) fn backslash_escapes(engine: Engine) -> bool {
    match engine {
        Engine::ClickHouse => true,
        Engine::Trino => false,
    }
}

/// Whether a heredoc (`$$ … $$`) opens a run of raw text, as it does on
/// `ClickHouse`. `Trino` has none.
pub(crate) fn has_heredoc(engine: Engine) -> bool {
    match engine {
        Engine::ClickHouse => true,
        Engine::Trino => false,
    }
}

/// Whether a `/* … */` comment nests, as it does on `ClickHouse`: the
/// comment then runs to the `*/` that closes the outermost one.
pub(crate) fn nests_block_comments(engine: Engine) -> bool {
    match engine {
        Engine::ClickHouse => true,
        Engine::Trino => false,
    }
}

/// Index of the newline that ends the line starting at `from` (the end of
/// the text when the last line has none), so the caller resumes on the
/// newline itself.
pub(crate) fn skip_to_line_end(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// Index just past the `/* … */` comment that opens at `from`. An
/// unterminated comment consumes the rest of the statement.
pub(crate) fn skip_block_comment(chars: &[char], from: usize) -> usize {
    let mut i = from + 2;
    while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
        i += 1;
    }
    (i + 2).min(chars.len())
}

/// Index just past the quoted run that opens at `from` with `quote`.
/// An unterminated quote consumes the rest of the statement, which is the
/// safe reading: the reader sees less, not more.
///
/// `backslash_escapes` says whether a backslash escapes the next character.
/// `ClickHouse` does, `Trino` does not, and [`lex`] passes the engine's own
/// answer.
pub(crate) fn skip_quoted(
    chars: &[char],
    from: usize,
    quote: char,
    backslash_escapes: bool,
) -> usize {
    let mut i = from + 1;
    while i < chars.len() {
        if backslash_escapes && chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            // A doubled quote is an escaped quote, not the end of the run.
            if chars.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// The heredoc delimiter that opens at `from`: `$`, a tag of ASCII letters,
/// digits and `_` (which may be empty), and a closing `$`.
///
/// Returns the range of the delimiter in `chars` and the index just past
/// it, or `None` when the text there opens no heredoc.
///
/// The engine opens a heredoc only where the `$` starts a token. The run of
/// `[A-Za-z0-9_$]` before it is a name when it starts with a letter, `_` or
/// `$` — the engine's bareword reader takes those — and a number otherwise,
/// so the `$` opens the heredoc. Measured on
/// `clickhouse/clickhouse-server:26.8.15.10`: `SELECT 1 AS a$$b` answers
/// `1` and `SELECT 'x' AS _a$$b$$` is the identifier `_a$$b$$`, while
/// `1$$`, `1.5$$`, `0x1F$$` and `1a$$` each open a heredoc. (`#437` review)
fn heredoc_opener(chars: &[char], from: usize) -> Option<(usize, usize)> {
    if chars.get(from) != Some(&'$') {
        return None;
    }
    // The run of name characters before the `$`: a name when it starts with
    // a letter, `_` or `$`, a number otherwise.
    let mut name_from = from;
    while name_from > 0 && is_name_char(chars[name_from - 1]) {
        name_from -= 1;
    }
    if name_from < from {
        let first = chars[name_from];
        if first.is_ascii_alphabetic() || first == '_' || first == '$' {
            return None;
        }
    }
    let mut at = from + 1;
    while at < chars.len() && (chars[at].is_ascii_alphanumeric() || chars[at] == '_') {
        at += 1;
    }
    if chars.get(at) != Some(&'$') {
        return None;
    }
    Some((from, at + 1))
}

/// Whether `c` is one of the characters the engine's bareword reader takes:
/// letters, digits, `_` and `$`.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Index just past the heredoc that opens with the delimiter
/// `chars[open..close]`, and whether the closing delimiter was found.
///
/// The closing delimiter is the opening one, as the engine reads it. A
/// heredoc that is never closed consumes the rest of the statement, which
/// is the scanner's safe reading: the rest of the statement is inside it.
/// The engine reads that tail as code, so the caller gives the span the
/// kind that says so.
fn skip_heredoc(chars: &[char], open: usize, close: usize) -> (usize, bool) {
    let delimiter = &chars[open..close];
    let mut at = close;
    while at < chars.len() {
        if chars[at..].starts_with(delimiter) {
            return (at + delimiter.len(), true);
        }
        at += 1;
    }
    (chars.len(), false)
}

/// Index just past the `/* … */` comment that opens at `from`, counting the
/// nested ones: `ClickHouse` nests block comments, so the comment ends at
/// the `*/` that closes the outermost one. An unterminated comment
/// consumes the rest of the statement.
fn skip_nested_block_comment(chars: &[char], from: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < chars.len() {
        match (chars[i], chars.get(i + 1)) {
            ('/', Some('*')) => {
                depth += 1;
                i += 2;
            }
            ('*', Some('/')) => {
                depth = depth.saturating_sub(1);
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    chars.len()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Engine, SpanKind, lex};

    /// The spans of `sql`, as `(kind, text)` pairs.
    fn spans(engine: Engine, sql: &str) -> Vec<(SpanKind, String)> {
        let chars: Vec<char> = sql.chars().collect();
        lex(engine, sql)
            .into_iter()
            .map(|span| {
                let text: String = chars[span.start..span.end].iter().collect();
                (span.kind, text)
            })
            .collect()
    }

    #[test]
    fn a_clickhouse_heredoc_after_a_number_is_one_literal() {
        // The `1` is code: the heredoc opens at the `$`, which starts a
        // token there because a number holds no `$`.
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT 1$$ body $$"),
            vec![
                (SpanKind::Code, "SELECT 1".to_owned()),
                (SpanKind::Literal, "$$ body $$".to_owned()),
            ]
        );
    }

    #[test]
    fn a_closed_clickhouse_heredoc_with_a_tag_is_one_literal() {
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT $t$ body $t$"),
            vec![
                (SpanKind::Code, "SELECT ".to_owned()),
                (SpanKind::Literal, "$t$ body $t$".to_owned()),
            ]
        );
    }

    /// A heredoc tag is word characters only: the tag scan stops at the
    /// space, so `$a b$` opens no heredoc and the text stays ordinary
    /// code.
    #[test]
    fn a_heredoc_tag_holds_word_characters_only() {
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT $a b$ x"),
            vec![(SpanKind::Code, "SELECT $a b$ x".to_owned())]
        );
    }

    /// `ClickHouse` opens a heredoc only when the closing delimiter is
    /// found again in the text: `src/Parsers/Lexer.cpp` (the `$` branch of
    /// `nextTokenImpl`) emits a heredoc token only then, and when the
    /// delimiter is not found again the `$` opens nothing: it is a
    /// standalone `$` or the start of one bareword, and the text after it
    /// is ordinary SQL to the engine.
    ///
    /// Measured on `clickhouse/clickhouse-server:26.8.15.10`, where
    /// `SELECT 1$b$` answers `1` and `SELECT 1$b$ ; DROP TABLE t` is two
    /// statements to the engine.
    #[test]
    fn an_unclosed_clickhouse_heredoc_is_the_tail_the_engine_reads_as_code() {
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT 1$b$ ; DROP TABLE t"),
            vec![
                (SpanKind::Code, "SELECT 1".to_owned()),
                (SpanKind::UnclosedHeredoc, "$b$ ; DROP TABLE t".to_owned()),
            ]
        );
        // `Trino` has no heredoc at all, so the same text is code there.
        assert_eq!(
            spans(Engine::Trino, "SELECT 1$b$ ; DROP TABLE t"),
            vec![(SpanKind::Code, "SELECT 1$b$ ; DROP TABLE t".to_owned())]
        );
    }

    #[test]
    fn a_name_that_holds_a_dollar_sign_is_code() {
        assert_eq!(
            spans(Engine::ClickHouse, "a$$b"),
            vec![(SpanKind::Code, "a$$b".to_owned())]
        );
    }

    #[test]
    fn a_hash_is_a_comment_on_clickhouse_only_after_a_space_or_bang() {
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT 1 # c"),
            vec![
                (SpanKind::Code, "SELECT 1 ".to_owned()),
                (SpanKind::Comment, "# c".to_owned()),
            ]
        );
        // `#!` is the engine's shebang form of the comment.
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT 1 #!c"),
            vec![
                (SpanKind::Code, "SELECT 1 ".to_owned()),
                (SpanKind::Comment, "#!c".to_owned()),
            ]
        );
        assert_eq!(
            spans(Engine::ClickHouse, "SELECT 1 #c"),
            vec![
                (SpanKind::Code, "SELECT 1 ".to_owned()),
                (SpanKind::NotCode, "#c".to_owned()),
            ]
        );
        assert_eq!(
            spans(Engine::Trino, "SELECT 1 # c"),
            vec![
                (SpanKind::Code, "SELECT 1 ".to_owned()),
                (SpanKind::NotCode, "# c".to_owned()),
            ]
        );
    }

    #[test]
    fn a_block_comment_nests_on_clickhouse_and_not_on_trino() {
        assert_eq!(
            spans(Engine::ClickHouse, "/* a /* b */ c */"),
            vec![(SpanKind::Comment, "/* a /* b */ c */".to_owned())]
        );
        assert_eq!(
            spans(Engine::Trino, "/* a /* b */"),
            vec![(SpanKind::Comment, "/* a /* b */".to_owned())]
        );
    }

    #[test]
    fn a_backslash_escapes_a_quote_on_clickhouse_and_not_on_trino() {
        assert_eq!(
            spans(Engine::ClickHouse, "'a\\' b'"),
            vec![(SpanKind::Literal, "'a\\' b'".to_owned())]
        );
        assert_eq!(
            spans(Engine::Trino, "'a\\' b'"),
            vec![
                (SpanKind::Literal, "'a\\'".to_owned()),
                (SpanKind::Code, " b".to_owned()),
                // The last quote opens a run that is never closed.
                (SpanKind::Literal, "'".to_owned()),
            ]
        );
    }
}
