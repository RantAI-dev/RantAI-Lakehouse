//! What governs a catalog asset: the policies bound to its tables and the
//! quality checks that name it, for the Access and Quality tabs of its
//! detail page (`catalog::detail`); and how fresh it is expected to be,
//! for the catalog list and the detail page alike.
//!
//! An asset goes by more than one name. A policy binds a qualified table
//! (`bronze.orders`, `silver.orders`); a quality rule's `asset` is whatever
//! its author typed, and an observed verdict's `tabel` is whatever the job
//! that wrote it uses. So both lookups take the list of names the caller
//! knows the asset by, and match any of them, ignoring case — the same
//! comparison `policy_engine` makes when it enforces.
//!
//! It also answers who uses the asset: the saved queries and dashboards
//! whose SQL reads it ([`dependents`]) and the queries run against it in
//! the last week ([`usage`]). Nothing records which tables a query
//! touched, so both parse the SQL itself with the same table resolver the
//! policy rewrite uses, and count a query only when it really reads the
//! table — not when the name merely appears in it.
//!
//! No lookup here can fail the detail page: a store or `ClickHouse` error
//! is logged and reads as "none", like the sample and the parts stats.

use std::collections::HashMap;

use lakehouse_auth::Principal;
use lakehouse_store::governance::{self as store, ClassificationRule, Policy, QualityRule};
use lakehouse_store::queries::HistoryMention;
use serde_json::{Map, Value, json};
use sqlparser::dialect::{ClickHouseDialect, GenericDialect};

use crate::policy_engine::PolicyCondition;
use crate::routes::governance::observed_quality;
use crate::routes::quality::{LatestRun, latest_runs, threshold_hint};
use crate::state::AppState;

/// Who may see a policy's roles and row filter. Anyone with `catalog:read`
/// learns that a policy exists and what it masks for them — they see the
/// `***` anyway — but who else it targets, and the predicate that hides
/// rows, are the policy's own content.
const POLICY_READ: &str = "policy:read";

/// Seconds in a day, the unit refresh frequencies are stated in.
const DAY_SECONDS: i64 = 86_400;

/// The age beyond which a dataset the registry says is refreshed
/// `frekuensi` is late — `None` for a frequency this does not know, which
/// then gets no verdict at all rather than a guessed one.
///
/// The registry states a cadence ("harian"), not a deadline, so the target
/// is one and a half intervals: a daily load that ran yesterday morning
/// and runs tonight is still daily. An authored SLA ([`sla_target_seconds`])
/// is a deadline, and is used as written.
pub(crate) fn frequency_target_seconds(frekuensi: &str) -> Option<i64> {
    let days = match frekuensi.trim().to_lowercase().as_str() {
        "harian" | "daily" => 1,
        "mingguan" | "weekly" => 7,
        "bulanan" | "monthly" => 31,
        "triwulanan" | "kuartalan" | "quarterly" => 92,
        "tahunan" | "yearly" | "annual" => 366,
        _ => return None,
    };
    Some(days * DAY_SECONDS * 3 / 2)
}

/// Every authored freshness SLA as `lower-cased table key → seconds`, for
/// the catalog list. A store failure is logged and reads as "none": the
/// list then falls back to registry frequencies, it does not fail.
pub(crate) async fn sla_targets(state: &AppState) -> HashMap<String, i64> {
    let Some(pg) = state.pg.as_deref() else {
        return HashMap::new();
    };
    match store::list_dataset_sla(pg).await {
        Ok(rows) => rows
            .into_iter()
            .map(|sla| {
                (
                    sla.table_name.to_lowercase(),
                    i64::from(sla.expected_interval_minutes) * 60,
                )
            })
            .collect(),
        Err(err) => {
            tracing::warn!(?err, "catalog: freshness SLAs unavailable");
            HashMap::new()
        }
    }
}

/// The authored freshness SLA for one table key (`bronze.orders`), in
/// seconds, when there is one.
pub(crate) async fn sla_target_seconds(state: &AppState, table_key: &str) -> Option<i64> {
    sla_targets(state)
        .await
        .get(&table_key.to_lowercase())
        .copied()
}

/// Whether `sql` reads one of `tables` (qualified keys, e.g.
/// `silver.orders`). Parsed as `ClickHouse` first, then generically for
/// the Trino form Query Studio also runs; SQL neither can parse reads
/// nothing as far as this can prove, and is left out.
fn reads_any(sql: &str, tables: &[String]) -> bool {
    let lower = sql.to_lowercase();
    // The parse is the expensive part: skip it when no table's own name
    // even appears in the text.
    if !tables
        .iter()
        .any(|t| lower.contains(&bare_name(t).to_lowercase()))
    {
        return false;
    }
    let referenced = crate::sql_rewrite::referenced_tables(sql, &ClickHouseDialect {})
        .or_else(|| crate::sql_rewrite::referenced_tables(sql, &GenericDialect {}));
    referenced.is_some_and(|names| names.iter().any(|name| is_one_of(name, tables)))
}

/// `orders` of `silver.orders`: what a table's key ends in.
fn bare_name(table_key: &str) -> &str {
    table_key.rsplit('.').next().unwrap_or(table_key)
}

/// What reads the asset besides pipelines (those come from the lineage
/// graph): saved queries, and dashboards with a chart on it — one entry
/// per dashboard, however many of its charts read the table. Each kind is
/// listed only to a caller who may open it (`query:read`,
/// `dashboard:read`).
pub(crate) async fn dependents(
    state: &AppState,
    principal: &Principal,
    tables: &[String],
) -> Vec<Value> {
    let mut out = Vec::new();
    if principal.has("query:read")
        && let Some(pg) = state.pg.as_deref()
    {
        match lakehouse_store::queries::list_saved(pg).await {
            Ok(saved) => out.extend(
                saved
                    .iter()
                    .filter(|q| reads_any(&q.sql, tables))
                    .map(|q| json!({ "id": q.id, "name": q.title, "kind": "saved query" })),
            ),
            Err(err) => tracing::warn!(?err, "catalog detail: saved queries unavailable"),
        }
    }
    if principal.has("dashboard:read") {
        out.extend(dashboard_dependents(state, tables).await);
    }
    out
}

async fn dashboard_dependents(state: &AppState, tables: &[String]) -> Vec<Value> {
    let ch = &state.clickhouse;
    let charts = match lakehouse_bi::store::list_stored_charts(ch).await {
        Ok(charts) => charts,
        Err(err) => {
            tracing::warn!(?err, "catalog detail: dashboard charts unavailable");
            return Vec::new();
        }
    };
    // Board id → how many of its charts read the asset, in first-seen order.
    let mut boards: Vec<(String, usize)> = Vec::new();
    let mut count = |board: &str| match boards.iter_mut().find(|(id, _)| id == board) {
        Some((_, n)) => *n += 1,
        None => boards.push((board.to_owned(), 1)),
    };
    for chart in charts.iter().filter(|c| reads_any(&c.spec.sql, tables)) {
        count(&chart.board);
    }
    // The default dashboard also shows the charts and KPI tiles this
    // deployment ships with (`lakehouse_bi::specs`), which are stored
    // nowhere: they read the asset as much as a saved chart does.
    let builtin = lakehouse_bi::specs::charts()
        .iter()
        .map(|c| c.sql.as_str())
        .chain(lakehouse_bi::specs::kpis().iter().map(|k| k.sql.as_str()))
        .filter(|sql| reads_any(sql, tables))
        .count();
    for _ in 0..builtin {
        count(DEFAULT_BOARD);
    }
    if boards.is_empty() {
        return Vec::new();
    }
    let names: HashMap<String, String> = match lakehouse_bi::store::list_boards(ch).await {
        Ok(list) => list.into_iter().map(|b| (b.id, b.name)).collect(),
        Err(err) => {
            tracing::warn!(?err, "catalog detail: dashboard names unavailable");
            HashMap::new()
        }
    };
    boards
        .into_iter()
        .map(|(id, charts)| {
            json!({
                "name": names.get(&id).cloned().unwrap_or_else(|| id.clone()),
                "id": id,
                "kind": "dashboard",
                "detail": format!("{charts} chart{}", if charts == 1 { "" } else { "s" }),
            })
        })
        .collect()
}

/// The board every deployment has, with no row in `console.bi_board`.
const DEFAULT_BOARD: &str = "default";

/// The window [`usage`] looks back over.
const USAGE_DAYS: i32 = 7;
/// How many of the caller's own recent queries [`usage`] returns.
const RECENT_QUERIES: usize = 5;

/// `usage` (queries, distinct people and average latency over the last
/// seven days) and `recentQueries` from the rows that really read one of
/// `tables`.
///
/// The counts cover everyone's queries; the query text does not. SQL can
/// carry literals — a customer id, a name — so `recentQueries` lists only
/// the caller's own. Latency averages completed runs only: a query that
/// failed or was blocked took no time to answer.
fn summarize_usage(
    mentions: &[HistoryMention],
    tables: &[String],
    caller: uuid::Uuid,
) -> (Value, Vec<Value>) {
    let reads: Vec<&HistoryMention> = mentions
        .iter()
        .filter(|m| reads_any(&m.sql, tables))
        .collect();
    let people: std::collections::HashSet<String> = reads
        .iter()
        .map(|m| {
            m.owner_id
                .map_or_else(|| m.user_name.clone(), |id| id.to_string())
        })
        .collect();
    let completed: Vec<i64> = reads
        .iter()
        .filter(|m| m.status == "completed")
        .map(|m| m.duration_ms)
        .collect();
    let avg_ms = if completed.is_empty() {
        0
    } else {
        completed.iter().sum::<i64>() / i64::try_from(completed.len()).unwrap_or(1)
    };
    let usage = json!({
        "queries7d": reads.len(),
        "users7d": people.len(),
        "avgLatencyMs": avg_ms,
    });
    let recent = reads
        .iter()
        .filter(|m| m.owner_id == Some(caller))
        .take(RECENT_QUERIES)
        .map(|m| {
            json!({
                "id": m.id,
                "sql": m.sql,
                "user": m.user_name,
                "at": m.at,
                "status": m.status,
                "auditEventId": m.audit_event_id,
            })
        })
        .collect();
    (usage, recent)
}

/// [`summarize_usage`] over the query history. `usage` is `null` — "not
/// measured" — only when the history cannot be read at all; a table nobody
/// queried this week is a measured zero.
pub(crate) async fn usage(
    state: &AppState,
    principal: &Principal,
    tables: &[String],
) -> (Value, Vec<Value>) {
    let Some(pg) = state.pg.as_deref() else {
        return (Value::Null, Vec::new());
    };
    let needles: Vec<String> = tables.iter().map(|t| bare_name(t).to_owned()).collect();
    match lakehouse_store::queries::history_mentioning(pg, USAGE_DAYS, &needles).await {
        Ok(mentions) => summarize_usage(&mentions, tables, principal.id.uuid()),
        Err(err) => {
            tracing::warn!(?err, "catalog detail: query history unavailable");
            (Value::Null, Vec::new())
        }
    }
}

/// How many audit events an asset's Change history shows.
const CHANGE_HISTORY_LIMIT: i64 = 50;

/// One audit event about a catalog asset, as a `changeHistory` entry: who,
/// when, and one line saying what they did.
fn change_entry(event: &lakehouse_store::audit::AuditEvent) -> Value {
    let arg = |key: &str| event.args.get(key).and_then(Value::as_str);
    let summary = match event.action.as_str() {
        "catalog.annotate" => {
            let fields: Vec<&str> = event
                .args
                .get("fields")
                .and_then(Value::as_array)
                .map(|f| f.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if fields.is_empty() {
                "Edited the asset's details".to_owned()
            } else {
                format!("Edited {}", fields.join(", "))
            }
        }
        "catalog.access_request" => arg("permission").map_or_else(
            || "Requested access".to_owned(),
            |permission| format!("Requested {permission}"),
        ),
        "catalog.classify" => format!(
            "Classified {} as {}",
            arg("column").map_or_else(|| "the asset".to_owned(), |c| format!("column {c}")),
            arg("classification").unwrap_or("unspecified"),
        ),
        "catalog.declassify" => format!(
            "Removed the classification of {} ({})",
            arg("column").map_or_else(|| "the asset".to_owned(), |c| format!("column {c}")),
            arg("classification").unwrap_or("unspecified"),
        ),
        "catalog.sla_remove" => "Removed the freshness target".to_owned(),
        "catalog.sla_set" => event
            .args
            .get("minutes")
            .and_then(Value::as_i64)
            .map_or_else(
                || "Set a freshness target".to_owned(),
                |minutes| format!("Set the freshness target to {}", span(minutes * 60)),
            ),
        "quality.rule_create" => format!("Added quality rule {}", arg("name").unwrap_or("")),
        "quality.rule_update" => {
            // Only the parts the rewrite changed (`changed`), each with
            // what it became.
            let changed = |part: &str| {
                event
                    .args
                    .get("changed")
                    .and_then(Value::as_array)
                    .is_some_and(|c| c.iter().any(|p| p.as_str() == Some(part)))
            };
            let parts: Vec<String> = [
                ("threshold", "threshold to \""),
                ("asset", "table to "),
                ("severity", "severity to "),
            ]
            .into_iter()
            .filter(|(part, _)| changed(part))
            .filter_map(|(part, words)| {
                let value = arg(part)?;
                let quote = if part == "threshold" { "\"" } else { "" };
                Some(format!("{words}{value}{quote}"))
            })
            .collect();
            let name = arg("name").unwrap_or("");
            if parts.is_empty() {
                format!("Changed quality rule {name}")
            } else {
                format!("Changed quality rule {name}: {}", parts.join(", "))
            }
        }
        "quality.rule_delete" => format!("Deleted quality rule {}", arg("name").unwrap_or("")),
        "policy.create" => format!("Added policy {}", arg("name").unwrap_or("")),
        "policy.enforce" => format!("Enforced policy {}", arg("name").unwrap_or("")),
        "policy.suspend" => format!("Stopped enforcing policy {}", arg("name").unwrap_or("")),
        "policy.delete" => {
            let was_enforced = event.args.get("enforced").and_then(Value::as_bool) == Some(true);
            format!(
                "Deleted policy {}{}",
                arg("name").unwrap_or(""),
                if was_enforced {
                    " (it was enforced)"
                } else {
                    ""
                }
            )
        }
        other => other.to_owned(),
    };
    json!({
        "id": event.id,
        "at": event.at,
        "actor": event
            .actor_label
            .clone()
            .or_else(|| event.principal_id.clone())
            .unwrap_or_else(|| "Unknown".to_owned()),
        "summary": summary.trim_end(),
    })
}

/// What people did to a catalog asset, newest first, from the audit trail
/// (`resource_kind = "catalog"`): edits to its details, access requests,
/// and the rules, classifications, policies and freshness targets added
/// for it. `keys` is every id the asset is recorded under — its catalog
/// id, and its table keys, which is what a rule names. Loads are the
/// Snapshots list's story, not this one's.
pub(crate) async fn change_history(state: &AppState, keys: &[String]) -> Vec<Value> {
    let Some(pg) = state.pg.as_deref() else {
        return Vec::new();
    };
    // A catalog id is recorded as written; a table key lower-cased
    // (`routes::governance::audit_table_change`). Ask for both spellings.
    let mut ids: Vec<String> = Vec::new();
    for key in keys {
        for id in [key.clone(), key.to_lowercase()] {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    match lakehouse_store::audit::list_for_resources(pg, "catalog", &ids, CHANGE_HISTORY_LIMIT)
        .await
    {
        Ok(events) => events.iter().map(change_entry).collect(),
        Err(err) => {
            tracing::warn!(?err, "catalog detail: change history unavailable");
            Vec::new()
        }
    }
}

fn is_one_of(name: &str, names: &[String]) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// One `policySummary` entry per policy whose condition binds one of
/// `tables`. A policy with no enforceable condition (legacy prose, or
/// none) governs no particular table and is left out.
///
/// `appliesToYou` is true only for a `ready` policy that names one of
/// `roles` — exactly when `policy_engine` would enforce it on the caller.
fn summarize_policies(
    policies: &[Policy],
    tables: &[String],
    roles: &[String],
    full: bool,
) -> Vec<Value> {
    policies
        .iter()
        .filter_map(|policy| {
            let cond = PolicyCondition::parse_opt(policy.conditions.as_deref())?;
            if !is_one_of(&cond.table, tables) {
                return None;
            }
            let applies = policy.status == "ready" && cond.roles.iter().any(|r| roles.contains(r));
            let mut o = Map::new();
            o.insert("id".to_owned(), json!(policy.id));
            o.insert("name".to_owned(), json!(policy.name));
            o.insert("kind".to_owned(), json!(policy.kind));
            o.insert("effect".to_owned(), json!(policy.effect));
            o.insert("status".to_owned(), json!(policy.status));
            o.insert("table".to_owned(), json!(cond.table));
            o.insert("appliesToYou".to_owned(), json!(applies));
            if full {
                o.insert("roles".to_owned(), json!(cond.roles));
                o.insert("rowFilter".to_owned(), json!(cond.row_filter));
            }
            if full || applies {
                o.insert("mask".to_owned(), json!(cond.mask));
            }
            Some(Value::Object(o))
        })
        .collect()
}

/// The policies bound to any of `tables`, as `principal` may see them.
pub(crate) async fn policy_summary(
    state: &AppState,
    principal: &Principal,
    tables: &[String],
) -> Vec<Value> {
    let Some(pg) = state.pg.as_deref() else {
        return Vec::new();
    };
    match store::list_policies(pg).await {
        Ok(policies) => summarize_policies(
            &policies,
            tables,
            &principal.role_names,
            principal.has(POLICY_READ),
        ),
        Err(err) => {
            tracing::warn!(?err, "catalog detail: policies unavailable");
            Vec::new()
        }
    }
}

/// An authored rule as a `qualityChecks` entry: `status`, `lastRun` and
/// `value` from its latest run (`routes::quality`), all `null` for a rule
/// nobody has run; `evaluable` and, when it is not, the `hint` saying how
/// to write a threshold that is.
fn rule_check(rule: &QualityRule, run: Option<&LatestRun>) -> Value {
    let hint = threshold_hint(&rule.threshold);
    json!({
        "id": rule.id,
        "name": rule.name,
        "asset": rule.asset,
        "dimension": rule.dimension,
        "threshold": rule.threshold,
        "severity": rule.severity,
        "status": run.map(|r| r.status.as_str()),
        "lastRun": run.map(|r| r.at.as_str()),
        "value": run.map(|r| r.value.as_str()),
        "evaluable": hint.is_none(),
        "hint": hint,
        "origin": "rule",
    })
}

/// An observed verdict ([`observed_quality`]'s `QualityRule` shape) as a
/// `qualityChecks` entry. Its list-wide `q-<n>` id means nothing here, so
/// the entry is keyed by what it checks.
fn observed_check(observed: &Value) -> Value {
    let text = |key: &str| {
        observed
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    json!({
        "id": format!("{}/{}", text("asset"), text("name")),
        "name": text("name"),
        "dimension": text("dimension"),
        "threshold": text("threshold"),
        "severity": text("severity"),
        "status": text("lastStatus"),
        "lastRun": text("lastRunAt"),
        "origin": "observed",
    })
}

/// Everything quality knows, loaded once: the verdicts a quality job
/// recorded, the authored rules, and each rule's latest run. The detail
/// route asks it about one asset; the catalog list about every row, for
/// their health.
pub(crate) struct QualityIndex {
    observed: Vec<Value>,
    rules: Vec<QualityRule>,
    runs: HashMap<String, LatestRun>,
}

impl QualityIndex {
    /// Loads the index — for the assets named in `only` when given (the
    /// detail route knows which names it will ask about), for every asset
    /// otherwise. Each source that fails is logged and reads as empty.
    pub(crate) async fn load(state: &AppState, only: Option<&[String]>) -> Self {
        let observed = match observed_quality(&state.clickhouse, only).await {
            Ok(observed) => observed,
            Err(err) => {
                tracing::warn!(?err, "catalog: observed quality unavailable");
                Vec::new()
            }
        };
        let rules = match state.pg.as_deref() {
            Some(pg) => store::list_quality_rules(pg).await.unwrap_or_else(|err| {
                tracing::warn!(?err, "catalog: quality rules unavailable");
                Vec::new()
            }),
            None => Vec::new(),
        };
        let rules: Vec<QualityRule> = match only {
            Some(names) => rules
                .into_iter()
                .filter(|r| is_one_of(&r.asset, names))
                .collect(),
            None => rules,
        };
        // Only worth asking `ClickHouse` when there is a rule to have run.
        let runs = if rules.is_empty() {
            HashMap::new()
        } else {
            latest_runs(&state.clickhouse).await
        };
        Self {
            observed,
            rules,
            runs,
        }
    }

    /// Every quality check that names the asset by one of `names`:
    /// verdicts a quality job recorded first (they carry a result), then
    /// authored rules.
    pub(crate) fn checks_for(&self, names: &[String]) -> Vec<Value> {
        let about_asset = |v: &Value| {
            v.get("asset")
                .and_then(Value::as_str)
                .is_some_and(|asset| is_one_of(asset, names))
        };
        self.observed
            .iter()
            .filter(|v| about_asset(v))
            .map(observed_check)
            .chain(
                self.rules
                    .iter()
                    .filter(|r| is_one_of(&r.asset, names))
                    .map(|r| rule_check(r, self.runs.get(&r.id))),
            )
            .collect()
    }
}

// ── Classification ─────────────────────────────────────────────────────

/// Classification levels, least to most restrictive.
const LEVELS: [&str; 4] = ["public", "internal", "confidential", "restricted"];

/// What an asset nobody classified is treated as — the same level
/// `GET /api/governance/classification` lists every catalog asset under.
const DEFAULT_LEVEL: &str = "internal";

fn rank(level: &str) -> usize {
    LEVELS.iter().position(|l| *l == level).unwrap_or(0)
}

/// An asset's classification, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Classified {
    /// The asset's level: its own rule's, raised to its most restrictive
    /// column's.
    pub(crate) level: String,
    /// `false` when no rule names the asset and the level is the default.
    pub(crate) from_rule: bool,
    /// `(column, level)` for each column a rule classifies.
    pub(crate) columns: Vec<(String, String)>,
    /// The rules behind the above — the asset's own first, then one per
    /// column — as `(rule id, column, level)`: what removing a
    /// classification removes.
    pub(crate) in_force: Vec<(String, Option<String>, String)>,
}

/// Classifies the asset known by `names` from `rules` (newest first, as
/// the store lists them). The newest rule wins — per column, and for the
/// asset as a whole — so a wrong classification is corrected by adding the
/// right one. The asset is never less restrictive than a column it holds.
pub(crate) fn classify(rules: &[ClassificationRule], names: &[String]) -> Classified {
    let mut asset_level: Option<&str> = None;
    let mut asset_rule: Option<(String, Option<String>, String)> = None;
    let mut columns: Vec<(String, String)> = Vec::new();
    let mut column_rules: Vec<(String, Option<String>, String)> = Vec::new();
    for rule in rules.iter().filter(|r| is_one_of(&r.asset, names)) {
        match rule.column.as_deref().filter(|c| !c.trim().is_empty()) {
            None => {
                if asset_level.is_none() {
                    asset_level = Some(rule.classification.as_str());
                    asset_rule = Some((rule.id.clone(), None, rule.classification.clone()));
                }
            }
            Some(column) => {
                if !columns.iter().any(|(c, _)| c == column) {
                    columns.push((column.to_owned(), rule.classification.clone()));
                    column_rules.push((
                        rule.id.clone(),
                        Some(column.to_owned()),
                        rule.classification.clone(),
                    ));
                }
            }
        }
    }
    let level = columns
        .iter()
        .map(|(_, level)| level.as_str())
        .chain(std::iter::once(asset_level.unwrap_or(DEFAULT_LEVEL)))
        .max_by_key(|level| rank(level))
        .unwrap_or(DEFAULT_LEVEL);
    Classified {
        level: level.to_owned(),
        from_rule: asset_level.is_some() || !columns.is_empty(),
        columns,
        in_force: asset_rule.into_iter().chain(column_rules).collect(),
    }
}

/// Every authored classification rule, newest first. A store failure is
/// logged and reads as "no rules": assets then show the default level.
pub(crate) async fn classification_rules(state: &AppState) -> Vec<ClassificationRule> {
    let Some(pg) = state.pg.as_deref() else {
        return Vec::new();
    };
    store::list_classification_rules(pg)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(?err, "catalog: classification rules unavailable");
            Vec::new()
        })
}

// ── Health ─────────────────────────────────────────────────────────────

/// A duration as a person says it: `8d 3h`, `5h 30m`, `45m`.
fn span(seconds: i64) -> String {
    let minutes = seconds / 60;
    let (days, hours) = (minutes / 1440, minutes / 60);
    if days >= 2 {
        format!("{days}d {}h", hours % 24)
    } else if hours >= 1 {
        format!("{hours}h {:02}m", minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// An asset's health from what is actually measured about it, with one
/// line per signal saying why:
///
/// - freshness, when the asset has a target to be judged against;
/// - its quality checks that have a result.
///
/// `unhealthy` when a critical- or high-severity check failed; `degraded`
/// when it is late, or any other check failed or could not be judged;
/// `healthy` when every signal there is passes; `unknown`, with no
/// reasons, when there is no signal at all — never "healthy" by default.
pub(crate) fn health(
    lag_seconds: Option<i64>,
    target_seconds: Option<i64>,
    checks: &[Value],
) -> (&'static str, Vec<String>) {
    let mut reasons = Vec::new();
    let mut degraded = false;
    let mut unhealthy = false;

    if let (Some(lag), Some(target)) = (lag_seconds, target_seconds) {
        if lag <= target {
            reasons.push(format!(
                "Fresh: written {} ago, within its {} target",
                span(lag),
                span(target)
            ));
        } else {
            degraded = true;
            reasons.push(format!(
                "Late: written {} ago, expected within {}",
                span(lag),
                span(target)
            ));
        }
    }

    let text = |check: &Value, key: &str| {
        check
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let evaluated: Vec<&Value> = checks
        .iter()
        .filter(|c| c.get("status").and_then(Value::as_str).is_some())
        .collect();
    if !evaluated.is_empty() {
        let passed = evaluated
            .iter()
            .filter(|c| text(c, "status") == "passed")
            .count();
        reasons.push(format!(
            "{passed} of {} quality checks passed",
            evaluated.len()
        ));
        let failed: Vec<String> = evaluated
            .iter()
            .filter(|c| text(c, "status") == "failed")
            .map(|c| text(c, "name"))
            .collect();
        if !failed.is_empty() {
            reasons.push(format!("Failed: {}", failed.join(", ")));
        }
        if passed < evaluated.len() {
            degraded = true;
        }
        unhealthy = evaluated.iter().any(|c| {
            text(c, "status") == "failed"
                && matches!(text(c, "severity").as_str(), "critical" | "high")
        });
    }

    let status = if unhealthy {
        "unhealthy"
    } else if degraded {
        "degraded"
    } else if reasons.is_empty() {
        "unknown"
    } else {
        "healthy"
    };
    (status, reasons)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn policy(name: &str, status: &str, conditions: Option<&str>) -> Policy {
        Policy {
            id: format!("id-{name}"),
            name: name.to_owned(),
            status: status.to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Analyst".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            version: 1,
            owner: "governance".to_owned(),
            updated_at: "2026-09-30T00:00:00Z".to_owned(),
            conditions: conditions.map(str::to_owned),
        }
    }

    const MASK_EMAIL: &str = r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"],"rowFilter":"region = 'ID'"}"#;

    fn tables(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// Only a policy whose condition binds one of the asset's tables is
    /// listed; prose-only and other-table policies are not.
    #[test]
    fn lists_only_policies_bound_to_the_assets_tables() {
        let policies = [
            policy("mask-email", "ready", Some(MASK_EMAIL)),
            policy("legacy", "ready", Some("analysts may read aggregated data")),
            policy("no-condition", "ready", None),
            policy(
                "elsewhere",
                "ready",
                Some(r#"{"roles":["Analyst"],"table":"silver.other","mask":["x"]}"#),
            ),
        ];
        let summary = summarize_policies(&policies, &tables(&["SERVING.mart_x"]), &[], true);
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0]["name"], "mask-email");
        assert_eq!(summary[0]["table"], "serving.mart_x");
    }

    /// With `policy:read` the summary carries who the policy targets and
    /// its row filter; `appliesToYou` follows the caller's own roles.
    #[test]
    fn a_policy_reader_sees_roles_mask_and_row_filter() {
        let policies = [policy("mask-email", "ready", Some(MASK_EMAIL))];
        let roles = vec!["Governance Admin".to_owned()];
        let summary = summarize_policies(&policies, &tables(&["serving.mart_x"]), &roles, true);
        assert_eq!(summary[0]["appliesToYou"], false);
        assert_eq!(summary[0]["roles"], json!(["Analyst"]));
        assert_eq!(summary[0]["mask"], json!(["email"]));
        assert_eq!(summary[0]["rowFilter"], "region = 'ID'");
    }

    /// Without `policy:read`, a governed caller learns the policy applies
    /// and what it masks for them — never the other roles or the predicate.
    #[test]
    fn a_governed_caller_sees_their_mask_but_not_roles_or_the_filter() {
        let policies = [policy("mask-email", "ready", Some(MASK_EMAIL))];
        let analyst = vec!["Analyst".to_owned()];
        let summary = summarize_policies(&policies, &tables(&["serving.mart_x"]), &analyst, false);
        assert_eq!(summary[0]["appliesToYou"], true);
        assert_eq!(summary[0]["mask"], json!(["email"]));
        assert!(summary[0].get("roles").is_none());
        assert!(summary[0].get("rowFilter").is_none());

        // A caller it does not target, without `policy:read`, sees neither.
        let other = summarize_policies(&policies, &tables(&["serving.mart_x"]), &[], false);
        assert_eq!(other[0]["appliesToYou"], false);
        assert!(other[0].get("mask").is_none());
    }

    /// A draft is listed, so its author can find it, but enforces nothing.
    #[test]
    fn a_draft_policy_never_applies() {
        let policies = [policy("mask-email", "draft", Some(MASK_EMAIL))];
        let analyst = vec!["Analyst".to_owned()];
        let summary = summarize_policies(&policies, &tables(&["serving.mart_x"]), &analyst, false);
        assert_eq!(summary[0]["status"], "draft");
        assert_eq!(summary[0]["appliesToYou"], false);
    }

    /// An authored rule nothing has evaluated reports no verdict, rather
    /// than the table's placeholder one.
    #[test]
    fn an_unevaluated_rule_has_no_status() {
        let rule = QualityRule {
            id: "r1".to_owned(),
            name: "orders_uniqueness".to_owned(),
            asset: "silver.orders".to_owned(),
            dimension: "uniqueness".to_owned(),
            threshold: "payment_id unique".to_owned(),
            severity: "high".to_owned(),
            last_status: None,
            last_run_at: None,
        };
        let check = rule_check(&rule, None);
        assert_eq!(check["status"], Value::Null);
        assert_eq!(check["lastRun"], Value::Null);
        assert_eq!(check["origin"], "rule");
        assert_eq!(check["evaluable"], true);

        // Once run, it reports that run — and nothing older.
        let run = LatestRun {
            status: "failed".to_owned(),
            value: "3 repeated values in 50 rows".to_owned(),
            at: "2026-10-01T09:00:00Z".to_owned(),
        };
        let ran = rule_check(&rule, Some(&run));
        assert_eq!(ran["status"], "failed");
        assert_eq!(ran["value"], "3 repeated values in 50 rows");
        assert_eq!(ran["lastRun"], "2026-10-01T09:00:00Z");
    }

    /// A threshold nobody can evaluate says so, with how to fix it.
    #[test]
    fn a_rule_in_prose_is_marked_not_evaluable() {
        let rule = QualityRule {
            id: "r2".to_owned(),
            name: "email_verified_completeness".to_owned(),
            asset: "serving.mart_customer_segment".to_owned(),
            dimension: "completeness".to_owned(),
            threshold: ">= 95%".to_owned(),
            severity: "medium".to_owned(),
            last_status: None,
            last_run_at: None,
        };
        let check = rule_check(&rule, None);
        assert_eq!(check["evaluable"], false);
        assert!(
            check["hint"]
                .as_str()
                .is_some_and(|h| h.contains("not null"))
        );
    }

    /// A stated cadence gets half an interval of slack; a frequency this
    /// does not know gets no target, never a guessed one.
    #[test]
    fn a_known_frequency_allows_one_and_a_half_intervals() {
        assert_eq!(frequency_target_seconds("harian"), Some(129_600));
        assert_eq!(frequency_target_seconds(" Mingguan "), Some(907_200));
        assert_eq!(frequency_target_seconds(""), None);
        assert_eq!(frequency_target_seconds("sewaktu-waktu"), None);
    }

    /// A query reads a table when the table is in its FROM, on either
    /// engine's spelling — not when the name only appears in the text.
    #[test]
    fn reads_any_needs_the_table_in_the_query_not_just_in_its_text() {
        let orders = tables(&["bronze.orders"]);
        assert!(reads_any(
            "SELECT * FROM icecat_api.`bronze.orders` LIMIT 5",
            &orders
        ));
        // `WHERE 1` keeps this fixture clear of the R11 lint
        // (`ops/lint/check_bare_iceberg_count.py`), which reads test text
        // too and takes any unqualified row count over a `bronze.` name as
        // the defect it guards against. The query's meaning here is only
        // which table it reads.
        assert!(reads_any(
            "SELECT count(*) FROM iceberg.bronze.orders WHERE 1",
            &orders
        ));
        assert!(reads_any(
            "SELECT o.id FROM silver.x x JOIN bronze.orders o ON o.id = x.id",
            &orders
        ));
        // Another table whose name contains this one's.
        assert!(!reads_any("SELECT * FROM bronze.orders_archive", &orders));
        // The name in a literal, and the same table name in another zone.
        assert!(!reads_any(
            "SELECT 'bronze.orders' AS t FROM silver.x",
            &orders
        ));
        assert!(!reads_any("SELECT * FROM silver.orders", &orders));
        // Not SQL at all.
        assert!(!reads_any("orders, please", &orders));
    }

    fn mention(id: &str, sql: &str, owner: uuid::Uuid, status: &str, ms: i64) -> HistoryMention {
        HistoryMention {
            id: id.to_owned(),
            sql: sql.to_owned(),
            user_name: "someone".to_owned(),
            owner_id: Some(owner),
            at: "2026-10-01T08:00:00Z".to_owned(),
            status: status.to_owned(),
            duration_ms: ms,
            audit_event_id: Some(format!("audit-{id}")),
        }
    }

    /// Counts cover everyone who really read the table; the query text
    /// shown back is only the caller's own.
    #[test]
    fn usage_counts_everyone_but_lists_only_the_callers_queries() {
        let (me, other) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let mentions = [
            mention("q1", "SELECT * FROM silver.orders", me, "completed", 100),
            mention(
                "q2",
                "SELECT * FROM silver.orders WHERE id = 7",
                other,
                "completed",
                300,
            ),
            mention("q3", "SELECT * FROM silver.orders", other, "failed", 9_000),
            // Mentions the name without reading the table.
            mention(
                "q4",
                "SELECT * FROM silver.orders_archive",
                me,
                "completed",
                50,
            ),
        ];
        let (usage, recent) = summarize_usage(&mentions, &tables(&["silver.orders"]), me);
        assert_eq!(usage["queries7d"], 3);
        assert_eq!(usage["users7d"], 2);
        // The failed run's nine seconds are not a latency.
        assert_eq!(usage["avgLatencyMs"], 200);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["id"], "q1");
        assert_eq!(recent[0]["auditEventId"], "audit-q1");
    }

    /// A table nobody queried is a measured zero, not "not measured".
    #[test]
    fn usage_of_an_unqueried_table_is_zero() {
        let (usage, recent) =
            summarize_usage(&[], &tables(&["silver.orders"]), uuid::Uuid::from_u128(1));
        assert_eq!(usage["queries7d"], 0);
        assert_eq!(usage["avgLatencyMs"], 0);
        assert!(recent.is_empty());
    }

    fn audit_event(action: &str, args: Value) -> lakehouse_store::audit::AuditEvent {
        lakehouse_store::audit::AuditEvent {
            id: "audit-1".to_owned(),
            at: "2026-10-01T08:00:00Z".to_owned(),
            principal_id: Some("u-1".to_owned()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some("Rina".to_owned()),
            action: action.to_owned(),
            resource_kind: Some("catalog".to_owned()),
            resource_id: Some("silver.orders".to_owned()),
            args,
            outcome: "executed".to_owned(),
            detail: None,
            run_id: None,
            approval_id: None,
            session_id: None,
        }
    }

    /// The history line says what was done in plain words, from the
    /// fields the audit event recorded.
    #[test]
    fn change_entries_say_what_was_edited_or_requested() {
        let edit = change_entry(&audit_event(
            "catalog.annotate",
            json!({ "fields": ["description", "owner"] }),
        ));
        assert_eq!(edit["actor"], "Rina");
        assert_eq!(edit["summary"], "Edited description, owner");
        assert_eq!(edit["at"], "2026-10-01T08:00:00Z");

        let request = change_entry(&audit_event(
            "catalog.access_request",
            json!({ "permission": "query:read" }),
        ));
        assert_eq!(request["summary"], "Requested query:read");

        // An older event recorded no permission.
        let bare = change_entry(&audit_event("catalog.access_request", json!({})));
        assert_eq!(bare["summary"], "Requested access");

        // What was added for the asset's table, in the same list.
        for (action, args, summary) in [
            (
                "catalog.classify",
                json!({ "column": "email", "classification": "restricted" }),
                "Classified column email as restricted",
            ),
            (
                "catalog.classify",
                json!({ "column": null, "classification": "public" }),
                "Classified the asset as public",
            ),
            (
                "catalog.sla_set",
                json!({ "minutes": 2160 }),
                "Set the freshness target to 36h 00m",
            ),
            (
                "quality.rule_create",
                json!({ "name": "orders_id_unique" }),
                "Added quality rule orders_id_unique",
            ),
            (
                "quality.rule_update",
                json!({
                    "name": "orders_id_unique",
                    "changed": ["threshold", "severity"],
                    "threshold": "order_id unique",
                    "asset": "silver.orders",
                    "severity": "high",
                }),
                "Changed quality rule orders_id_unique: threshold to \"order_id unique\", severity to high",
            ),
            (
                "quality.rule_update",
                json!({
                    "name": "orders_id_unique",
                    "changed": ["asset"],
                    "threshold": "order_id unique",
                    "asset": "silver.orders_clean",
                    "severity": "high",
                }),
                "Changed quality rule orders_id_unique: table to silver.orders_clean",
            ),
            (
                "quality.rule_update",
                json!({ "name": "orders_id_unique", "threshold": "order_id unique" }),
                "Changed quality rule orders_id_unique",
            ),
            (
                "quality.rule_delete",
                json!({ "name": "orders_id_unique" }),
                "Deleted quality rule orders_id_unique",
            ),
            (
                "policy.create",
                json!({ "name": "mask-email" }),
                "Added policy mask-email",
            ),
            (
                "policy.enforce",
                json!({ "name": "mask-email" }),
                "Enforced policy mask-email",
            ),
            (
                "policy.suspend",
                json!({ "name": "mask-email" }),
                "Stopped enforcing policy mask-email",
            ),
            (
                "policy.delete",
                json!({ "name": "mask-email", "enforced": true }),
                "Deleted policy mask-email (it was enforced)",
            ),
            (
                "policy.delete",
                json!({ "name": "mask-email", "enforced": false }),
                "Deleted policy mask-email",
            ),
        ] {
            assert_eq!(
                change_entry(&audit_event(action, args))["summary"],
                summary,
                "{action}"
            );
        }
    }

    /// What was taken back reads as plainly as what was added.
    #[test]
    fn change_entries_say_what_was_taken_back() {
        for (action, args, summary) in [
            (
                "catalog.declassify",
                json!({ "column": "email", "classification": "restricted" }),
                "Removed the classification of column email (restricted)",
            ),
            (
                "catalog.declassify",
                json!({ "column": null, "classification": "public" }),
                "Removed the classification of the asset (public)",
            ),
            (
                "catalog.sla_remove",
                json!({ "minutes": 2160 }),
                "Removed the freshness target",
            ),
        ] {
            let entry = change_entry(&audit_event(action, args));
            assert_eq!(entry["summary"], summary, "{action}");
        }
    }

    fn rule(asset: &str, column: Option<&str>, level: &str) -> ClassificationRule {
        ClassificationRule {
            id: format!("c-{asset}-{column:?}-{level}"),
            asset: asset.to_owned(),
            column: column.map(str::to_owned),
            classification: level.to_owned(),
            confidence: 1,
            review_status: "reviewed".to_owned(),
            masking_rule: None,
        }
    }

    /// No rule: the default level, and it says it is the default.
    #[test]
    fn an_unclassified_asset_is_internal_by_default() {
        let other = [rule("silver.other", None, "restricted")];
        let c = classify(&other, &tables(&["silver.orders"]));
        assert_eq!(c.level, "internal");
        assert!(!c.from_rule);
        assert!(c.columns.is_empty());
    }

    /// The newest rule wins, so a mistake is fixed by adding the right
    /// rule; and the asset is at least as restrictive as its columns.
    #[test]
    fn the_newest_rule_wins_and_a_column_raises_the_asset() {
        // Newest first, as the store lists them.
        let rules = [
            rule("silver.orders", None, "public"),
            rule("silver.orders", Some("email"), "confidential"),
            rule("SILVER.ORDERS", None, "restricted"),
            rule("silver.orders", Some("email"), "restricted"),
            rule("silver.orders", Some("card"), "restricted"),
        ];
        let c = classify(&rules, &tables(&["silver.orders"]));
        assert!(c.from_rule);
        assert_eq!(
            c.columns,
            vec![
                ("email".to_owned(), "confidential".to_owned()),
                ("card".to_owned(), "restricted".to_owned()),
            ]
        );
        // The asset rule says public; the card column makes it restricted.
        assert_eq!(c.level, "restricted");
        // The rules that won, the asset's own first: what a reader removes
        // to take a classification back. The older rules are not among them.
        assert_eq!(
            c.in_force,
            vec![
                (rules[0].id.clone(), None, "public".to_owned()),
                (
                    rules[1].id.clone(),
                    Some("email".to_owned()),
                    "confidential".to_owned()
                ),
                (
                    rules[4].id.clone(),
                    Some("card".to_owned()),
                    "restricted".to_owned()
                ),
            ]
        );

        let asset_only = classify(&rules[..1], &tables(&["silver.orders"]));
        assert_eq!(asset_only.level, "public");
    }

    fn check(name: &str, status: Option<&str>, severity: &str) -> Value {
        json!({ "name": name, "status": status, "severity": severity })
    }

    const DAY: i64 = 86_400;

    /// Nothing measured is "unknown" — never healthy by default.
    #[test]
    fn health_is_unknown_without_any_signal() {
        assert_eq!(health(None, None, &[]), ("unknown", Vec::new()));
        // An age with no target, and a rule nobody ran, are not signals.
        let unrun = [check("orders_unique", None, "high")];
        assert_eq!(health(Some(9 * DAY), None, &unrun).0, "unknown");
    }

    #[test]
    fn health_follows_freshness_and_quality() {
        let (status, reasons) = health(Some(5 * 3600 + 1800), Some(36 * 3600), &[]);
        assert_eq!(status, "healthy");
        assert_eq!(
            reasons,
            vec!["Fresh: written 5h 30m ago, within its 36h 00m target"]
        );

        let (status, reasons) = health(Some(8 * DAY + 3 * 3600), Some(36 * 3600), &[]);
        assert_eq!(status, "degraded");
        assert_eq!(
            reasons,
            vec!["Late: written 8d 3h ago, expected within 36h 00m"]
        );

        let checks = [
            check("rows", Some("passed"), "low"),
            check("region_filled", Some("failed"), "medium"),
        ];
        let (status, reasons) = health(None, None, &checks);
        assert_eq!(status, "degraded");
        assert_eq!(
            reasons,
            vec!["1 of 2 quality checks passed", "Failed: region_filled"]
        );
    }

    /// A failed check that matters makes the asset unhealthy, even when
    /// it is fresh.
    #[test]
    fn a_failed_high_severity_check_is_unhealthy() {
        let checks = [check("id_unique", Some("failed"), "high")];
        assert_eq!(health(Some(60), Some(DAY), &checks).0, "unhealthy");
        // "warning" — nothing to check — degrades, it does not fail.
        let warned = [check("id_unique", Some("warning"), "high")];
        assert_eq!(health(None, None, &warned).0, "degraded");
    }

    #[test]
    fn an_observed_verdict_keeps_its_result() {
        let observed = json!({
            "id": "q-7", "name": "row_count", "asset": "orders", "dimension": "completeness",
            "threshold": "row_count > 0 & does not drop >50%", "severity": "high",
            "lastStatus": "failed", "lastRunAt": "2026-09-30 01:00:00",
        });
        let check = observed_check(&observed);
        assert_eq!(check["id"], "orders/row_count");
        assert_eq!(check["status"], "failed");
        assert_eq!(check["lastRun"], "2026-09-30 01:00:00");
        assert_eq!(check["origin"], "observed");
    }
}
