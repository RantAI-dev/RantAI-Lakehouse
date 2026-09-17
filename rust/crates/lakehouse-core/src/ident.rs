//! Safe SQL identifier and literal newtypes.
//!
//! This is a security hardening over the TypeScript implementation in
//! `src/services/clients/bi-store.ts`, which validates identifiers with a
//! regex test (`IDENT.test(...)`) and escapes values with a hand-rolled
//! `esc()` function at the call site. Here the validation and escaping are
//! baked into the type itself, so a raw, unchecked `String` can never reach
//! a query builder as an identifier or literal.
//!
//! # Scope of the guarantee
//!
//! [`Ident`] guarantees **lexical** safety only — that the string cannot break
//! out of its syntactic position. It does **not** guarantee the identifier is
//! one the caller is allowed to reference. The TypeScript always pairs
//! `IDENT.test(...)` with an existence check against `system.columns`
//! (`bi-store.ts:337, 344, 361, 372, 410`) and callers here must keep doing
//! the same. Concretely, `Ident::new("_part")` succeeds — a leading underscore
//! is legal — and `_part`, `_shard_num`, `_partition_id`, and `_row_exists`
//! are real `ClickHouse` virtual columns that never appear in `system.columns`.
//! Dropping the allowlist because "the type is safe" would expose them.
//!
//! [`Ident`] is also slightly stricter than the TypeScript, which permits a
//! leading digit (`IDENT = /^[a-zA-Z0-9_]+$/`, `bi-store.ts:47`). `ClickHouse`
//! would reject `2024col` unquoted anyway, so the effect is only that the
//! rejection becomes a clean 400 instead of a downstream query failure.

use std::fmt;

use thiserror::Error;

/// A validated SQL identifier: non-empty, ASCII alphanumeric plus `_`, and
/// not starting with a digit. Cannot be constructed from a string that
/// would allow SQL injection through an unquoted identifier position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ident(String);

/// Reasons an [`Ident`] could not be constructed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IdentError {
    /// The input string was empty.
    #[error("identifier must not be empty")]
    Empty,
    /// The input contained a character outside `[A-Za-z0-9_]`.
    #[error("identifier contains illegal character '{0}'")]
    IllegalChar(char),
    /// The input started with a digit.
    #[error("identifier must not start with a digit")]
    LeadingDigit,
}

impl Ident {
    /// Validate and construct an [`Ident`] from any string-like input.
    ///
    /// # Errors
    ///
    /// Returns [`IdentError`] if the input is empty, starts with a digit,
    /// or contains any character other than ASCII letters, digits, or `_`.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentError> {
        let value = value.into();
        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return Err(IdentError::Empty);
        };
        if first.is_ascii_digit() {
            return Err(IdentError::LeadingDigit);
        }
        if !(first.is_ascii_alphanumeric() || first == '_') {
            return Err(IdentError::IllegalChar(first));
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return Err(IdentError::IllegalChar(c));
            }
        }
        Ok(Self(value))
    }

    /// The validated identifier as a plain string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reasons [`split_namespaced_table`] rejected its input.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NamespacedTableError {
    /// No `.` separator was found.
    #[error("expected <namespace>.<table>, no separator found")]
    MissingSeparator,
    /// One or both halves failed [`Ident`] validation.
    #[error(transparent)]
    Invalid(#[from] IdentError),
}

/// Split `raw` on the first `.` and validate each half as a real [`Ident`],
/// returning the two validated substrings on success.
///
/// This guard used to be written twice, two commits apart: once in
/// `lakehouse-api::routes::governance::validate_namespaced_table` (for
/// `PUT /api/governance/sla`'s `tableName`) and once inline in
/// `lakehouse-alerts::normalize_freshness` (for a `Freshness` rule's
/// `mart`-as-target-table). Neither crate could reuse the other's copy —
/// `lakehouse-alerts` cannot depend on `lakehouse-api` (that dependency
/// runs the other way: `lakehouse-api` depends on `lakehouse-alerts`) — so
/// putting the helper in either of them made it unreachable from the
/// other. `lakehouse-core` sits below both, so this is the only place a
/// single copy can live. See AGENTS.md rule 4: a duplicated guard is a
/// finding.
///
/// # Errors
///
/// [`NamespacedTableError::MissingSeparator`] if `raw` has no `.`;
/// [`NamespacedTableError::Invalid`] if either half is not a valid
/// [`Ident`].
pub fn split_namespaced_table(raw: &str) -> Result<(&str, &str), NamespacedTableError> {
    let (namespace, table) = raw
        .split_once('.')
        .ok_or(NamespacedTableError::MissingSeparator)?;
    Ident::new(namespace)?;
    Ident::new(table)?;
    Ok((namespace, table))
}

/// A SQL string literal that escapes itself safely on [`Display`].
///
/// Escaping matches the TypeScript `esc()` function in
/// `src/services/clients/bi-store.ts` exactly: backslashes are doubled
/// first, then single quotes are doubled, so previously-stored values that
/// relied on that escaping order continue to round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlLiteral(String);

impl<T: Into<String>> From<T> for SqlLiteral {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for SqlLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let escaped = self.0.replace('\\', "\\\\").replace('\'', "''");
        write!(f, "'{escaped}'")
    }
}

#[cfg(test)]
mod tests {
    #![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

    use super::*;

    #[test]
    fn ident_accepts_plain_column_name() {
        assert_eq!(Ident::new("tahun").unwrap().as_str(), "tahun");
    }
    #[test]
    fn ident_accepts_underscore_and_digits() {
        assert!(Ident::new("mart_kunjungan_2024").is_ok());
    }
    #[test]
    fn ident_rejects_quote() {
        assert!(Ident::new("tahun'").is_err());
    }
    #[test]
    fn ident_rejects_semicolon_injection() {
        assert!(Ident::new("x; DROP TABLE y").is_err());
    }
    #[test]
    fn ident_rejects_empty() {
        assert!(Ident::new("").is_err());
    }
    #[test]
    fn literal_escapes_single_quote() {
        assert_eq!(SqlLiteral::from("O'Brien").to_string(), "'O''Brien'");
    }
    #[test]
    fn literal_escapes_backslash_before_quote() {
        assert_eq!(SqlLiteral::from(r"a\b'c").to_string(), r"'a\\b''c'");
    }

    #[test]
    fn split_namespaced_table_accepts_a_valid_pair() {
        assert_eq!(
            split_namespaced_table("bronze.orders").unwrap(),
            ("bronze", "orders")
        );
    }

    #[test]
    fn split_namespaced_table_rejects_missing_separator() {
        assert_eq!(
            split_namespaced_table("bronze"),
            Err(NamespacedTableError::MissingSeparator)
        );
    }

    #[test]
    fn split_namespaced_table_rejects_an_injection_shaped_half() {
        assert!(matches!(
            split_namespaced_table("bronze.orders; DROP TABLE dataset_sla;--"),
            Err(NamespacedTableError::Invalid(_))
        ));
    }
}
