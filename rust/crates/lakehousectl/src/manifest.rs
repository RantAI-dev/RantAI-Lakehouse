//! The release manifest, as the tool reads it (REL-2 S3-T3).
//!
//! The manifest is the file the release key signs, so everything the
//! installer trusts comes from here: the version, every image's digest,
//! every image's ID, and the checksum of every file the release ships.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The release manifest, `release.json`.
///
/// The model carries what the INSTALL and the UPDATE need: the
/// version, every image's digest, every image's ID, the checksum of
/// every file the release ships, and the upgrade path — the lowest
/// version that may update to this one directly (`minFrom`) and the
/// versions an update to this one must pass through (`stops`). The
/// fields this model does not name are ignored when the manifest is
/// read.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// The release's version.
    pub version: String,
    /// The lowest version that may update to this one directly.
    ///
    /// The release policy's promise is a direct update from the
    /// previous two minor releases, so the release tooling writes the
    /// default and a release may lower it (REL-2 decision D10).
    #[serde(default)]
    pub min_from: Option<String>,
    /// The versions an update to this one must pass through.
    ///
    /// Empty for a release that may be updated to from any release its
    /// `minFrom` allows (REL-2 decision D10).
    #[serde(default)]
    pub stops: Vec<String>,
    /// Every image the release runs, service by service.
    pub images: BTreeMap<String, String>,
    /// The ID of every image, by reference.
    #[serde(default)]
    pub image_ids: BTreeMap<String, String>,
    /// The checksum of every file the release ships.
    pub files: BTreeMap<String, String>,
}

impl Manifest {
    /// The unique image references the release runs.
    ///
    /// Several services may run the same image, and an image is pulled
    /// once.
    #[must_use]
    pub fn references(&self) -> Vec<String> {
        let mut references: Vec<String> = self.images.values().cloned().collect();
        references.sort();
        references.dedup();
        references
    }

    /// The registry digest of an image reference.
    ///
    /// A reference is `name:tag@sha256:…`, and the digest is what
    /// follows the `@`.
    #[must_use]
    pub fn digest_of(reference: &str) -> Option<&str> {
        reference.split_once('@').map(|(_, digest)| digest)
    }
}

/// The name one image's files carry in the release bundle.
///
/// The rule is the one `ops/release/image_name.sh` owns: the
/// repository name, without the tag and without the digest.
///
///     ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:… → rantai-lakehouse-api
///     postgres:16.15@sha256:…                                    → postgres
#[must_use]
pub fn image_name(reference: &str) -> String {
    let without_digest = reference.split('@').next().unwrap_or(reference);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split(':').next().unwrap_or(last).to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_name_of_an_image_is_its_repository_without_the_tag() {
        assert_eq!(
            image_name("ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:abc"),
            "rantai-lakehouse-api"
        );
        assert_eq!(image_name("postgres:16.15@sha256:abc"), "postgres");
        assert_eq!(
            image_name("quay.io/lakekeeper/catalog:v0.13.3@sha256:abc"),
            "catalog"
        );
        assert_eq!(image_name("alpine:3.20"), "alpine");
    }

    #[test]
    fn the_registry_digest_of_a_reference_is_what_follows_the_at_sign() {
        assert_eq!(
            Manifest::digest_of("postgres:16.15@sha256:abc"),
            Some("sha256:abc")
        );
        assert_eq!(Manifest::digest_of("postgres:16.15"), None);
    }

    #[test]
    fn a_manifest_names_every_image_once() {
        let manifest = Manifest {
            version: "0.2.0".to_string(),
            min_from: None,
            stops: Vec::new(),
            images: BTreeMap::from([
                ("a".to_string(), "one@sha256:1".to_string()),
                ("b".to_string(), "one@sha256:1".to_string()),
                ("c".to_string(), "two@sha256:2".to_string()),
            ]),
            image_ids: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        assert_eq!(
            manifest.references(),
            vec!["one@sha256:1".to_string(), "two@sha256:2".to_string()]
        );
    }
}
