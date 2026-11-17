//! The versions of a release, and how two of them compare (REL-2 S4 decision D10).
//!
//! # Why this module exists
//!
//! The upgrade path is a comparison of two versions: the one that is
//! installed (`state.json`'s `currentVersion`) and the one the update
//! moves to. The rules of that comparison are D17's:
//!
//! - the SAME version: nothing to do;
//! - an OLDER release: refused, pointing at `lakehousectl rollback`;
//! - a release below the target's `minFrom`, or one with a `stops`
//!   entry between: the update goes through the stops.
//!
//! So the comparison has to be exact, and it has to be ONE rule: the
//! install, the update and the status all read a version through here.
//!
//! # The rule
//!
//! A version is `major.minor.patch` with an OPTIONAL suffix
//! (`0.2.0-update-proof.1`). Two versions are compared by their
//! numbers, and a version that carries a suffix is a LATER BUILD of
//! the release its numbers name, so it sorts after the bare one.
//!
//! The suffix is NOT `SemVer`'s prerelease. `SemVer` says
//! `0.2.0-rc.1` is OLDER than `0.2.0`, and D17 refuses an older
//! release — so under `SemVer`'s rule the proof's second bundle could
//! not be installed over the first. D21 builds the proof's bundles as
//! later builds of one release (`0.2.0`, `0.2.0-update-proof.1`,
//! `0.2.0-update-proof.2`), and the rule here is the one that lets the
//! tool move from one to the next. The release policy's own `SemVer`
//! rule applies to the releases themselves
//! (`docs/core/reference/release-policy.md`); this rule is the one the
//! tool uses to order the two releases it is asked to move between.

use std::cmp::Ordering;

/// A version, as the tool reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// The major number.
    pub major: u64,
    /// The minor number.
    pub minor: u64,
    /// The patch number.
    pub patch: u64,
    /// The suffix, without its `-`.
    ///
    /// `None` for a bare release (`0.2.0`).
    pub suffix: Option<String>,
}

impl Version {
    /// The version, as it is written.
    #[must_use]
    pub fn as_string(&self) -> String {
        match &self.suffix {
            Some(suffix) => format!("{}.{}.{}-{suffix}", self.major, self.minor, self.patch),
            None => format!("{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}

/// Parse a version.
///
/// A version is `major.minor.patch`, with an optional `-suffix`. The
/// numbers may not carry a leading `+`: that is build metadata, and
/// two releases that differ only in it are the same release.
///
/// # Errors
///
/// Returns the message that says what the version should look like.
pub fn parse(version: &str) -> Result<Version, String> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let (numbers, suffix) = match version.split_once('-') {
        Some((numbers, suffix)) => (numbers, Some(suffix)),
        None => (version, None),
    };
    let mut parts = numbers.split('.');
    let mut number = |name: &str| -> Result<u64, String> {
        let text = parts.next().unwrap_or_default();
        text.parse::<u64>().map_err(|_| {
            format!("the version {version:?} is not `major.minor.patch`: its {name} is {text:?}.")
        })
    };
    let major = number("major")?;
    let minor = number("minor")?;
    let patch = number("patch")?;
    if parts.next().is_some() {
        return Err(format!(
            "the version {version:?} is not `major.minor.patch`: it holds more than three numbers."
        ));
    }
    if let Some(suffix) = &suffix
        && suffix.is_empty()
    {
        return Err(format!(
            "the version {version:?} carries an empty suffix. A suffix names a later build of the \
             release, so it says which one."
        ));
    }
    Ok(Version {
        major,
        minor,
        patch,
        suffix: suffix.map(str::to_string),
    })
}

/// Compare two versions.
///
/// The numbers decide, and a version that carries a suffix sorts after
/// the bare version with the same numbers (see the module's own docs).
///
/// # Errors
///
/// Returns the message that says what the version should look like.
pub fn compare(left: &str, right: &str) -> Result<Ordering, String> {
    let left = parse(left)?;
    let right = parse(right)?;
    Ok(order(&left, &right))
}

/// The order of two parsed versions.
///
/// The numbers decide, and a version that carries a suffix sorts after
/// the bare version with the same numbers (see the module's own docs).
#[must_use]
pub fn order(left: &Version, right: &Version) -> Ordering {
    left.major
        .cmp(&right.major)
        .then(left.minor.cmp(&right.minor))
        .then(left.patch.cmp(&right.patch))
        .then_with(|| suffix_order(left.suffix.as_deref(), right.suffix.as_deref()))
}

/// The order of two suffixes.
///
/// A version with no suffix is the release itself, and a version with a
/// suffix is a later build of it: the bare one comes first. Two
/// suffixes are compared with their numbers read as numbers, so
/// `-update-proof.10` is newer than `-update-proof.9`.
fn suffix_order(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => compare_suffix(left, right),
    }
}

/// Compare two suffixes, number by number.
fn compare_suffix(left: &str, right: &str) -> Ordering {
    let mut left = runs(left).into_iter();
    let mut right = runs(right).into_iter();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(one), Some(other)) => {
                let order = match (one.parse::<u64>(), other.parse::<u64>()) {
                    (Ok(one), Ok(other)) => one.cmp(&other),
                    _ => one.cmp(other),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The runs of a suffix: a run of digits, or a run of anything else.
///
/// The runs are cut on byte boundaries, and a suffix is ASCII in every
/// release this tool reads.
fn runs(suffix: &str) -> Vec<&str> {
    let bytes = suffix.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let is_digit = bytes[start].is_ascii_digit();
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() == is_digit {
            end += 1;
        }
        parts.push(&suffix[start..end]);
        start = end;
    }
    parts
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_bare_version_is_read_as_its_three_numbers() {
        let version = parse("0.2.0").expect("the version is read");
        assert_eq!(version.major, 0);
        assert_eq!(version.minor, 2);
        assert_eq!(version.patch, 0);
        assert_eq!(version.suffix, None);
        assert_eq!(version.as_string(), "0.2.0");
    }

    #[test]
    fn a_version_with_a_suffix_is_read_as_a_later_build() {
        let version = parse("0.2.0-update-proof.1").expect("the version is read");
        assert_eq!(version.suffix.as_deref(), Some("update-proof.1"));
        assert_eq!(version.as_string(), "0.2.0-update-proof.1");
    }

    #[test]
    fn a_version_may_carry_a_v_prefix() {
        assert_eq!(
            parse("v0.2.0").expect("the version is read"),
            parse("0.2.0").expect("the version is read")
        );
    }

    #[test]
    fn a_version_that_is_not_three_numbers_is_refused() {
        for version in ["0.2", "0.2.0.1", "zero.two.zero", "0.2.x", ""] {
            assert!(parse(version).is_err(), "{version:?} was not refused");
        }
    }

    #[test]
    fn a_version_with_an_empty_suffix_is_refused() {
        assert!(parse("0.2.0-").is_err());
    }

    #[test]
    fn the_numbers_decide_the_order() {
        assert_eq!(
            compare("0.3.0", "0.2.9").expect("the versions are compared"),
            Ordering::Greater
        );
        assert_eq!(
            compare("1.0.0", "0.9.9").expect("the versions are compared"),
            Ordering::Greater
        );
        assert_eq!(
            compare("0.2.1", "0.2.0").expect("the versions are compared"),
            Ordering::Greater
        );
    }

    #[test]
    fn a_later_build_is_newer_than_the_release_it_is_a_build_of() {
        // D21: the proof's bundles are later builds of one release, and
        // D17 refuses an older release — so the rule has to put them
        // in this order.
        assert_eq!(
            compare("0.2.0-update-proof.1", "0.2.0").expect("the versions are compared"),
            Ordering::Greater
        );
        assert_eq!(
            compare("0.2.0-update-proof.2", "0.2.0-update-proof.1")
                .expect("the versions are compared"),
            Ordering::Greater
        );
    }

    #[test]
    fn the_numbers_of_a_suffix_are_read_as_numbers() {
        assert_eq!(
            compare("0.2.0-update-proof.10", "0.2.0-update-proof.9")
                .expect("the versions are compared"),
            Ordering::Greater
        );
    }

    #[test]
    fn the_same_version_is_the_same_version() {
        assert_eq!(
            compare("0.2.0", "0.2.0").expect("the versions are compared"),
            Ordering::Equal
        );
        assert_eq!(
            compare("0.2.0-update-proof.1", "0.2.0-update-proof.1")
                .expect("the versions are compared"),
            Ordering::Equal
        );
    }

    #[test]
    fn a_suffix_is_compared_with_the_release_it_names() {
        // A suffix that is not a number is compared as text, so the
        // order is still a total one.
        assert_eq!(
            compare("0.2.0-alpha", "0.2.0-beta").expect("the versions are compared"),
            Ordering::Less
        );
    }
}
