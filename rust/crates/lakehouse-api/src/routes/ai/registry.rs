//! The AI Copilot tool registry: one static table of [`ToolSpec`]s that is
//! the single source of truth for a tool's JSON schema (fed to the LLM in
//! [`tool_schemas`]), its risk tier (consumed by the Ask-mode gate in
//! [`super::gate`]), and the console-route permission it stands in for.
//!
//! Before this module existed, the schema list and the dispatch `match` in
//! `ai.rs` were two hand-maintained lists that had to be kept in sync by
//! hand. [`tool_schemas`] is now derived from [`TOOLS`], and
//! [`super::tools::run_tool`] looks a name up here (via [`find`]) before
//! dispatching, so the two can no longer drift apart.

use serde_json::{Value, json};

/// How dangerous a tool call is, and therefore what gate it must pass
/// before it executes.
///
/// Only [`Risk::Read`] is enforced today — the Ask-mode gate in
/// [`super::gate`] refuses every non-`Read` tool in `mode: "ask"`.
/// `WriteLow`/`WriteHigh` are assigned now, ahead of the inline-confirm
/// and approvals-inbox flows that will actually branch on them (see the
/// copilot-operations-handover plan, Tier 0), so that later work is pure
/// gate logic rather than also having to invent a risk tier for every
/// tool from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Never mutates the lakehouse, a dashboard, or a pipeline. Available
    /// in every chat mode, including `ask`.
    Read,
    /// Mutates something, but is judged safe enough for an inline
    /// chat-side confirmation rather than a human-approval queue.
    /// **Not enforced yet** — today this is treated identically to
    /// [`Risk::WriteHigh`] by the Ask-mode gate (both are simply
    /// "non-`Read`"); the distinction becomes load-bearing once the
    /// inline-confirmation flow lands.
    WriteLow,
    /// Mutates something destructively or irreversibly enough that it
    /// should require routing through a human-approval queue rather than
    /// an inline confirmation. **Not enforced yet** — see [`Risk::WriteLow`].
    WriteHigh,
}

/// One entry in the AI Copilot's tool table: everything the chat loop
/// needs to advertise a tool to the LLM, gate its execution by chat mode,
/// and — in a later task — check the calling principal's permissions and
/// audit the call.
pub struct ToolSpec {
    /// The name the LLM calls this tool by; also the dispatch key
    /// [`super::tools::run_tool`] matches on.
    pub name: &'static str,
    /// Builds this tool's `OpenAI`-compatible JSON function schema. A
    /// plain `fn` pointer rather than a closure: none of these schemas
    /// capture anything, they are pure functions of no input.
    pub schema: fn() -> Value,
    /// This tool's risk tier — see [`Risk`].
    pub risk: Risk,
    /// The `resource:action` permission the equivalent console route
    /// requires, per `lakehouse-api::policy::POLICY_TABLE` — e.g.
    /// `create_chart`'s `"dashboard:write"` mirrors
    /// `POST /api/dashboard/specs`. An empty string means the closest
    /// equivalent route is `Policy::RequiresAuth` with no specific
    /// permission (authenticated only); there is nothing narrower to
    /// carry for that tool today.
    ///
    /// Enforced by [`super::gate::decide`] (T0.2 of the
    /// copilot-operations-handover plan): a principal whose merged
    /// `PermissionSet` lacks this permission has every call to this tool
    /// refused at dispatch, regardless of chat mode. Also used, as a pure
    /// optimisation (dispatch remains the real enforcement), to filter the
    /// tool list advertised to the model in [`tool_schemas_for`].
    pub permission: &'static str,
}

/// The `chart_kind_enum` JSON array shared by [`create_chart_schema`] and
/// [`update_chart_schema`] — factored out only because the two schemas
/// are otherwise identical lists; not shared with any other tool.
fn chart_kind_enum() -> Value {
    json!([
        "bar",
        "hbar",
        "line",
        "area",
        "stacked",
        "combo",
        "pie",
        "rose",
        "funnel",
        "treemap",
        "scatter",
        "bubble",
        "heatmap",
        "radar",
        "waterfall",
        "geomap",
        "kpi",
        "gauge",
        "table",
        "text"
    ])
}

fn run_sql_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_sql",
        "description": "Jalankan query SELECT ClickHouse (read-only) untuk menjawab pertanyaan data. Gunakan tabel serving.mart_* (Gold) untuk agregasi atau silver.`<nama>` untuk detail. Selalu SELECT saja, LIMIT wajar.",
        "parameters": { "type": "object",
            "properties": { "sql": { "type": "string", "description": "Query SELECT ClickHouse" } },
            "required": ["sql"] } } })
}

fn list_datasets_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_datasets",
        "description": "Daftar dataset di katalog lakehouse (opsional filter kata kunci / tier primer|sekunder).",
        "parameters": { "type": "object", "properties": {
            "search": { "type": "string" },
            "tier": { "type": "string", "enum": ["primer", "sekunder"] } } } } })
}

fn describe_dataset_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_dataset",
        "description": "Metadata + skema kolom + jumlah baris satu dataset (by slug).",
        "parameters": { "type": "object", "properties": { "slug": { "type": "string" } },
            "required": ["slug"] } } })
}

fn get_lineage_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_lineage",
        "description": "Silsilah sebuah dataset: source → Bronze → Silver + mapping kolom (by slug).",
        "parameters": { "type": "object", "properties": { "slug": { "type": "string" } },
            "required": ["slug"] } } })
}

fn get_quality_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_quality",
        "description": "Ringkasan kualitas data lakehouse (jumlah cek pass/warn/fail + contoh masalah).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn trigger_lakehouse_build_schema() -> Value {
    json!({ "type": "function", "function": { "name": "trigger_lakehouse_build",
        "description": "BANGUN ULANG lakehouse: tarik data SDI+berkas ke Bronze, generate Silver bertipe, build mart Gold. Menjalankan job Dagster 'refresh_lakehouse'. Pakai saat user minta membangun/menyegarkan data Bronze/Silver/Gold.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_build_status_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_build_status",
        "description": "Status run pipeline lakehouse terakhir (Dagster).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn describe_mart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "describe_mart",
        "description": "Lihat mart Gold (serving.*) yang bisa divisualisasikan. Tanpa argumen: daftar semua mart. Dengan `mart`: kolom mart itu, terbagi dimensi (kategori/waktu) & measure (angka). PANGGIL INI DULU sebelum create_chart agar memilih kolom yang benar-benar ada.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "nama mart, mis. mart_wisman" } } } } })
}

fn create_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_chart",
        "description": "Buat kartu chart baru di dashboard (/dashboards) dari mart Gold. Server menyusun SQL-nya sendiri dari kolom yang kamu pilih (agregasi per dimensi) — kamu TIDAK menulis SQL. Panggil describe_mart dulu untuk tahu kolom valid. Chart langsung tersimpan & tampil.",
        "parameters": { "type": "object", "properties": {
            "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "text": { "type": "string" }, "caption": { "type": "string" },
            "target": { "type": "number" }, "dimension": { "type": "string" },
            "measures": { "type": "array", "items": { "type": "string" } },
            "breakdown": { "type": "string" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "board": { "type": "string" } },
            "required": ["title", "kind"] } } })
}

fn update_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "update_chart",
        "description": "Ubah chart tersimpan (by id) — mempertahankan id, mengganti definisinya. Kirim SEMUA field seperti create_chart dengan nilai baru. Pakai list_charts untuk tahu id.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" }, "title": { "type": "string" }, "subtitle": { "type": "string" },
            "mart": { "type": "string" }, "kind": { "type": "string", "enum": chart_kind_enum() },
            "dimension": { "type": "string" },
            "measures": { "type": "array", "items": { "type": "string" } },
            "breakdown": { "type": "string" }, "caption": { "type": "string" },
            "target": { "type": "number" },
            "aggregate": { "type": "string", "enum": ["sum", "avg", "max", "min", "count"] },
            "limit": { "type": "number" }, "span": { "type": "number", "enum": [1, 2] },
            "board": { "type": "string" } },
            "required": ["id", "title", "kind"] } } })
}

fn create_board_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_board",
        "description": "Buat board (dashboard bernama) baru. Kembalikan id-nya untuk dipakai di create_chart.",
        "parameters": { "type": "object", "properties": { "name": { "type": "string" } },
            "required": ["name"] } } })
}

fn list_boards_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_boards",
        "description": "Daftar board (dashboard bernama) yang ada.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn suggest_dashboard_schema() -> Value {
    json!({ "type": "function", "function": { "name": "suggest_dashboard",
        "description": "Ambil katalog SEMUA mart Gold beserta dimensi & measure-nya sekaligus — untuk MENGUSULKAN set kartu dashboard.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_charts_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_charts",
        "description": "Daftar kartu chart tersimpan di dashboard (yang dibuat lewat chat/UI).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn delete_chart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_chart",
        "description": "Hapus satu kartu chart tersimpan dari dashboard (by id). Spec bawaan tak bisa dihapus.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

/// The AI Copilot's full tool table, in the exact order the LLM sees them
/// in — [`tool_schemas`] preserves this order verbatim, and it is
/// load-bearing for the committed snapshot in
/// `tests/fixtures/tool_schemas.json`.
///
/// Risk assignment for this task (T0.1 of the copilot-operations-handover
/// plan) is deliberately coarse: every tool that was in the old
/// `WRITE_TOOLS` array is non-`Read` (`WriteLow`, except `delete_chart`
/// which is `WriteHigh`); everything else is `Read`. Refining
/// `WriteLow` vs `WriteHigh` for tools added in later tasks, and actually
/// branching gate behaviour on the distinction, happens in T0.4/T0.5.
pub static TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "run_sql",
        schema: run_sql_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    ToolSpec {
        name: "list_datasets",
        schema: list_datasets_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "describe_dataset",
        schema: describe_dataset_schema,
        risk: Risk::Read,
        permission: "catalog:read",
    },
    ToolSpec {
        name: "get_lineage",
        schema: get_lineage_schema,
        risk: Risk::Read,
        permission: "lineage:read",
    },
    ToolSpec {
        name: "get_quality",
        schema: get_quality_schema,
        risk: Risk::Read,
        // No route with a narrower permission than `RequiresAuth` covers
        // quality directly (`GET /api/governance/{kind}` is
        // `RequiresAuth`) — empty means "authenticated only", see the
        // `permission` field doc comment.
        permission: "",
    },
    ToolSpec {
        name: "trigger_lakehouse_build",
        schema: trigger_lakehouse_build_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "get_build_status",
        schema: get_build_status_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "describe_mart",
        schema: describe_mart_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "create_chart",
        schema: create_chart_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "update_chart",
        schema: update_chart_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "create_board",
        schema: create_board_schema,
        risk: Risk::WriteLow,
        permission: "dashboard:write",
    },
    ToolSpec {
        name: "list_boards",
        schema: list_boards_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "suggest_dashboard",
        schema: suggest_dashboard_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "list_charts",
        schema: list_charts_schema,
        risk: Risk::Read,
        permission: "dashboard:read",
    },
    ToolSpec {
        name: "delete_chart",
        schema: delete_chart_schema,
        risk: Risk::WriteHigh,
        permission: "dashboard:write",
    },
];

/// The `OpenAI`-compatible `tools` schema array, matching
/// `TOOL_SCHEMAS = Object.values(TOOLS).map((t) => t.schema)` in
/// `ai-tools.ts` — now derived from [`TOOLS`] rather than declared
/// separately. Byte-identical to the pre-refactor output; see
/// `tests/fixtures/tool_schemas.json`.
#[must_use]
pub fn tool_schemas() -> Vec<Value> {
    TOOLS.iter().map(|t| (t.schema)()).collect()
}

/// Looks up a tool by name. `None` means the name is not a registered
/// tool at all (the "tool tak dikenal" case in
/// [`super::tools::run_tool`]), as distinct from a registered tool this
/// mode/principal is not allowed to call.
#[must_use]
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The tool schemas a principal with `perms` should be OFFERED, given
/// `perms` — an optimisation only, not enforcement: a tool this filters
/// out is still refused by [`super::gate::decide`] at dispatch if it
/// somehow reaches `run_tool` anyway (a hallucinated `tool_calls` entry, or
/// `MiniMax` XML extracted from free text). Unlike [`tool_schemas`], which
/// is pinned byte-identical to `tests/fixtures/tool_schemas.json` and must
/// never change, this is a fresh accessor so that snapshot stays untouched.
///
/// `perms: None` matches [`super::gate::decide`]'s absent-principal
/// contract: treated as "authenticated, no grants" — every tool with a
/// non-empty `permission` is filtered out, every tool with an empty one
/// (`""`, "authenticated only") is kept.
#[must_use]
pub fn tool_schemas_for(perms: Option<&lakehouse_auth::PermissionSet>) -> Vec<Value> {
    TOOLS
        .iter()
        .zip(tool_schemas())
        .filter(|(t, _)| t.permission.is_empty() || perms.is_some_and(|p| p.has(t.permission)))
        .map(|(_, schema)| schema)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn tool_schemas_has_fifteen_entries() {
        assert_eq!(tool_schemas().len(), 15);
    }

    /// Characterization snapshot (T0.1): `tool_schemas()`, now derived
    /// from [`TOOLS`], must still reproduce the exact JSON captured from
    /// the pre-refactor `ai.rs` byte-for-byte.
    #[test]
    fn tool_schemas_snapshot_is_byte_identical() {
        let expected = include_str!("../../../tests/fixtures/tool_schemas.json");
        let actual = serde_json::to_string_pretty(&tool_schemas()).unwrap();
        assert_eq!(
            actual, expected,
            "tool_schemas() output drifted from the committed snapshot \
             (rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json) — \
             regenerate the fixture ONLY for an intentional, reviewed \
             schema change, never to make this test pass during a refactor"
        );
    }

    #[test]
    fn tools_len_matches_schema_len() {
        assert_eq!(TOOLS.len(), tool_schemas().len());
    }

    /// The five tools that made up the old `WRITE_TOOLS` array must all
    /// still be non-`Read`; two representative read tools must not be.
    #[test]
    fn former_write_tools_are_non_read() {
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "delete_chart",
            "create_board",
        ] {
            let spec = find(name).expect("registered tool");
            assert_ne!(spec.risk, Risk::Read, "{name} must be a non-Read risk");
        }
        assert_eq!(find("run_sql").expect("registered tool").risk, Risk::Read);
        assert_eq!(
            find("list_datasets").expect("registered tool").risk,
            Risk::Read
        );
    }

    /// T0.1's assigned risk split: `delete_chart` is `WriteHigh`, the
    /// other four former `WRITE_TOOLS` are `WriteLow`.
    #[test]
    fn delete_chart_is_write_high_others_are_write_low() {
        assert_eq!(
            find("delete_chart").expect("registered tool").risk,
            Risk::WriteHigh
        );
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "create_board",
        ] {
            assert_eq!(
                find(name).expect("registered tool").risk,
                Risk::WriteLow,
                "{name} must be WriteLow"
            );
        }
    }

    #[test]
    fn find_returns_none_for_unknown_name() {
        assert!(find("not_a_real_tool").is_none());
    }

    /// `tool_schemas_for` is an optimisation over the same [`TOOLS`] table:
    /// a Platform Admin (`*:*`) is offered every tool.
    #[test]
    fn tool_schemas_for_admin_offers_every_tool() {
        let perms = lakehouse_auth::PermissionSet::parse("*:*");
        assert_eq!(tool_schemas_for(Some(&perms)).len(), TOOLS.len());
    }

    /// An Analyst (`query:read, catalog:read, lineage:read`) is offered
    /// only the tools with an empty `permission` or one of those three —
    /// no `dashboard:*` tool.
    #[test]
    fn tool_schemas_for_analyst_excludes_dashboard_tools() {
        let perms = lakehouse_auth::PermissionSet::parse("query:read, catalog:read, lineage:read");
        let offered = tool_schemas_for(Some(&perms));
        let offered_names: Vec<&str> = offered
            .iter()
            .map(|v| v["function"]["name"].as_str().expect("name"))
            .collect();
        assert!(offered_names.contains(&"run_sql"));
        assert!(offered_names.contains(&"list_datasets"));
        assert!(offered_names.contains(&"get_lineage"));
        assert!(!offered_names.contains(&"describe_mart"));
        assert!(!offered_names.contains(&"create_chart"));
        assert!(!offered_names.contains(&"list_boards"));
    }

    /// With no principal at all, only empty-`permission` tools are offered.
    #[test]
    fn tool_schemas_for_none_offers_only_empty_permission_tools() {
        let offered_names: Vec<String> = tool_schemas_for(None)
            .iter()
            .map(|v| v["function"]["name"].as_str().expect("name").to_owned())
            .collect();
        // Only `get_quality` has an empty `permission` today.
        assert_eq!(offered_names, vec!["get_quality".to_owned()]);
    }
}
