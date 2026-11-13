//! The permission catalog's drift test and the console's JSON fixture
//! (`WEB-8` D5, D6).
//!
//! # The drift test
//!
//! `lakehouse_auth::permission_catalog::PERMISSIONS` is the one list the
//! console's pickers offer, so it must contain every permission string
//! the API actually checks. A check added anywhere without a catalog
//! entry would leave the console unable to grant it — this file fails
//! the moment that happens, from four surfaces:
//!
//! 1. every `Policy::RequiresPermission` in [`POLICY_TABLE`];
//! 2. every non-empty tool permission in `routes::ai::registry::TOOLS`
//!    (read through `routes::ai_tool_permissions`, because `registry` is
//!    not visible to integration tests);
//! 3. every string literal passed to `.has("…")` under this crate's
//!    `src/`, read from disk at test time;
//! 4. the three named permission constants — [`SAMPLE_PERMISSION`],
//!    [`ENTRY_PERMISSION`] and [`POLICY_READ`] — whose values are pinned
//!    to their catalog entries here, so renaming one without updating the
//!    catalog fails too.
//!
//! The `.has()` scan is deliberately text-level (the plan's
//! `\.has\("([^"]+)"\)` regex, matched by hand to avoid a regex
//! dependency): a `.has("query:read")` in a doc comment or a `#[cfg(test)]`
//! module still names a permission somebody wrote down, and listing it
//! costs nothing.
//!
//! # The fixture
//!
//! `tests/fixtures/permissions.json` is [`PERMISSIONS`] as JSON, the list
//! the console's `src/features/copilot`-style tests read
//! (`capabilities.test.ts` reads `tool_schemas.json` the same way;
//! `ADM-13` does the same for this fixture). The equality test pins it
//! byte-for-byte; the ignored writer regenerates it, only for a reviewed
//! catalog change.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;

use lakehouse_api::policy::{POLICY_TABLE, Policy};
use lakehouse_api::routes::ai_tool_permissions;
use lakehouse_auth::permission_catalog::PERMISSIONS;

/// `WEB-8` D5: every permission string the API checks is in the catalog.
/// Each numbered surface is listed in the module doc comment; the failure
/// names the missing strings so the fix is to add catalog entries, never
/// to weaken this test.
#[test]
fn every_permission_the_api_checks_is_in_the_catalog() {
    // 4's pinned constants: the named permission constants —
    // `catalog.rs`'s `SAMPLE_PERMISSION`, `dashboard_access.rs`'s
    // `ENTRY_PERMISSION` and `catalog_governance.rs`'s `POLICY_READ` —
    // are private consts, so their VALUES are pinned here against their
    // catalog entries, and the source scan below fails if a constant's
    // value moves without this list being revisited.
    const PINNED_CONSTANTS: [(&str, &str); 3] = [
        ("SAMPLE_PERMISSION", "query:read"),
        ("ENTRY_PERMISSION", "dashboard:read"),
        ("POLICY_READ", "policy:read"),
    ];

    let listed: BTreeSet<&str> = PERMISSIONS.iter().map(|p| p.name).collect();
    let mut missing: Vec<&str> = Vec::new();

    // 1. The route policy table.
    for (_method, _path, policy) in POLICY_TABLE {
        match policy {
            Policy::RequiresPermission(permission) => {
                if !listed.contains(permission) {
                    missing.push(permission);
                }
            }
            Policy::RequiresAnyPermission(permissions) => {
                for permission in *permissions {
                    if !listed.contains(permission) {
                        missing.push(permission);
                    }
                }
            }
            Policy::Public | Policy::RequiresAuth => {}
        }
    }

    // 2. The AI tool table's non-empty permissions.
    for permission in ai_tool_permissions() {
        if !listed.contains(permission) {
            missing.push(permission);
        }
    }

    // 3. Every `.has("…")` literal under src/, read from disk so the scan
    //    cannot go stale.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = String::new();
    collect_rust_sources(&src, &mut sources);
    for literal in has_literals(&sources) {
        if !listed.contains(literal) {
            missing.push(literal);
        }
    }

    for (name, value) in PINNED_CONSTANTS {
        let found = const_string_value(&sources, name)
            .unwrap_or_else(|| panic!("const {name} must exist in src/ for the drift test"));
        assert_eq!(
            found, value,
            "const {name} moved to {found}; update the catalog drift test's pinned value"
        );
        if !listed.contains(value) {
            missing.push(value);
        }
    }

    assert!(
        missing.is_empty(),
        "permissions the API checks but the catalog does not list: {missing:?}"
    );
}

/// `WEB-8` D6: the committed fixture is the catalog, byte for byte.
#[test]
fn the_committed_fixture_equals_the_catalog() {
    let actual = serde_json::to_string_pretty(&PERMISSIONS).unwrap();
    let committed = include_str!("fixtures/permissions.json");
    assert_eq!(
        actual, committed,
        "tests/fixtures/permissions.json drifted from PERMISSIONS — regenerate it ONLY for \
         a reviewed catalog change, with: cargo test -p lakehouse-api --test it \
         permission_catalog::write_permission_catalog_fixture -- --ignored"
    );
}

/// One-off fixture writer, run by hand only after an intentional catalog
/// change: `cargo test -p lakehouse-api --test it
/// permission_catalog::write_permission_catalog_fixture -- --ignored`
/// (`OPS-32`: the file is a module of the `it` binary now).
#[test]
#[ignore = "writes tests/fixtures/permissions.json; run only for a reviewed catalog change"]
fn write_permission_catalog_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/permissions.json"
    );
    let body = serde_json::to_string_pretty(&PERMISSIONS).unwrap();
    std::fs::write(path, body).unwrap();
}

// ── the source scanners ──────────────────────────────────────────────────

/// Append every `.rs` file under `dir` into `out`. Depth-first, no
/// ordering promise — the drift test only needs every file's text.
fn collect_rust_sources(dir: &Path, out: &mut String) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            out.push_str(&text);
            out.push('\n');
        }
    }
}

/// Every string literal passed to `.has("…")` — the plan's
/// `\.has\("([^"]+)"\)` regex, matched by hand so the test needs no regex
/// dependency. A `.has("")` (empty group) is not a match, like the regex.
fn has_literals(sources: &str) -> Vec<&str> {
    const NEEDLE: &str = ".has(\"";
    let mut found = Vec::new();
    let mut rest = sources;
    while let Some(start) = rest.find(NEEDLE) {
        let after = &rest[start + NEEDLE.len()..];
        match after.find('"') {
            // Non-empty literal: a match; keep scanning after it.
            Some(end) if end > 0 => {
                found.push(&after[..end]);
                rest = &after[end + 1..];
            }
            // Empty literal, or no closing quote left: skip this hit.
            Some(end) => rest = &after[end + 1..],
            None => break,
        }
    }
    found
}

/// The string value of `const <name>: &str = "<value>";` in `sources`.
/// The three permission constants are declared exactly once each, so the
/// first hit is the declaration.
fn const_string_value(sources: &str, name: &str) -> Option<String> {
    let needle = format!("const {name}: &str = \"");
    let start = sources.find(&needle)? + needle.len();
    let after = &sources[start..];
    let end = after.find('"')?;
    Some(after[..end].to_owned())
}
