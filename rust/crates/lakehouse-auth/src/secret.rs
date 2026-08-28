//! A wrapper that makes "this value must never reach a log line" a type
//! guarantee rather than a convention every call site has to remember.
//!
//! Every password, raw session/service token, and password hash that flows
//! through this crate is wrapped in [`Secret`] the moment it is read from a
//! request or the database. [`Secret`] deliberately does not derive
//! `Debug`/`Display` in the normal way — both are hand-implemented to print
//! a fixed redaction marker — so a `{:?}`/`{}` on a struct that happens to
//! contain one (a `tracing` span, a `dbg!`, an error message built by
//! interpolating a whole request) cannot leak the value, no matter how it
//! got there. [`Secret`] also intentionally has no [`PartialEq`]/[`Eq`]
//! impl: comparing two secrets with `==` is the exact timing side-channel
//! this crate is required not to have, so the type itself refuses it,
//! forcing callers through [`Secret::constant_time_eq`] instead.

use subtle::ConstantTimeEq;

/// A value that must never be rendered, logged, or compared in variable
/// time. See the module doc comment.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wrap a value as a secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Access the wrapped value. Every caller of this must have a specific
    /// reason it needs the raw bytes (hashing, sending over TLS to the
    /// database as a bind parameter, ...) — it is the one intentional
    /// escape hatch in this type, not a general accessor to reach for by
    /// default.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constant-time equality. The only sanctioned way to compare two
    /// secrets: unlike `str`'s `==`, this does not short-circuit on the
    /// first mismatched byte, so how much of the value matched is not
    /// observable from wall-clock time.
    #[must_use]
    pub fn constant_time_eq(&self, other: &Self) -> bool {
        self.0.as_bytes().ct_eq(other.0.as_bytes()).into()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("REDACTED")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::Secret;

    #[test]
    fn debug_never_renders_the_wrapped_value() {
        let secret = Secret::new("hunter2-super-secret-password");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret(REDACTED)");
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn display_never_renders_the_wrapped_value() {
        let secret = Secret::new("hunter2-super-secret-password");
        let rendered = format!("{secret}");
        assert_eq!(rendered, "REDACTED");
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn a_secret_embedded_in_a_larger_debug_struct_still_redacts() {
        #[derive(Debug)]
        struct Wrapper {
            #[allow(dead_code)]
            token: Secret,
        }
        let wrapper = Wrapper {
            token: Secret::new("do-not-leak-me"),
        };
        let rendered = format!("{wrapper:?}");
        assert!(!rendered.contains("do-not-leak-me"));
    }

    #[test]
    fn constant_time_eq_agrees_with_equal_values() {
        let a = Secret::new("same-value");
        let b = Secret::new("same-value");
        assert!(a.constant_time_eq(&b));
    }

    #[test]
    fn constant_time_eq_agrees_with_unequal_values() {
        let a = Secret::new("value-a");
        let b = Secret::new("value-b");
        assert!(!a.constant_time_eq(&b));
    }
}
