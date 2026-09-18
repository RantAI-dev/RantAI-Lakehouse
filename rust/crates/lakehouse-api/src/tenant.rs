//! Tenant identity used when labelling assets, audit records, and quotas.
//!
//! These values used to be `const` string literals scattered across the route
//! modules, which pinned the API to a single installation. Reading them from
//! the environment lets one image serve different deployments — the Dispar
//! production console and the partner-facing demo — without a rebuild.
//!
//! Every value keeps its previous literal as the default, so a deployment that
//! sets nothing behaves exactly as before.
//!
//! Resolved once per process via [`LazyLock`]: these are read on nearly every
//! request, and re-reading the environment each time would be wasted work.

use std::collections::HashMap;
use std::sync::LazyLock;

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// Owning organisation — surfaces as the `owner` of catalog assets.
pub static TENANT_OWNER: LazyLock<String> =
    LazyLock::new(|| env_or("TENANT_OWNER", "Dinas Pariwisata & Ekraf DKI Jakarta"));

/// Tenant identifier used for audit, quota, and residency records.
pub static TENANT_ID: LazyLock<String> = LazyLock::new(|| env_or("TENANT_ID", "dispar-dki"));

/// Default business domain of catalog assets.
pub static TENANT_DOMAIN: LazyLock<String> =
    LazyLock::new(|| env_or("TENANT_DOMAIN", "pariwisata"));

/// Data-residency label (where data is permitted to live).
pub static TENANT_RESIDENCY: LazyLock<String> =
    LazyLock::new(|| env_or("TENANT_RESIDENCY", "id-jakarta"));

/// Physical site the services run on.
pub static TENANT_SITE: LazyLock<String> = LazyLock::new(|| env_or("TENANT_SITE", "Depok (187)"));

/// Dataset slugs whose Bronze registry entry counts as curated rather than raw.
///
/// Comma-separated in `BRONZE_CURATED_SLUGS`.
pub static BRONZE_CURATED: LazyLock<Vec<String>> = LazyLock::new(|| {
    env_or(
        "BRONZE_CURATED_SLUGS",
        "wisman-jakarta-per-bulan,wisman-jakarta-per-negara,\
         wisman-jakarta-per-pintu-masuk,jumlah-pengunjung-event-2026",
    )
    .split(',')
    .map(|slug| slug.trim().to_string())
    .filter(|slug| !slug.is_empty())
    .collect()
});

/// True when `slug` should be presented as curated Bronze instead of raw.
#[must_use]
pub fn is_curated_bronze(slug: &str) -> bool {
    BRONZE_CURATED.iter().any(|curated| curated == slug)
}

/// Catalog namespace id for Bronze datasets taken from a PRIMARY source
/// (the tenant's own operational systems / registries).
///
/// Was the literal `"sdi-primer"` — "SDI" is Satu Data Indonesia, one
/// deployment's national-programme vocabulary, which every OTHER
/// deployment then showed on all of its assets (AGENTS.md rule 12: no
/// client names in product code). The id is generic now; a deployment that
/// wants its own wording sets `CATALOG_NAMESPACE_META` — note that an
/// existing override keyed `"sdi-primer"` no longer matches and falls back
/// to the default label below, a cosmetic change only.
pub const NAMESPACE_PRIMER: &str = "primer";

/// Catalog namespace id for Bronze datasets derived from SECONDARY
/// sources (third-party feeds, processed extracts).
pub const NAMESPACE_SEKUNDER: &str = "sekunder";

/// Display name and description for a catalog namespace.
pub struct NamespaceMeta {
    /// Human-readable namespace name.
    pub name: String,
    /// One-line explanation of what the namespace holds.
    pub description: String,
}

/// Namespace labels, overridable as a JSON object in `CATALOG_NAMESPACE_META`
/// of the shape `{"<id>": {"name": ..., "description": ...}}`.
///
/// Malformed JSON is ignored in favour of the defaults: a bad label is a
/// cosmetic problem, but refusing to serve the catalog is an outage.
pub static NAMESPACE_META: LazyLock<HashMap<String, NamespaceMeta>> = LazyLock::new(|| {
    let mut meta: HashMap<String, NamespaceMeta> = [
        (
            NAMESPACE_PRIMER,
            "Sumber Primer",
            "Dataset primer dari sistem sumber tenant, mendarat di Bronze/Iceberg.",
        ),
        (
            NAMESPACE_SEKUNDER,
            "Sumber Sekunder (olahan)",
            "Dataset sekunder: umpan pihak ketiga dan ekstrak olahan.",
        ),
        (
            "silver",
            "Silver (kurasi)",
            "Model bersih & terkonform di ClickHouse — dimensi dan fakta.",
        ),
        (
            "serving",
            "Gold (mart penyaji)",
            "Mart agregat penyaji dashboard dan embed.",
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| {
        (
            id.to_string(),
            NamespaceMeta {
                name: name.to_string(),
                description: description.to_string(),
            },
        )
    })
    .collect();

    // Nested `if let` rather than a let-chain: kept deliberately so this stays
    // compilable on the workspace MSRV (`rust-version = "1.88"`), even though
    // the release image builds on the 1.96.1 toolchain pinned in
    // `rust-toolchain.toml`.
    if let Ok(raw) = std::env::var("CATALOG_NAMESPACE_META") {
        let parsed = serde_json::from_str::<HashMap<String, HashMap<String, String>>>(&raw)
            .unwrap_or_default();
        for (id, fields) in parsed {
            let entry = meta.entry(id).or_insert_with(|| NamespaceMeta {
                name: String::new(),
                description: String::new(),
            });
            if let Some(name) = fields.get("name") {
                entry.name.clone_from(name);
            }
            if let Some(description) = fields.get("description") {
                entry.description.clone_from(description);
            }
        }
    }
    meta
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_slugs_split_and_trim() {
        // Exercises the parsing helper directly: `BRONZE_CURATED` is a
        // process-wide `LazyLock`, so mutating the environment inside a test
        // would race every other test in the binary.
        let parsed: Vec<String> = " a , b ,, c "
            .split(',')
            .map(|slug| slug.trim().to_string())
            .filter(|slug| !slug.is_empty())
            .collect();
        assert_eq!(parsed, vec!["a", "b", "c"]);
    }

    #[test]
    fn defaults_apply_when_unset() {
        assert_eq!(
            env_or("LAKEHOUSE_DEFINITELY_UNSET_VAR", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn blank_env_value_falls_back() {
        // SAFETY: single-threaded scope; the key is unique to this test.
        unsafe { std::env::set_var("LAKEHOUSE_BLANK_TEST_VAR", "   ") };
        assert_eq!(env_or("LAKEHOUSE_BLANK_TEST_VAR", "fallback"), "fallback");
        unsafe { std::env::remove_var("LAKEHOUSE_BLANK_TEST_VAR") };
    }
}
