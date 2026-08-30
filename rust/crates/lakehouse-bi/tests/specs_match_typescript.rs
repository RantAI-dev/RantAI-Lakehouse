//! Drift guard: `lakehouse_bi::specs::{KPIS, CHARTS}` vs. their `TypeScript`
//! source of truth, `src/lib/dashboard-specs.ts`.
//!
//! The static chart/KPI catalog now exists in two languages — a hand port,
//! not a codegen — and nothing stops the two from drifting the next time
//! either side gets a new chart or an SQL tweak. This test shells out to
//! `bun` to import the TS module directly (it has no imports of its own, so
//! no bundler/Next.js context is needed) and compares `id`/`sql` for every
//! entry, in order.
//!
//! At the time this test was written, a manual review confirmed all 13
//! entries (9 charts + 4 KPIs) are byte-identical, so this test is expected
//! to PASS on a clean checkout. If it fails, the Rust side drifted — fix
//! `lakehouse-bi::specs`, not this test (and not the TS, which is the
//! source of truth here).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

use lakehouse_bi::specs::{CHARTS, KPIS};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct TsSpec {
    id: String,
    sql: String,
}

#[derive(Debug, Deserialize)]
struct TsSpecs {
    kpis: Vec<TsSpec>,
    charts: Vec<TsSpec>,
}

/// `true` if a `bun` binary is on `PATH` and runs. Checked with `bun
/// --version` rather than assumed, so CI environments without `bun`
/// installed skip cleanly instead of failing on a missing binary.
fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn rust_specs_match_typescript_source_of_truth() {
    if !bun_available() {
        eprintln!(
            "SKIP rust_specs_match_typescript_source_of_truth: `bun` not found on PATH. \
             This test cross-checks lakehouse_bi::specs against src/lib/dashboard-specs.ts by \
             shelling out to `bun`; install bun (https://bun.sh) to run it locally or in CI."
        );
        return;
    }

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/extract_specs.ts");
    assert!(script.is_file(), "missing {}", script.display());

    let output = Command::new("bun")
        .arg("run")
        .arg(&script)
        .output()
        .unwrap_or_else(|err| panic!("running `bun run {}`: {err}", script.display()));
    assert!(
        output.status.success(),
        "`bun run {}` failed (status {}):\nstdout: {}\nstderr: {}",
        script.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let ts: TsSpecs = serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parsing `bun run {}` stdout as JSON: {err}\nstdout: {}",
            script.display(),
            String::from_utf8_lossy(&output.stdout)
        )
    });

    let rust_kpis: Vec<(&str, &str)> = KPIS.iter().map(|k| (k.id, k.sql)).collect();
    let ts_kpis: Vec<(&str, &str)> = ts
        .kpis
        .iter()
        .map(|k| (k.id.as_str(), k.sql.as_str()))
        .collect();
    assert_eq!(
        rust_kpis, ts_kpis,
        "lakehouse_bi::specs::KPIS drifted from src/lib/dashboard-specs.ts's KPIS \
         (id/sql, in order) — fix the Rust side, not this test"
    );

    let rust_charts: Vec<(&str, &str)> = CHARTS.iter().map(|c| (c.id, c.sql)).collect();
    let ts_charts: Vec<(&str, &str)> = ts
        .charts
        .iter()
        .map(|c| (c.id.as_str(), c.sql.as_str()))
        .collect();
    assert_eq!(
        rust_charts, ts_charts,
        "lakehouse_bi::specs::CHARTS drifted from src/lib/dashboard-specs.ts's CHARTS \
         (id/sql, in order) — fix the Rust side, not this test"
    );
}
