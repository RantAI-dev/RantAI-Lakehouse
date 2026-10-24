//! How long a dashboard or a chart may be served from remembered results
//! (`BI-4` part B, feature page
//! `docs/core/features/result-caching-and-parallel-tiles.md`, decision 2).
//!
//! One rule type for the three places it is set (the install, a dashboard, a
//! chart). The narrowest one set wins; "Follow the default" is the absence of
//! a rule at that level. What the default does (a result is used until the
//! tables under it change, and never longer than 24 hours) is the cache's
//! business (`lakehouse-api` `result_cache`); this module only says what a
//! person may write and checks it, so the console, the assistant and the
//! stored JSON all refuse the same values with the same sentence.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

/// The fewest minutes "Remember for N minutes" accepts.
pub const MIN_MINUTES: u32 = 1;
/// The most minutes it accepts: 24 hours, the cache's own outer bound.
pub const MAX_MINUTES: u32 = 24 * 60;

/// What a person is told when the minutes are outside the range.
pub const MINUTES_REFUSED: &str = "Remember for between 1 minute and 24 hours (1 to 1440 minutes).";

/// What a person is told when the rule is not one of the three.
pub const RULE_REFUSED: &str =
    "The freshness rule must be default, fresh, or minutes with a number of minutes.";

/// One freshness rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheRule {
    /// "Follow the default": the level above decides (a dashboard follows the
    /// install, a chart follows its dashboard; the install's own default is
    /// the signed rule of decision 1).
    #[default]
    Follow,
    /// "Always fresh": every read computes, nothing is remembered.
    Fresh,
    /// "Remember for N minutes" (1 to 1440): used while younger than this and
    /// while the tables under it are unchanged.
    Minutes(u32),
}

impl CacheRule {
    /// The rule for the wire's `mode` and `minutes`.
    ///
    /// # Errors
    ///
    /// [`MINUTES_REFUSED`] when `minutes` is outside 1 to 1440 or sent with a
    /// mode that takes none, [`RULE_REFUSED`] for any other mode.
    pub fn parse(mode: &str, minutes: Option<i64>) -> Result<Self, String> {
        match (mode, minutes) {
            ("default", None) => Ok(Self::Follow),
            ("fresh", None) => Ok(Self::Fresh),
            ("minutes", Some(n)) => u32::try_from(n)
                .ok()
                .filter(|n| (MIN_MINUTES..=MAX_MINUTES).contains(n))
                .map(Self::Minutes)
                .ok_or_else(|| MINUTES_REFUSED.to_owned()),
            ("default" | "fresh", Some(_)) | ("minutes", None) => Err(MINUTES_REFUSED.to_owned()),
            _ => Err(RULE_REFUSED.to_owned()),
        }
    }

    /// The wire's `mode`.
    #[must_use]
    pub fn mode(self) -> &'static str {
        match self {
            Self::Follow => "default",
            Self::Fresh => "fresh",
            Self::Minutes(_) => "minutes",
        }
    }

    /// The wire's `minutes`, only for [`Self::Minutes`].
    #[must_use]
    pub fn minutes(self) -> Option<u32> {
        match self {
            Self::Minutes(n) => Some(n),
            _ => None,
        }
    }

    /// `{"mode": ..., "minutes"?: ...}`.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Self::Minutes(n) => json!({ "mode": "minutes", "minutes": n }),
            other => json!({ "mode": other.mode() }),
        }
    }

    /// The rule written in a stored column: empty is "follow".
    ///
    /// # Errors
    ///
    /// A sentence when the text is not a rule.
    pub fn from_stored(text: &str) -> Result<Self, String> {
        if text.is_empty() {
            return Ok(Self::Follow);
        }
        serde_json::from_str(text).map_err(|_| RULE_REFUSED.to_owned())
    }

    /// The text a column stores: empty for "follow".
    #[must_use]
    pub fn to_stored(self) -> String {
        match self {
            Self::Follow => String::new(),
            other => other.to_json().to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    mode: String,
    minutes: Option<i64>,
}

impl<'de> Deserialize<'de> for CacheRule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = Wire::deserialize(d)?;
        Self::parse(&wire.mode, wire.minutes).map_err(serde::de::Error::custom)
    }
}

impl Serialize for CacheRule {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_rules_round_trip_through_json_and_the_stored_text() {
        for rule in [CacheRule::Follow, CacheRule::Fresh, CacheRule::Minutes(30)] {
            let json = serde_json::to_string(&rule).unwrap_or_default();
            assert_eq!(
                serde_json::from_str::<CacheRule>(&json).ok(),
                Some(rule),
                "{json}"
            );
            assert_eq!(CacheRule::from_stored(&rule.to_stored()), Ok(rule));
        }
        assert_eq!(CacheRule::Follow.to_stored(), "");
    }

    #[test]
    fn minutes_outside_one_to_1440_are_refused_with_a_sentence() {
        for bad in [0, -5, 1441, 100_000] {
            assert_eq!(
                CacheRule::parse("minutes", Some(bad)),
                Err(MINUTES_REFUSED.to_owned()),
                "{bad}"
            );
        }
        assert_eq!(
            CacheRule::parse("minutes", Some(1)),
            Ok(CacheRule::Minutes(1))
        );
        assert_eq!(
            CacheRule::parse("minutes", Some(1440)),
            Ok(CacheRule::Minutes(1440))
        );
        assert_eq!(
            CacheRule::parse("minutes", None),
            Err(MINUTES_REFUSED.to_owned())
        );
        assert_eq!(
            CacheRule::parse("fresh", Some(5)),
            Err(MINUTES_REFUSED.to_owned())
        );
        assert_eq!(
            CacheRule::parse("sometimes", None),
            Err(RULE_REFUSED.to_owned())
        );
    }

    #[test]
    fn a_stored_text_that_is_not_a_rule_is_refused_not_read_as_follow() {
        assert!(CacheRule::from_stored("{\"mode\":\"minutes\"}").is_err());
        assert!(CacheRule::from_stored("nonsense").is_err());
    }
}
