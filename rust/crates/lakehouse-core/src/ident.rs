//! Safe SQL identifier and literal newtypes.
//!
//! This is a security hardening over the TypeScript implementation in
//! `src/services/clients/bi-store.ts`, which validates identifiers with a
//! regex test (`IDENT.test(...)`) and escapes values with a hand-rolled
//! `esc()` function at the call site. Here the validation and escaping are
//! baked into the type itself, so a raw, unchecked `String` can never reach
//! a query builder as an identifier or literal.

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
}
