//! Where a release's assets live (REL-2 S4-T1).
//!
//! One place owns the release's URLs. The install fetches the
//! manifest, the Compose file and every file the release ships from
//! here, and the name of an asset is built here too — so the install
//! and the file fetcher cannot disagree about where a release lives
//! (rule 4: one rule, one place).

/// Where a release's assets live.
pub const RELEASES_URL: &str = "https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download";

/// Where the newest release is named.
pub const LATEST_URL: &str =
    "https://api.github.com/repos/RantAI-dev/RantAI-Lakehouse/releases/latest";

/// The URL of one asset of a release.
///
/// The asset's name is the file's own name: the release publishes one
/// asset per file it ships.
#[must_use]
pub fn asset_url(version: &str, name: &str) -> String {
    format!("{RELEASES_URL}/v{version}/{name}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_asset_of_a_release_is_under_the_release_tag() {
        assert_eq!(
            asset_url("0.2.0", "docker-compose.yml"),
            "https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download/v0.2.0/docker-compose.yml"
        );
    }
}
