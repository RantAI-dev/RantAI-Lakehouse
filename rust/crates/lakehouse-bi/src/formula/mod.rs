//! The formula language of calculated fields (`BI-8`, with `AI-4`): a small
//! language of our own, parsed and compiled by the server into `ClickHouse`
//! SQL. Nothing a person types is ever placed in SQL as it is.
//!
//! # What lives here
//!
//! - [`lex`]: characters to tokens (`[Column Name]`, bare names, numbers,
//!   quoted text, operators), every token with its character position.
//! - [`parse`]: tokens to a syntax tree, with the limits of the language
//!   (2,000 characters, nesting depth 32).
//! - [`catalog`]: the one list of functions (name, signature, category, one
//!   line of help, an example). The compiler, the console's suggestions and
//!   the assistant's tool all read this list; nothing else names a function.
//! - [`compile`]: the type and level checker and the SQL generator in one
//!   pass, so that "the formula checks out" and "this is the SQL" cannot
//!   disagree.
//!
//! # Why the SQL cannot carry the person's text
//!
//! Every piece of the statement is one of: a function name from the fixed
//! table in [`compile`]; a column name that is both a column of the source
//! and a valid [`lakehouse_core::ident::Ident`]; text printed through
//! [`lakehouse_core::ident::SqlLiteral`]; a number re-printed by Rust from
//! its parsed value; a unit word picked from a closed list; punctuation the
//! compiler writes itself. A column is looked up by exact name in the
//! source's own column list, so `[x]` can only ever name something that is
//! already readable through the source (and therefore already goes through
//! the role rewrite of that source). The property tests in [`compile`] feed
//! generated hostile text through the compiler and check these claims on the
//! output.
//!
//! # Limits
//!
//! 2,000 characters; nesting depth 32; a field may reference other fields up
//! to 8 deep, without cycles; and the SQL that references expand into is
//! capped, so a short formula cannot grow into a huge statement.

pub mod catalog;
pub mod compile;
pub mod lex;
pub mod parse;

use serde::Serialize;

/// A formula that does not check out: what is wrong and where.
///
/// `position` and `length` count characters (not bytes) from the start of the
/// formula, so the console can mark the span under the box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormulaError {
    /// A sentence for the person who wrote the formula.
    pub message: String,
    /// Character offset of the start of the offending span.
    pub position: usize,
    /// Characters in the span (at least 1).
    pub length: usize,
}

impl FormulaError {
    /// An error at `position` covering `length` characters.
    #[must_use]
    pub fn at(message: impl Into<String>, position: usize, length: usize) -> Self {
        Self {
            message: message.into(),
            position,
            length: length.max(1),
        }
    }
}

impl std::fmt::Display for FormulaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at character {})", self.message, self.position + 1)
    }
}

/// Shorten text that is echoed back in a message, so a pasted wall of text
/// does not become a wall of error.
pub(crate) fn clip(text: &str) -> String {
    const MAX: usize = 40;
    if text.chars().count() <= MAX {
        text.to_owned()
    } else {
        let head: String = text.chars().take(MAX).collect();
        format!("{head}...")
    }
}
