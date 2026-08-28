//! Golden-corpus parity harness — the cutover gate.
//!
//! Replays every entry in `rust/tests/parity/corpus/*.json` (73 real
//! responses captured from the live `TypeScript` backend) against a running
//! Rust service and asserts structural equality modulo fields that are
//! genuinely non-deterministic (timestamps, generated ids, live cluster
//! aggregates, LLM output — see `rust/tests/parity/README.md`).
//!
//! This is the ONLY safety net for the big-bang cutover: there is no
//! per-endpoint flag to roll back individually, so every failure here is a
//! potential production incident. Do not weaken the normalizer to make a
//! real defect disappear — widen it only for fields the README documents as
//! genuinely volatile, and otherwise fix the Rust or flag the corpus entry.
//!
//! # Running
//!
//! This test needs a live Rust service — it deliberately does NOT spawn one,
//! so failures point at a real, inspectable process. Start one first:
//!
//! ```bash
//! set -a; . ../.env.local; set +a
//! cargo run -p lakehouse-api &
//! PARITY_TARGET=http://localhost:8080 \
//!   cargo test -p lakehouse-api --test parity -- --ignored --nocapture
//! ```
//!
//! `PARITY_TARGET` defaults to `http://localhost:8080`. Set `PUBLIC_DASH_TOKEN`
//! to the same value used when the corpus was captured (or any currently
//! valid public dashboard token) to exercise `public-dash-ok`; without it,
//! that single entry is skipped with a clear message.
//!
//! # Task 3.2: replaying against an authenticated service
//!
//! The corpus was captured from an unauthenticated TypeScript backend, but
//! this Rust service now requires authentication on every route except the
//! four `Policy::Public` entries in `lakehouse_api::policy::POLICY_TABLE`
//! (not reachable from here — this is a separate binary crate's
//! integration test — so this file keeps its own short mirror of just the
//! fact that matters: a valid principal is needed). This harness solves
//! that the same way any other authenticated API caller would: it sends
//! `Authorization: Bearer <token>` for a real
//! [`lakehouse_auth::ServiceTokenAuthenticator`]-backed service credential
//! on every request (see [`resolve_service_token`]) — it does NOT bypass
//! auth, use a test-only header, or rely on any `AUTH_DISABLED`-style
//! escape hatch (there is no such flag in this codebase). The service
//! identity's scopes are `["*:*"]`: this harness's job is to prove
//! response-shape parity across ~70 corpus entries spanning almost every
//! permission in `POLICY_TABLE`, not to test authorization decisions
//! themselves (that's `routes::route_policy_tests` and the task's curl
//! transcripts), so a narrower scope set would just be extra bookkeeping
//! with no parity-signal benefit — this is a fixture, not a production
//! credential, and it is never issued outside test setup.
//!
//! Set `PARITY_SERVICE_TOKEN` to reuse an already-issued token. If unset,
//! [`resolve_service_token`] idempotently provisions its own
//! `service_identity`/`service_credential` fixture row (named
//! `"parity-harness"`) directly against `DATABASE_URL`, the same database
//! the target server itself uses — mirroring how `main::bootstrap_admin`
//! seeds its own fixture identity. If neither a token nor a reachable
//! database is available, every non-public corpus entry is skipped with a
//! clear per-entry message rather than failing opaquely.
//!
//! The corpus contains only validation/error paths for every mutating
//! handler (`alerts`, `dashboard/boards`, `dashboard/specs`, `ai/sessions`,
//! `pipelines/{id}/trigger`) plus read-only queries, so a full replay against
//! a live backend has no destructive side effects. `_OMITTED_alerts-run` is
//! not in the corpus at all (see README) and is never replayed.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// One `rust/tests/parity/corpus/*.json` file.
#[derive(Debug, Deserialize)]
struct CorpusEntry {
    request: RequestSpec,
    status: u16,
    #[serde(rename = "contentType")]
    content_type: String,
    body: Value,
}

#[derive(Debug, Deserialize)]
struct RequestSpec {
    name: String,
    method: Option<String>,
    path: Option<String>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default, rename = "expectStatus")]
    expect_status: Option<u16>,
    #[serde(default)]
    skip: bool,
}

/// LLM-backed captures whose output is genuinely non-deterministic model
/// text/tool-call content. Compared by top-level key presence + JSON type
/// only — never by value or array length/content, since a different model
/// run can legitimately call different tools or return a different number
/// of results.
const STRUCTURE_ONLY: &[&str] = &[
    "ai-chat-ok",
    "agent-ask-ok",
    "agent-query-ok",
    "agent-text-to-sql-ok",
];

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/parity/corpus")
}

/// Per-entry additional keys to normalize (at any depth), beyond the global
/// rules. Kept to an explicit per-entry allowlist rather than a global
/// key-name rule because every key added here (`error`, `title`, `content`,
/// `sql`, `secret`, `current`, `computeUnits`, `budgetSpent`) is compared
/// exactly everywhere else it occurs in the corpus — only these specific
/// captures redacted or cannot reproduce that particular value.
fn extra_normalize_keys(name: &str) -> HashSet<&'static str> {
    match name {
        "alerts-create-bad-body" | "dashboard-boards-create-bad-body" => {
            eprintln!(
                "NOTE: normalizing the `error` field for `{name}` — it records Bun's JSON-parser \
                 message (`JSON Parse error: Unexpected identifier \"not\"`), which the Rust \
                 service cannot reproduce byte-for-byte. See rust/tests/parity/README.md."
            );
            HashSet::from(["error"])
        }
        // `capture.ts`'s REDACT_TEXT_IN × REDACT_TEXT_KEYS: real chat-session
        // free text (and, for this specific capture, the SQL a real tool
        // call generated) was redacted at capture time and can never be
        // compared by value.
        "ai-sessions-list" => HashSet::from(["title"]),
        "ai-sessions-detail" => HashSet::from(["title", "content", "sql"]),
        // The embed HMAC signing secret (`REDACT_KEYS_ALWAYS` in
        // capture.ts) — redacted at capture time for the same reason.
        "dashboard-embed-info" => HashSet::from(["secret"]),
        // Live SLO snapshot text (e.g. `"1233ms"`), embedded in a string
        // rather than a bare number so the generic numeric-leaf rule can't
        // catch it. Same "live cluster aggregate" category as
        // `queryP95Ms`/`p95Ms`, see README.
        "ops-observability" => HashSet::from(["current"]),
        // Per-tenant echoes of the live `computeUnits7d` aggregate.
        "ops-usage" => HashSet::from(["computeUnits", "budgetSpent"]),
        _ => HashSet::new(),
    }
}

/// Entries whose named top-level array field has no stable order to assert
/// on. `catalog-list`'s `assets` comes from `SELECT ... FROM
/// bronze_meta.dataset_catalog UNION ALL SELECT ... FROM
/// bronze_meta_sec.dataset_catalog` with no `ORDER BY` — confirmed live
/// (three consecutive `curl`s against the same unmodified Rust process
/// returned two different orderings of the same rows). The `TypeScript`
/// route runs the identical unordered SQL, so this is shared, pre-existing
/// behavior, not a porting regression; sorting both sides before comparing
/// asserts every field of every asset while dropping the one guarantee
/// neither backend's SQL ever made.
fn unordered_array_keys(name: &str) -> &'static [&'static str] {
    match name {
        // `namespaces` is grouped from the same unordered `assets` scan, so
        // it inherits the same lack of ordering guarantee.
        "catalog-list" => &["assets", "namespaces"],
        // `routes::governance::classification`'s ClickHouse-derived rows
        // come from the identical shape of query as `catalog-list`
        // (`SELECT ... FROM bronze_meta.dataset_catalog UNION ALL SELECT
        // ... FROM bronze_meta_sec.dataset_catalog`, no `ORDER BY`) --
        // confirmed live the same way: consecutive requests against an
        // unmodified process returned different orderings. Pre-existing,
        // not introduced by the Task 2.3/2.6 Postgres unions; it only
        // surfaced once this entry's corpus needed re-capturing for those.
        "gov-classification" => &["classifications"],
        _ => &[],
    }
}

/// Per-entry arrays that are APPEND-ONLY LIVE HISTORY: every new `Dagster`
/// run adds elements, and a run that was `failed` at capture time can later be
/// `completed` after a retry, which also shifts every subsequent index.
///
/// Asserting length or per-index values here was never going to hold — the
/// corpus is a snapshot of an unbounded, continuously-growing dataset, and it
/// decayed within a day of capture (`gov-audit` 26 → 28 entries,
/// `pipeline-runs` 14 → 15).
///
/// These are compared by ELEMENT SHAPE instead: the set of distinct
/// key → JSON-type signatures across the array. A renamed field, a dropped
/// field, or a changed type still fails, which is what this harness exists to
/// catch. Only growth and reordering are tolerated.
///
/// Deliberately NOT `STRUCTURE_ONLY`: that would drop every value assertion on
/// the whole response, including the stable scalars around these arrays.
fn append_only_array_keys(name: &str) -> &'static [&'static str] {
    match name {
        // routes::governance::audit — derived from Dagster run history.
        "gov-audit" => &["audit"],
        // routes::pipelines::runs — literally the Dagster run list.
        "pipeline-runs" => &["runs"],
        _ => &[],
    }
}

/// Replaces `value[key]` (an array of objects) with the sorted set of distinct
/// `field:type` signatures of its elements, so growth and reordering compare
/// equal while a schema change does not.
fn reduce_array_to_element_shapes(value: &mut Value, key: &str) {
    let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    let mut shapes: Vec<String> = items
        .iter()
        .map(|item| match item {
            Value::Object(map) => {
                let mut fields: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{k}:{}", json_kind(v)))
                    .collect();
                fields.sort();
                fields.join(",")
            }
            other => json_kind(other).to_owned(),
        })
        .collect();
    shapes.sort();
    shapes.dedup();
    *value.get_mut(key).unwrap_or(&mut Value::Null) =
        Value::Array(shapes.into_iter().map(Value::String).collect());
}

/// Sorts `value.get(key)` (a JSON array of objects) by each element's `id`
/// field, in place, if present. Used to compare an inherently-unordered
/// array (see [`unordered_array_keys`]) without caring about position.
fn sort_array_by_id(value: &mut Value, key: &str) {
    let Some(Value::Array(items)) = value.get_mut(key) else {
        return;
    };
    items.sort_by(|a, b| {
        let ka = a.get("id").and_then(Value::as_str).unwrap_or_default();
        let kb = b.get("id").and_then(Value::as_str).unwrap_or_default();
        ka.cmp(kb)
    });
}

/// Keys that are volatile everywhere they occur, regardless of endpoint —
/// timestamps, live-cluster aggregates, re-signed tokens. See
/// `rust/tests/parity/README.md` for why each is here.
fn is_globally_volatile_key(key: &str) -> bool {
    matches!(
        key,
        // Clock/id-derived timestamps.
        "createdAt"
            | "updatedAt"
            | "startedAt"
            | "endedAt"
            | "lastRunAt"
            | "at"
            // Per-query/per-run metrics.
            | "durationMs"
            | "elapsedMs"
            | "scannedBytes"
            | "scannedBytes24h"
            | "scannedBytes7d"
            | "costUnits"
            // Live cluster aggregates (system.query_log / system.processes
            // rollups in ops-usage, ops-observability, overview-get,
            // storage-get; ops-workloads is read entirely as structure-only
            // below the JSON-object level via the same numeric rule).
            | "computeUnits7d"
            | "volume24h"
            | "p95Ms"
            | "queryP95Ms"
            // Re-signed / regenerated on every request.
            | "publicToken"
            | "sampleToken"
    )
}

/// `q-<epoch-ms>` (query-run ids) and UUID-v4-shaped strings (Dagster run
/// ids, activity/audit ids) are generated per-request and can never match
/// the corpus. Matched by VALUE SHAPE, not by key name, because plenty of
/// stable ids (`u_2dadfe2e`, `b_73c6fcc1`, `default`, `s1`, dataset slugs)
/// also live under a key literally called `id` and must keep being compared
/// exactly — a shape-based rule is what lets those coexist safely.
fn looks_like_generated_id(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("q-") {
        if rest.len() >= 10 && rest.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, want_len)| p.len() == want_len && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `capture.ts`'s `REDACTED_MARKER` shape (`^<redacted:\d+>$`) — a value the
/// corpus deliberately never persisted, so it can never be compared by
/// value. Anything shaped like this is unmatchable by construction, whatever
/// key it lives under.
fn looks_like_redacted_marker(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("<redacted:") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix('>') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn type_marker(v: &Value) -> Value {
    let s = match v {
        Value::Null => "<null>",
        Value::Bool(_) => "<bool>",
        Value::Number(_) => "<number>",
        Value::String(_) => "<string>",
        Value::Array(_) => "<array>",
        Value::Object(_) => "<object>",
    };
    Value::String(s.to_owned())
}

/// Recursively replace volatile leaves with a type marker (preserving JSON
/// kind, not value), on ONE tree. Applying this independently to the
/// expected and actual trees — rather than walking both in lockstep — is
/// what makes a field that disappears entirely still fail: the expected
/// side gets a marker, the actual side simply lacks the key, and object
/// equality (`serde_json::Map`, order-independent) reports the mismatch.
fn normalize(value: &Value, key: Option<&str>, extra: &HashSet<&str>) -> Value {
    if let Some(k) = key {
        if is_globally_volatile_key(k) || extra.contains(k) {
            return type_marker(value);
        }
        if k == "rows" && matches!(value, Value::Number(_)) {
            // Row *counts* on catalog/dataset entries drift with every
            // ingest; row *arrays* (query results) are a different shape
            // entirely (`Value::Array`) and are intentionally excluded here.
            return type_marker(value);
        }
    }
    match value {
        Value::String(s) => {
            // `runId` on /api/ai/build-status is an exact echo of the query
            // parameter we sent — worth comparing verbatim, not masking.
            if key != Some("runId") && (looks_like_generated_id(s) || looks_like_redacted_marker(s))
            {
                type_marker(value)
            } else {
                value.clone()
            }
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), normalize(v, Some(k), extra));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| normalize(v, key, extra)).collect())
        }
        other => other.clone(),
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Top-level key presence + JSON type only, for LLM-backed entries. See
/// [`STRUCTURE_ONLY`].
fn compare_structure_only(expected: &Value, actual: &Value) -> Result<(), String> {
    let (Value::Object(exp_map), Value::Object(act_map)) = (expected, actual) else {
        return if json_kind(expected) == json_kind(actual) {
            Ok(())
        } else {
            Err(format!(
                "$: expected kind {}, got {}",
                json_kind(expected),
                json_kind(actual)
            ))
        };
    };
    let mut diffs = Vec::new();
    for (k, ev) in exp_map {
        match act_map.get(k) {
            None => diffs.push(format!(
                "$.{k}: missing in actual (expected {})",
                json_kind(ev)
            )),
            Some(av) if json_kind(av) != json_kind(ev) => diffs.push(format!(
                "$.{k}: expected kind {}, got {}",
                json_kind(ev),
                json_kind(av)
            )),
            Some(_) => {}
        }
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(diffs.join("; "))
    }
}

fn collect_diffs(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    if out.len() >= 20 {
        return;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                let child = format!("{path}.{k}");
                match a.get(k) {
                    None => out.push(format!("{child}: missing in actual (expected {ev})")),
                    Some(av) => collect_diffs(&child, ev, av, out),
                }
            }
            for k in a.keys() {
                if !e.contains_key(k) {
                    out.push(format!(
                        "{path}.{k}: unexpected key in actual (got {})",
                        a[k]
                    ));
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                out.push(format!(
                    "{path}: array length mismatch — expected {}, got {}",
                    e.len(),
                    a.len()
                ));
            }
            for (i, (ev, av)) in e.iter().zip(a.iter()).enumerate() {
                collect_diffs(&format!("{path}[{i}]"), ev, av, out);
            }
        }
        _ => {
            if expected != actual {
                out.push(format!("{path}: expected {expected}, got {actual}"));
            }
        }
    }
}

fn diff_json(name: &str, expected: &Value, actual: &Value) -> String {
    let mut diffs = Vec::new();
    collect_diffs("$", expected, actual, &mut diffs);
    if diffs.is_empty() {
        // Structural equality already failed but the field-by-field walk
        // found nothing — fall back to a raw dump rather than pretend
        // everything matched.
        diffs.push(format!(
            "(no field-level diff found; raw)\n  expected: {expected}\n  actual:   {actual}"
        ));
    }
    format!(
        "{name}: body mismatch after normalization\n  {}",
        diffs.join("\n  ")
    )
}

fn diff_text(name: &str, expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let mut diffs = Vec::new();
    for i in 0..exp_lines.len().max(act_lines.len()) {
        let e = exp_lines.get(i).copied().unwrap_or("<missing line>");
        let a = act_lines.get(i).copied().unwrap_or("<missing line>");
        if e != a {
            diffs.push(format!("  line {}: expected {e:?}, got {a:?}", i + 1));
            if diffs.len() >= 20 {
                diffs.push("  ... (truncated)".to_owned());
                break;
            }
        }
    }
    format!("{name}: yaml body mismatch\n{}", diffs.join("\n"))
}

/// The fixed name of the fixture `service_identity` this harness
/// provisions for itself when `PARITY_SERVICE_TOKEN` isn't set. Unique
/// (`service_identity_name_unique`), so a second run finds the same row
/// instead of erroring or creating a duplicate — the same idempotency
/// shape `main::bootstrap_admin` uses for its own fixture.
const PARITY_SERVICE_IDENTITY_NAME: &str = "parity-harness";

/// Resolve an `Authorization: Bearer` token to replay the corpus as an
/// authenticated caller. See the module doc comment ("Task 3.2: replaying
/// against an authenticated service") for the full reasoning.
///
/// Prefers `PARITY_SERVICE_TOKEN` if set (reusing an already-issued
/// token — useful in CI, where provisioning a fresh one on every run is
/// unnecessary). Otherwise connects to `DATABASE_URL` (defaulting the same
/// way `lakehouse_api::config::Config` does) and idempotently provisions
/// its own fixture. Returns `None` (never panics) if neither path
/// produces a usable token — the caller degrades to skipping non-public
/// entries with a clear message rather than failing opaquely.
async fn resolve_service_token() -> Option<String> {
    if let Ok(token) = std::env::var("PARITY_SERVICE_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://lakehouse:lakehouse@localhost:5432/lakehouse".to_owned());
    let pool = lakehouse_store::connect_lazy(&database_url).ok()?;

    let existing: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM service_identity WHERE name = $1")
            .bind(PARITY_SERVICE_IDENTITY_NAME)
            .fetch_optional(&pool)
            .await
            .ok()?;
    let service_identity_id = if let Some((id,)) = existing {
        id
    } else {
        let inserted: (uuid::Uuid,) = sqlx::query_as(
            "INSERT INTO service_identity (name, scopes, environment, expires_at) \
             VALUES ($1, $2, 'test', now() + interval '10 years') RETURNING id",
        )
        .bind(PARITY_SERVICE_IDENTITY_NAME)
        .bind(vec!["*:*".to_owned()])
        .fetch_one(&pool)
        .await
        .ok()?;
        inserted.0
    };

    let token =
        lakehouse_auth::service_token::create_service_credential(&pool, service_identity_id)
            .await
            .ok()?;
    Some(token.expose().to_owned())
}

fn load_corpus() -> Vec<(String, CorpusEntry)> {
    let dir = corpus_dir();
    let mut entries: Vec<(String, CorpusEntry)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("reading corpus dir {}: {err}", dir.display()))
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .map(|e| {
            let file_name = e.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(e.path())
                .unwrap_or_else(|err| panic!("reading {}: {err}", e.path().display()));
            let parsed: CorpusEntry = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("parsing {}: {err}", e.path().display()));
            (file_name, parsed)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

#[tokio::test]
#[ignore = "needs a live `lakehouse-api` server; see module docs for the opt-in command"]
#[allow(clippy::too_many_lines)] // one linear replay-and-assert loop; splitting it up would
// scatter the per-entry decision sequence (status → content-type → body) across files.
async fn parity_replays_corpus_against_live_service() {
    let target =
        std::env::var("PARITY_TARGET").unwrap_or_else(|_| "http://localhost:8080".to_owned());
    let public_dash_token = std::env::var("PUBLIC_DASH_TOKEN").ok();
    let service_token = resolve_service_token().await;
    if service_token.is_none() {
        eprintln!(
            "WARNING: no PARITY_SERVICE_TOKEN and no reachable DATABASE_URL to provision one — \
             every entry requiring authentication will be skipped. See this file's module doc \
             comment."
        );
    }

    let entries = load_corpus();
    assert!(
        !entries.is_empty(),
        "corpus dir at {:?} is empty",
        corpus_dir()
    );

    let client = reqwest::Client::new();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped = 0_usize;
    let mut passed = 0_usize;
    let mut replayed = 0_usize;

    for (file, entry) in &entries {
        let name = entry.request.name.clone();

        if entry.request.skip {
            eprintln!("SKIP {name} ({file}): request.skip = true");
            skipped += 1;
            continue;
        }
        let (Some(method), Some(raw_path)) =
            (entry.request.method.clone(), entry.request.path.clone())
        else {
            eprintln!("SKIP {name} ({file}): no method/path recorded");
            skipped += 1;
            continue;
        };

        let path = if raw_path.contains("__PUBLIC_DASH_TOKEN__") {
            if let Some(tok) = &public_dash_token {
                raw_path.replace("__PUBLIC_DASH_TOKEN__", tok)
            } else {
                eprintln!(
                    "SKIP {name}: path references __PUBLIC_DASH_TOKEN__ and $PUBLIC_DASH_TOKEN is unset"
                );
                skipped += 1;
                continue;
            }
        } else {
            raw_path
        };

        // Every corpus entry except the embed/public-dashboard ones now
        // needs authentication (Task 3.2) — see this file's module doc
        // comment. Those two ARE self-authenticated (a signed embed JWT / a
        // share-link token IS the credential), so they never need the
        // bearer header and must keep working with no service token at
        // all.
        let is_self_authenticated_entry =
            name.starts_with("embed-data") || name.starts_with("public-dash");
        if !is_self_authenticated_entry && service_token.is_none() {
            eprintln!("SKIP {name}: no service token available to authenticate this request");
            skipped += 1;
            continue;
        }

        replayed += 1;
        let url = format!("{}{path}", target.trim_end_matches('/'));
        let http_method: reqwest::Method = method.parse().unwrap_or(reqwest::Method::GET);
        let mut req = client.request(http_method, &url);
        if let Some(token) = &service_token {
            req = req.bearer_auth(token);
        }
        if let Some(raw) = &entry.request.raw {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(raw.clone());
        } else if let Some(body) = &entry.request.body {
            req = req.json(body);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(err) => {
                failures.push(format!("{name}: request to {url} failed: {err}"));
                continue;
            }
        };

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let expected_status = entry.request.expect_status.unwrap_or(entry.status);

        if status != expected_status {
            failures.push(format!(
                "{name}: status mismatch — expected {expected_status}, got {status}"
            ));
            continue;
        }
        if content_type != entry.content_type {
            failures.push(format!(
                "{name}: content-type mismatch — expected {:?}, got {:?}",
                entry.content_type, content_type
            ));
            continue;
        }

        if name == "dashboard-export" {
            let expected_yaml = entry
                .body
                .get("__nonJsonBody")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match resp.text().await {
                Ok(actual_yaml) if actual_yaml == expected_yaml => passed += 1,
                Ok(actual_yaml) => failures.push(diff_text(&name, expected_yaml, &actual_yaml)),
                Err(err) => failures.push(format!("{name}: reading body: {err}")),
            }
            continue;
        }

        let actual_body: Value = match resp.json().await {
            Ok(v) => v,
            Err(err) => {
                failures.push(format!("{name}: response body is not valid JSON: {err}"));
                continue;
            }
        };

        if STRUCTURE_ONLY.contains(&name.as_str()) {
            match compare_structure_only(&entry.body, &actual_body) {
                Ok(()) => passed += 1,
                Err(diff) => {
                    failures.push(format!(
                        "{name}: structure mismatch (model output) — {diff}"
                    ));
                }
            }
            continue;
        }

        let extra = extra_normalize_keys(&name);
        let mut norm_expected = normalize(&entry.body, None, &extra);
        let mut norm_actual = normalize(&actual_body, None, &extra);
        for unordered_key in unordered_array_keys(&name) {
            sort_array_by_id(&mut norm_expected, unordered_key);
            sort_array_by_id(&mut norm_actual, unordered_key);
        }
        for history_key in append_only_array_keys(&name) {
            reduce_array_to_element_shapes(&mut norm_expected, history_key);
            reduce_array_to_element_shapes(&mut norm_actual, history_key);
        }

        if norm_expected == norm_actual {
            passed += 1;
        } else {
            failures.push(diff_json(&name, &norm_expected, &norm_actual));
        }
    }

    println!(
        "parity: {replayed} replayed, {passed} passed, {} failed, {skipped} skipped (of {} corpus files)",
        failures.len(),
        entries.len()
    );

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("\n--- FAIL ---\n{f}");
        }
        panic!(
            "{} of {replayed} parity entries failed against {target} (see diffs above)",
            failures.len()
        );
    }
}
