//! Who may use a dashboard's signed embed, and who may frame it (`SEC-12`).
//!
//! Three things are kept with the board (`console.bi_board`, three columns
//! beside `embed_enabled`):
//!
//! - **`revoked_before`**: "withdraw all". A Unix-seconds instant; a token
//!   whose `iat` is at or before it is refused.
//! - **`withdrawn`**: "withdraw one". A small list of `(jti, exp)`. A token
//!   carrying a listed `jti` is refused for this board only. An entry is
//!   dropped once the token could no longer be accepted anyway, which is
//!   `exp` plus the clock tolerance, not `exp` alone: the verifier accepts a
//!   token up to 60 seconds past its `exp`, so dropping the record at `exp`
//!   would let the token back in for that minute.
//! - **`origins`**: the sites allowed to frame the embed pages.
//!
//! # Why on the board
//!
//! The board is read with `FINAL` on every `POST /api/embed/data`, so a
//! withdrawal written by the Share dialog is seen by the next request. There
//! is no cache in between (see the plan, section 7). Putting the state in
//! the board row also means deleting a board deletes its withdrawals.
//!
//! # Limits
//!
//! `revoked_before` is the API server's clock at the moment of withdrawal,
//! compared with the `iat` the customer's server wrote. A token minted by a
//! clock that runs ahead of ours by up to the 60-second tolerance carries an
//! `iat` later than the withdrawal and survives "withdraw all" until it
//! expires; the maximum lifetime bounds that. Withdraw one by `jti` is
//! exact.

use serde::{Deserialize, Serialize};

/// At most this many allowed origins per dashboard.
pub const MAX_ORIGINS: usize = 20;

/// At most this many individually withdrawn tokens kept per dashboard. The
/// list only needs to hold tokens that are still within their lifetime; past
/// this, "withdraw all" is the right tool and the caller is told so.
pub const MAX_WITHDRAWN_TOKENS: usize = 1_000;

/// One individually withdrawn token: its id and when it stops being
/// acceptable anyway (its `exp`, Unix seconds, rounded up).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawnToken {
    /// The token's `jti`.
    pub jti: String,
    /// The token's own `exp`, Unix seconds.
    pub exp: u64,
}

/// A board's embed access state. `Default` is "nothing withdrawn, no site
/// may frame it".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbedAccess {
    /// Tokens with `iat` at or before this (Unix seconds) are refused.
    pub revoked_before: u64,
    /// Individually withdrawn tokens.
    pub withdrawn: Vec<WithdrawnToken>,
    /// Sites allowed to frame the embed pages, normalised.
    pub origins: Vec<String>,
}

impl EmbedAccess {
    /// Rebuild the state from the three stored columns.
    ///
    /// Fails closed (principle 3): a withdrawn-tokens column that does not
    /// parse is something this code never writes, and ignoring it would
    /// silently re-admit withdrawn tokens, so every token is refused instead
    /// (`revoked_before` is set to its maximum) until the board is saved
    /// again. An origins column that does not parse allows no site.
    #[must_use]
    pub fn from_stored(revoked_before: u64, withdrawn_json: &str, origins_json: &str) -> Self {
        let withdrawn_json = withdrawn_json.trim();
        let (revoked_before, withdrawn) = if withdrawn_json.is_empty() {
            (revoked_before, Vec::new())
        } else {
            match serde_json::from_str::<Vec<WithdrawnToken>>(withdrawn_json) {
                Ok(list) => (revoked_before, list),
                Err(_) => (u64::MAX, Vec::new()),
            }
        };
        let origins = if origins_json.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<String>>(origins_json).unwrap_or_default()
        };
        Self {
            revoked_before,
            withdrawn,
            origins,
        }
    }

    /// Whether a token with this `iat` and `jti` has been withdrawn.
    #[must_use]
    pub fn is_withdrawn(&self, iat: f64, jti: Option<&str>) -> bool {
        // `u64 -> f64` is exact below 2^53, far beyond any instant; the
        // fail-closed `u64::MAX` only has to compare above every real `iat`.
        #[allow(
            clippy::cast_precision_loss,
            reason = "an instant in Unix seconds; the one huge value, u64::MAX, only needs to exceed every iat"
        )]
        let revoked_before = self.revoked_before as f64;
        if iat <= revoked_before {
            return true;
        }
        jti.is_some_and(|jti| self.withdrawn.iter().any(|w| w.jti == jti))
    }

    /// "Withdraw all": raise `revoked_before` to `now` (never lower it).
    #[must_use]
    pub fn revoke_all(mut self, now: u64) -> Self {
        self.revoked_before = self.revoked_before.max(now);
        self
    }

    /// "Withdraw one": add `jti` (valid until `exp`), after dropping the
    /// entries that could no longer be accepted anyway at `now`.
    ///
    /// `tolerance_secs` is the clock tolerance the token verifier applies
    /// (`lakehouse_embed::CLOCK_TOLERANCE_SECONDS`); it is a parameter so
    /// this crate does not depend on `lakehouse-embed`, and the caller owns
    /// the one constant.
    ///
    /// # Errors
    ///
    /// A message for the caller when the list is full. Adding a `jti` that
    /// is already listed is not an error.
    pub fn withdraw_token(
        mut self,
        jti: &str,
        exp: u64,
        now: u64,
        tolerance_secs: u64,
    ) -> Result<Self, String> {
        // `exp + tolerance < now`: past that the verifier refuses the token
        // on its own.
        self.withdrawn
            .retain(|w| w.exp.saturating_add(tolerance_secs) >= now);
        if !self.withdrawn.iter().any(|w| w.jti == jti) {
            if self.withdrawn.len() >= MAX_WITHDRAWN_TOKENS {
                return Err(
                    "too many tokens are withdrawn one by one for this dashboard; withdraw all of them instead."
                        .to_owned(),
                );
            }
            self.withdrawn.push(WithdrawnToken {
                jti: jti.to_owned(),
                exp,
            });
        }
        Ok(self)
    }
}

/// Validate and normalise a list of allowed sites.
///
/// Each entry is `https://host[:port]`, or `http://localhost[:port]` (plain
/// `http` only for `localhost`, for development). No wildcard, path, query,
/// fragment, user-info or IPv6 literal. The host is lower-cased, a port is
/// written without leading zeros, duplicates are dropped, and at most
/// [`MAX_ORIGINS`] remain. The result is safe to place in a
/// `Content-Security-Policy` header: it contains only `[a-z0-9.:/-]`.
///
/// # Errors
///
/// A message naming the first entry that is not acceptable, or the limit.
pub fn validate_origins(raw: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for entry in raw {
        let origin = normalize_origin(entry)?;
        if !out.contains(&origin) {
            out.push(origin);
        }
    }
    if out.len() > MAX_ORIGINS {
        return Err(format!(
            "at most {MAX_ORIGINS} sites can be allowed per dashboard."
        ));
    }
    Ok(out)
}

fn normalize_origin(raw: &str) -> Result<String, String> {
    let shown: String = raw.chars().take(80).collect();
    let bad = |why: &str| Err(format!("\"{shown}\" is not an allowed site: {why}"));
    let entry = raw.trim();
    if entry.contains('*') {
        return bad("wildcards are not accepted; list each site");
    }
    let (scheme, rest) = if let Some(rest) = entry.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = entry.strip_prefix("http://") {
        ("http", rest)
    } else {
        return bad("write it as https://host or https://host:port");
    };
    if rest.is_empty()
        || rest
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':')))
    {
        return bad(
            "write only the scheme, host and optional port, with no path, query, user name or address in brackets",
        );
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    let host = host.to_ascii_lowercase();
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        });
    if !host_ok {
        return bad("the host name is not valid");
    }
    if scheme == "http" && host != "localhost" {
        return bad("http:// is only accepted for localhost; use https://");
    }
    match port {
        None => Ok(format!("{scheme}://{host}")),
        Some(port) => match port.parse::<u16>() {
            Ok(n) if n != 0 && port.chars().all(|c| c.is_ascii_digit()) => {
                Ok(format!("{scheme}://{host}:{n}"))
            }
            _ => bad("the port must be a number from 1 to 65535"),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn accepts_https_hosts_with_and_without_a_port() {
        assert_eq!(
            validate_origins(&strings(&[
                "https://app.customer.example",
                "https://b.test:8443"
            ])),
            Ok(strings(&[
                "https://app.customer.example",
                "https://b.test:8443"
            ]))
        );
    }

    #[test]
    fn http_is_only_accepted_for_localhost() {
        assert_eq!(
            validate_origins(&strings(&["http://localhost", "http://localhost:3000"])),
            Ok(strings(&["http://localhost", "http://localhost:3000"]))
        );
        let err = validate_origins(&strings(&["http://app.customer.example"])).unwrap_err();
        assert!(err.contains("only accepted for localhost"), "{err}");
        assert!(validate_origins(&strings(&["http://127.0.0.1:3000"])).is_err());
    }

    #[test]
    fn wildcards_are_refused() {
        for bad in [
            "https://*.customer.example",
            "*",
            "https://*",
            "*.customer.example",
        ] {
            let err = validate_origins(&strings(&[bad])).unwrap_err();
            assert!(err.contains("wildcard"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_missing_or_unsupported_scheme_is_refused() {
        for bad in [
            "app.customer.example",
            "ftp://a.test",
            "//a.test",
            "",
            "https://",
        ] {
            assert!(validate_origins(&strings(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_path_query_fragment_user_info_or_bracketed_address_is_refused() {
        for bad in [
            "https://a.test/",
            "https://a.test/path",
            "https://a.test?x=1",
            "https://a.test#frag",
            "https://user@a.test",
            "https://[::1]",
            "https://a.test b.test",
            "https://a.test;script-src",
            "https://a.test'",
        ] {
            assert!(validate_origins(&strings(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_bad_port_or_host_is_refused() {
        for bad in [
            "https://a.test:0",
            "https://a.test:65536",
            "https://a.test:abc",
            "https://a.test:",
            "https://a.test:80:80",
            "https://-a.test",
            "https://a..test",
            "https://a_b.test",
        ] {
            assert!(validate_origins(&strings(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hosts_are_lower_cased_ports_normalised_and_duplicates_dropped() {
        assert_eq!(
            validate_origins(&strings(&[
                "https://App.Customer.EXAMPLE:0443",
                "https://app.customer.example:443",
                "https://app.customer.example",
            ])),
            Ok(strings(&[
                "https://app.customer.example:443",
                "https://app.customer.example",
            ]))
        );
    }

    #[test]
    fn at_most_twenty_origins_are_accepted() {
        let twenty: Vec<String> = (0..20).map(|i| format!("https://s{i}.test")).collect();
        assert_eq!(validate_origins(&twenty).unwrap().len(), 20);
        let twenty_one: Vec<String> = (0..21).map(|i| format!("https://s{i}.test")).collect();
        let err = validate_origins(&twenty_one).unwrap_err();
        assert!(err.contains("20"), "{err}");
    }

    #[test]
    fn an_empty_list_is_valid_and_means_no_site_may_frame_the_embed() {
        assert_eq!(validate_origins(&[]), Ok(Vec::new()));
    }

    #[test]
    fn a_token_issued_at_or_before_revoked_before_is_withdrawn_and_a_later_one_is_not() {
        let access = EmbedAccess::default().revoke_all(1_000);
        assert!(access.is_withdrawn(999.0, None));
        assert!(access.is_withdrawn(1_000.0, None));
        assert!(!access.is_withdrawn(1_000.5, None));
        assert!(!access.is_withdrawn(1_001.0, None));
    }

    #[test]
    fn a_fresh_board_withdraws_nothing() {
        assert!(!EmbedAccess::default().is_withdrawn(1.0, Some("j")));
    }

    #[test]
    fn revoke_all_never_lowers_the_instant() {
        let access = EmbedAccess::default().revoke_all(2_000).revoke_all(1_000);
        assert_eq!(access.revoked_before, 2_000);
    }

    #[test]
    fn a_withdrawn_jti_is_refused_and_another_is_not() {
        let access = EmbedAccess::default()
            .withdraw_token("one", 5_000, 1_000, 60)
            .unwrap();
        assert!(access.is_withdrawn(2_000.0, Some("one")));
        assert!(!access.is_withdrawn(2_000.0, Some("two")));
        assert!(!access.is_withdrawn(2_000.0, None));
    }

    #[test]
    fn a_record_is_kept_until_the_token_could_no_longer_be_accepted_anyway() {
        let access = EmbedAccess::default()
            .withdraw_token("one", 5_000, 1_000, 60)
            .unwrap();
        // The verifier accepts a token up to 60 s past `exp`, so at
        // exp + 60 the record must still be there ...
        let still = access
            .clone()
            .withdraw_token("two", 9_000, 5_060, 60)
            .unwrap();
        assert!(still.withdrawn.iter().any(|w| w.jti == "one"));
        // ... and one second later it may go.
        let gone = access.withdraw_token("two", 9_000, 5_061, 60).unwrap();
        assert!(!gone.withdrawn.iter().any(|w| w.jti == "one"));
    }

    #[test]
    fn withdrawing_the_same_jti_twice_keeps_one_record() {
        let access = EmbedAccess::default()
            .withdraw_token("one", 5_000, 1_000, 60)
            .unwrap()
            .withdraw_token("one", 5_000, 1_001, 60)
            .unwrap();
        assert_eq!(access.withdrawn.len(), 1);
    }

    #[test]
    fn the_withdrawn_list_has_a_limit_and_says_to_withdraw_all() {
        let mut access = EmbedAccess::default();
        for i in 0..MAX_WITHDRAWN_TOKENS {
            access = access
                .withdraw_token(&format!("j{i}"), 5_000, 1_000, 60)
                .unwrap();
        }
        let err = access
            .withdraw_token("one-more", 5_000, 1_000, 60)
            .unwrap_err();
        assert!(err.contains("withdraw all"), "{err}");
    }

    #[test]
    fn stored_columns_round_trip() {
        let access =
            EmbedAccess::from_stored(77, r#"[{"jti":"a","exp":9}]"#, r#"["https://a.test"]"#);
        assert_eq!(access.revoked_before, 77);
        assert_eq!(
            access.withdrawn,
            vec![WithdrawnToken {
                jti: "a".to_owned(),
                exp: 9
            }]
        );
        assert_eq!(access.origins, strings(&["https://a.test"]));
    }

    #[test]
    fn a_corrupt_withdrawn_column_fails_closed_and_a_corrupt_origins_column_allows_no_site() {
        let access = EmbedAccess::from_stored(0, "not json", "also not json");
        assert!(
            access.is_withdrawn(4_000_000_000.0, None),
            "every token is refused"
        );
        assert!(access.origins.is_empty());
    }

    #[test]
    fn an_empty_stored_column_is_the_default() {
        assert_eq!(EmbedAccess::from_stored(0, "", ""), EmbedAccess::default());
    }
}
