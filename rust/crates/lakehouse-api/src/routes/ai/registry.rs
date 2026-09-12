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
    /// Enforced by [`super::gate::decide`] (T0.4): a call without
    /// `"confirmed": true` in its args gets a `needs_confirmation` result
    /// instead of executing.
    WriteLow,
    /// Mutates something destructively or irreversibly enough that it
    /// should require routing through a human-approval queue rather than
    /// an inline confirmation. The real approvals-inbox flow is T0.5 of
    /// the copilot-operations-handover plan and does not exist yet — until
    /// then, [`super::gate::decide`] refuses every `WriteHigh` call with a
    /// clear "not implemented" reason rather than executing it or letting
    /// it through the `WriteLow` confirmation path.
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

// ── T1.1 Alerts ──────────────────────────────────────────────────────────

/// The five comparison operators an alert rule may use, mirroring
/// `lakehouse_alerts::AlertOp` — shared between [`create_alert_rule_schema`]
/// and [`update_alert_rule_schema`] so the two lists cannot drift apart.
fn alert_op_enum() -> Value {
    json!([">", ">=", "<", "<=", "=="])
}

/// The five aggregate functions an alert rule may watch, mirroring
/// `lakehouse_alerts::AGGS`.
fn alert_agg_enum() -> Value {
    json!(["sum", "avg", "max", "min", "count"])
}

fn list_alert_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_alert_rules",
        "description": "Daftar semua aturan alert (peringatan ambang batas) dan digest (ringkasan berkala) yang terpasang, beserta status aktif/nonaktifnya.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn create_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_alert_rule",
        "description": "Buat aturan alert atau digest baru. Untuk type=alert: isi mart, measure, agg (agregat), op (operator pembanding), threshold — rule ini akan memantau nilai agregat itu. Untuk type=digest: isi board (id dashboard yang diringkas). Pengiriman lewat channel webhook (target=URL) atau email (target=alamat).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "type": { "type": "string", "enum": ["alert", "digest"] },
            "mart": { "type": "string", "description": "nama mart Gold, untuk type=alert" },
            "measure": { "type": "string", "description": "kolom ukuran yang dipantau, untuk type=alert" },
            "agg": { "type": "string", "enum": alert_agg_enum() },
            "op": { "type": "string", "enum": alert_op_enum() },
            "threshold": { "type": "number" },
            "board": { "type": "string", "description": "id board dashboard, untuk type=digest" },
            "channel": { "type": "string", "enum": ["webhook", "email"] },
            "target": { "type": "string", "description": "URL webhook atau alamat email tujuan" },
            "enabled": { "type": "boolean" } },
            "required": ["name", "type", "channel", "target"] } } })
}

fn update_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "update_alert_rule",
        "description": "Ubah aturan alert/digest tersimpan (by id) — kirim semua field seperti create_alert_rule dengan nilai baru. Pakai list_alert_rules untuk tahu id.",
        "parameters": { "type": "object", "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "type": { "type": "string", "enum": ["alert", "digest"] },
            "mart": { "type": "string" },
            "measure": { "type": "string" },
            "agg": { "type": "string", "enum": alert_agg_enum() },
            "op": { "type": "string", "enum": alert_op_enum() },
            "threshold": { "type": "number" },
            "board": { "type": "string" },
            "channel": { "type": "string", "enum": ["webhook", "email"] },
            "target": { "type": "string" },
            "enabled": { "type": "boolean" } },
            "required": ["id"] } } })
}

fn delete_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_alert_rule",
        "description": "Hapus aturan alert/digest secara permanen (by id). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn run_alert_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_alert_rule",
        "description": "Jalankan evaluasi satu aturan alert/digest sekarang (by id) — bila kondisinya terpenuhi, webhook/email BENERAN terkirim ke target.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T1.2 Connectors ──────────────────────────────────────────────────────

fn connector_direction_enum() -> Value {
    json!(["source", "sink", "bidirectional"])
}

fn list_connectors_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_connectors",
        "description": "Daftar semua connector (sumber/tujuan data) yang terdaftar beserta status kesehatannya.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn create_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "create_connector",
        "description": "Daftarkan connector baru. PENTING: secretRef HARUS berupa referensi kredensial (mis. \"env:NAMA_SECRET\" atau \"vault://path\"), JANGAN PERNAH kredensial mentah (password/token asli) — permintaan akan ditolak jika terlihat seperti kredensial asli.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "type": { "type": "string", "description": "mis. PostgreSQL, Object storage, Kafka" },
            "direction": { "type": "string", "enum": connector_direction_enum() },
            "host": { "type": "string", "description": "target koneksi (host:port atau endpoint)" },
            "secretRef": { "type": "string", "description": "REFERENSI kredensial, mis. env:DB_PASSWORD atau vault://secret/data/x — bukan kredensial asli" },
            "secretRefSecondary": { "type": "string", "description": "referensi kredensial kedua (mis. secret key S3), opsional" },
            "environment": { "type": "string" },
            "tenant": { "type": "string" },
            "residency": { "type": "string" },
            "capabilities": { "type": "array", "items": { "type": "string" } },
            "owner": { "type": "string" } },
            "required": ["name", "type", "direction", "host", "secretRef", "environment", "tenant"] } } })
}

fn test_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "test_connector",
        "description": "Tes koneksi nyata ke connector (by id). Hanya PostgreSQL dan Object storage (S3-compatible) yang benar-benar bisa dites di build ini — tipe lain akan mengembalikan supported:false, bukan hasil palsu.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn delete_connector_schema() -> Value {
    json!({ "type": "function", "function": { "name": "delete_connector",
        "description": "Hapus registrasi connector secara permanen (by id). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T1.3 Pipelines (additions) ───────────────────────────────────────────

fn list_pipelines_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_pipelines",
        "description": "Daftar semua pipeline (job Dagster + pipeline yang dibuat via chat/UI) beserta status & jadwalnya.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_pipeline_runs_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_pipeline_runs",
        "description": "Daftar run terbaru (maks 30) dari satu pipeline (by id).",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn trigger_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "trigger_pipeline",
        "description": "Jalankan satu pipeline tertentu sekarang (by id) — berbeda dari trigger_lakehouse_build yang selalu menjalankan pipeline utama Bronze→Silver→Gold.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn retry_pipeline_run_schema() -> Value {
    json!({ "type": "function", "function": { "name": "retry_pipeline_run",
        "description": "Jalankan ulang satu run pipeline yang sudah selesai dari awal (by runId).",
        "parameters": { "type": "object", "properties": { "runId": { "type": "string" } },
            "required": ["runId"] } } })
}

fn pause_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "pause_pipeline",
        "description": "Jeda jadwal terjadwal sebuah pipeline (by id). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn resume_pipeline_schema() -> Value {
    json!({ "type": "function", "function": { "name": "resume_pipeline",
        "description": "Aktifkan kembali jadwal terjadwal sebuah pipeline yang sebelumnya dijeda (by id).",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

fn cancel_pipeline_run_schema() -> Value {
    json!({ "type": "function", "function": { "name": "cancel_pipeline_run",
        "description": "Hentikan paksa satu run pipeline yang sedang berjalan (by runId). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": { "runId": { "type": "string" } },
            "required": ["runId"] } } })
}

// ── T1.4 Saved queries ───────────────────────────────────────────────────

fn save_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "save_query",
        "description": "Simpan query SQL sebagai saved query bernama, untuk dijalankan ulang lewat run_saved_query kapan saja.",
        "parameters": { "type": "object", "properties": {
            "title": { "type": "string" },
            "sql": { "type": "string", "description": "Query SELECT ClickHouse" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "owner": { "type": "string" } },
            "required": ["title", "sql"] } } })
}

fn list_saved_queries_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_saved_queries",
        "description": "Daftar semua saved query yang tersimpan.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn run_saved_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_saved_query",
        "description": "Jalankan satu saved query tersimpan (by id) dan kembalikan hasilnya. Hanya query baca (SELECT/WITH/SHOW/DESCRIBE/EXPLAIN) yang diizinkan, sama seperti Query Studio.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T2.1 Governance reads ────────────────────────────────────────────────

fn get_audit_history_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_audit_history",
        "description": "Riwayat audit gabungan: run pipeline Dagster + aksi copilot/console (audit_event) — siapa/apa melakukan apa, kapan, dan hasilnya.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_classification_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_classification_rules",
        "description": "Daftar klasifikasi data per aset/kolom (public/internal/confidential/restricted) — hasil observasi ClickHouse digabung dengan aturan yang sudah ditulis (authored).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn list_quality_rules_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_quality_rules",
        "description": "Daftar aturan & hasil kualitas data (completeness/uniqueness/dll) — hasil observasi ClickHouse digabung dengan aturan yang sudah ditulis (authored).",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_cdc_health_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_cdc_health",
        "description": "Kesehatan replication slot CDC (lag, WAL retained, status) per connector — untuk mendeteksi slot yang macet/tertinggal sebelum memenuhi disk database sumber.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn get_maintenance_metrics_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_maintenance_metrics",
        "description": "Riwayat run maintenance Bronze (remove_orphan_files): file data/manifest yatim yang dihapus, per tabel. Catatan: expire_snapshots DITOLAK ClickHouse untuk tabel Iceberg berkatalog (Code: 48), jadi verb itu hanya tercatat sebagai skip, bukan hasil. Baca-saja — pakai run_bronze_maintenance untuk benar-benar menjalankan maintenance.",
        "parameters": { "type": "object", "properties": {} } } })
}

// ── T2.2 Maintenance ─────────────────────────────────────────────────────
//
// See the copilot-operations-handover plan, section 3.7 correction C2:
// there is no dry-run-only trigger. `bronze_maintenance_job`
// (`dagster/dispar_orchestrate/maintenance.py:297-299`) always runs a dry
// pass AND THEN the applied pass in the same job, and
// `DgClient::launch_run` takes a job name only, with no run-config
// override to split the two. So there is exactly ONE maintenance tool,
// `WriteHigh` (it genuinely deletes orphan Iceberg data/manifest files),
// never advertised as a "dry run".

fn run_bronze_maintenance_schema() -> Value {
    json!({ "type": "function", "function": { "name": "run_bronze_maintenance",
        "description": "Jalankan maintenance Bronze SEKARANG (job Dagster bronze_maintenance_job). Tindakan ini MENERAPKAN perubahan — menghapus file data/manifest Iceberg yatim (orphan) yang sudah tidak dipakai snapshot mana pun. Ini BUKAN dry run: tidak ada mode dry-run terpisah di build ini (satu job selalu menjalankan dry-run lalu applied run sekaligus). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": {} } } })
}

// ── T2.3 Workloads ───────────────────────────────────────────────────────

fn list_workloads_schema() -> Value {
    json!({ "type": "function", "function": { "name": "list_workloads",
        "description": "Daftar query ClickHouse yang sedang berjalan sekarang (workload), beserta id (\"w-<n>\") yang dipakai kill_query.",
        "parameters": { "type": "object", "properties": {} } } })
}

fn kill_query_schema() -> Value {
    json!({ "type": "function", "function": { "name": "kill_query",
        "description": "Hentikan paksa satu query ClickHouse yang sedang berjalan (KILL QUERY sungguhan, by id dari list_workloads, mis. \"w-0\"). Tindakan ini butuh persetujuan manusia sebelum dijalankan.",
        "parameters": { "type": "object", "properties": { "id": { "type": "string" } },
            "required": ["id"] } } })
}

// ── T2.4 Gold export ─────────────────────────────────────────────────────

fn export_gold_mart_schema() -> Value {
    json!({ "type": "function", "function": { "name": "export_gold_mart",
        "description": "Ekspor satu mart Gold (serving.<mart>) ke tabel Iceberg Gold lewat Lakekeeper. PENTING: ekspor ini APPEND-ONLY — menjalankan ulang akan MENAMBAH baris baru (dengan timestamp _exported_at baru), bukan menggantikan yang lama. Konsumen tabel Iceberg-nya harus memfilter _exported_at sendiri; tool ini tidak idempoten.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "nama mart Gold, mis. mart_wisman" } },
            "required": ["mart"] } } })
}

fn get_gold_export_schema() -> Value {
    json!({ "type": "function", "function": { "name": "get_gold_export",
        "description": "Baca balik tabel Iceberg Gold (by mart) lewat Lakekeeper: jumlah baris & format version saat ini — bukti independen dari klaim export_gold_mart, tidak menyentuh ClickHouse sama sekali.",
        "parameters": { "type": "object", "properties": {
            "mart": { "type": "string", "description": "nama mart Gold, mis. mart_wisman" } },
            "required": ["mart"] } } })
}

// ── T2.5 Governance draft tools ──────────────────────────────────────────
//
// All three create a record in the least-active state the underlying
// store can express, never anything a human would recognise as "already
// active": `draft_policy` always forces `activate: false` (status
// `"draft"` — `policy.status_check` also allows `"ready"`, which this
// tool never produces). `quality_rule`/`classification_rule` have NO
// activation concept in the schema at all (`0003_governance.sql`: no
// `enabled`/`active` column, no `POST .../activate` route anywhere in
// `routes::governance`) — every row `create_quality_rule`/
// `create_classification_rule` inserts starts `last_status = 'warning'` /
// `review_status = 'needs-review'` (an authored-but-unevaluated fact, per
// `lakehouse_store::governance`'s module doc comment) and there is no API
// path in this codebase that ever promotes one further. So these two
// tools cannot accidentally create something "more active" than a human
// clicking the same console form would — draft-only is already the only
// state reachable.

fn draft_policy_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_policy",
        "description": "Buat DRAFT kebijakan (policy) governance baru — SELALU berstatus draft, tidak pernah langsung aktif. Mengaktifkan kebijakan tetap aksi manusia di console (Governance → Policies).",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "kind": { "type": "string", "description": "mis. \"Row filter\", \"Agent autonomy\"" },
            "subjects": { "type": "string", "description": "siapa/apa yang dikenai kebijakan" },
            "resources": { "type": "string", "description": "apa yang dikenai kebijakan" },
            "effect": { "type": "string", "description": "mis. \"Permit with obligation\", \"Require approval\"" },
            "conditions": { "type": "string" },
            "owner": { "type": "string" } },
            "required": ["name", "kind", "subjects", "resources", "effect"] } } })
}

fn draft_classification_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_classification_rule",
        "description": "Tulis aturan klasifikasi/masking baru untuk sebuah aset (opsional kolom tertentu). Selalu masuk sebagai \"needs-review\" — tidak ada status aktif terpisah di build ini; peninjauan tetap aksi manusia di console.",
        "parameters": { "type": "object", "properties": {
            "asset": { "type": "string" },
            "column": { "type": "string" },
            "classification": { "type": "string", "enum": ["public", "internal", "confidential", "restricted"] },
            "maskingRule": { "type": "string" } },
            "required": ["asset", "classification"] } } })
}

fn draft_quality_rule_schema() -> Value {
    json!({ "type": "function", "function": { "name": "draft_quality_rule",
        "description": "Tulis aturan kualitas data baru untuk sebuah aset. Rule yang baru ditulis SELALU berstatus \"warning\" (belum pernah dievaluasi) — tidak ada status aktif terpisah di build ini.",
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string" },
            "asset": { "type": "string" },
            "dimension": { "type": "string", "description": "mis. completeness, uniqueness, accuracy" },
            "threshold": { "type": "string", "description": "mis. \">= 95%\"" },
            "severity": { "type": "string", "enum": ["critical", "high", "medium", "low", "info"] } },
            "required": ["name", "asset", "dimension", "threshold", "severity"] } } })
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
    // ── T1.1 Alerts (Tier 1 of the copilot-operations-handover plan) ────
    // Permission strings verified against `policy.rs::POLICY_TABLE`
    // (plan section 3.7 C1): `GET /api/alerts` is `RequiresAuth` (no
    // narrower permission — empty string, same convention as
    // `get_quality`), `POST`/`PUT`/`DELETE /api/alerts` and the live
    // `/api/alerts/run` evaluation all require `alert:write`.
    ToolSpec {
        name: "list_alert_rules",
        schema: list_alert_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "create_alert_rule",
        schema: create_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    ToolSpec {
        name: "update_alert_rule",
        schema: update_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    ToolSpec {
        name: "delete_alert_rule",
        schema: delete_alert_rule_schema,
        risk: Risk::WriteHigh,
        permission: "alert:write",
    },
    ToolSpec {
        name: "run_alert_rule",
        schema: run_alert_rule_schema,
        risk: Risk::WriteLow,
        permission: "alert:write",
    },
    // ── T1.2 Connectors ───────────────────────────────────────────────
    // Every `/api/connectors*` route — including `GET` and `/test` —
    // requires `connector:manage` (policy.rs:266-275); there is no
    // narrower read permission to carry here.
    ToolSpec {
        name: "list_connectors",
        schema: list_connectors_schema,
        risk: Risk::Read,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "create_connector",
        schema: create_connector_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "test_connector",
        schema: test_connector_schema,
        risk: Risk::WriteLow,
        permission: "connector:manage",
    },
    ToolSpec {
        name: "delete_connector",
        schema: delete_connector_schema,
        risk: Risk::WriteHigh,
        permission: "connector:manage",
    },
    // ── T1.3 Pipelines (additions) ────────────────────────────────────
    // `pipeline:read` for the two list tools, `pipeline:write` for every
    // mutation (policy.rs:210-218) — same split `trigger_lakehouse_build`/
    // `get_build_status` already use above.
    ToolSpec {
        name: "list_pipelines",
        schema: list_pipelines_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "list_pipeline_runs",
        schema: list_pipeline_runs_schema,
        risk: Risk::Read,
        permission: "pipeline:read",
    },
    ToolSpec {
        name: "trigger_pipeline",
        schema: trigger_pipeline_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "retry_pipeline_run",
        schema: retry_pipeline_run_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "pause_pipeline",
        schema: pause_pipeline_schema,
        risk: Risk::WriteHigh,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "resume_pipeline",
        schema: resume_pipeline_schema,
        risk: Risk::WriteLow,
        permission: "pipeline:write",
    },
    ToolSpec {
        name: "cancel_pipeline_run",
        schema: cancel_pipeline_run_schema,
        risk: Risk::WriteHigh,
        permission: "pipeline:write",
    },
    // ── T1.4 Saved queries ────────────────────────────────────────────
    // `POST /api/query/run`, `GET /api/query/saved` and `/history` all
    // require the SAME `query:read` (policy.rs:202-205) — there is no
    // separate write permission for saved queries in `POLICY_TABLE`.
    ToolSpec {
        name: "save_query",
        schema: save_query_schema,
        risk: Risk::WriteLow,
        permission: "query:read",
    },
    ToolSpec {
        name: "list_saved_queries",
        schema: list_saved_queries_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    ToolSpec {
        name: "run_saved_query",
        schema: run_saved_query_schema,
        risk: Risk::Read,
        permission: "query:read",
    },
    // ── T2.1 Governance reads ─────────────────────────────────────────
    // `GET /api/governance/{kind}` is `RequiresAuth` for every kind
    // (policy.rs:164, C1) — no narrower permission to carry.
    ToolSpec {
        name: "get_audit_history",
        schema: get_audit_history_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "list_classification_rules",
        schema: list_classification_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "list_quality_rules",
        schema: list_quality_rules_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "get_cdc_health",
        schema: get_cdc_health_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "get_maintenance_metrics",
        schema: get_maintenance_metrics_schema,
        risk: Risk::Read,
        permission: "",
    },
    // ── T2.2 Maintenance (C2: exactly one tool, see its schema doc) ────
    ToolSpec {
        name: "run_bronze_maintenance",
        schema: run_bronze_maintenance_schema,
        risk: Risk::WriteHigh,
        permission: "",
    },
    // ── T2.3 Workloads ──────────────────────────────────────────────
    // `GET /api/ops/workloads` is `RequiresAuth`; cancelling one requires
    // `workload:cancel` (policy.rs:156, C1).
    ToolSpec {
        name: "list_workloads",
        schema: list_workloads_schema,
        risk: Risk::Read,
        permission: "",
    },
    ToolSpec {
        name: "kill_query",
        schema: kill_query_schema,
        risk: Risk::WriteHigh,
        permission: "workload:cancel",
    },
    // ── T2.4 Gold export ────────────────────────────────────────────
    // `GET`/`POST /api/gold/export/{mart}` are both `RequiresAuth`
    // (policy.rs:197-198, C1) — the route's OWN `x-run-token`/service-
    // identity guard is a separate, stricter door for the Dagster
    // scheduler (ADR 0011), not something the copilot goes through; see
    // `tools::gold`'s module doc comment for why bypassing it here is the
    // same precedent as `run_alert_rule` bypassing `check_run_token`.
    ToolSpec {
        name: "export_gold_mart",
        schema: export_gold_mart_schema,
        risk: Risk::WriteLow,
        permission: "",
    },
    ToolSpec {
        name: "get_gold_export",
        schema: get_gold_export_schema,
        risk: Risk::Read,
        permission: "",
    },
    // ── T2.5 Governance draft tools ─────────────────────────────────
    // `POST /api/governance/policies` requires `policy:write`
    // (policy.rs:163); `POST /api/governance/{kind}` (quality,
    // classification) is `RequiresAuth` only (policy.rs:165, C1).
    ToolSpec {
        name: "draft_policy",
        schema: draft_policy_schema,
        risk: Risk::WriteLow,
        permission: "policy:write",
    },
    ToolSpec {
        name: "draft_classification_rule",
        schema: draft_classification_rule_schema,
        risk: Risk::WriteLow,
        permission: "",
    },
    ToolSpec {
        name: "draft_quality_rule",
        schema: draft_quality_rule_schema,
        risk: Risk::WriteLow,
        permission: "",
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
    fn tool_schemas_has_forty_seven_entries() {
        // 15 pre-T1 tools + 19 Tier 1 operations tools (5 alerts + 4
        // connectors + 7 pipelines + 3 saved queries) + 13 Tier 2 tools
        // (5 governance reads + 1 maintenance + 2 workloads + 2 gold
        // export + 3 governance drafts).
        assert_eq!(tool_schemas().len(), 47);
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
        // Every tool whose `permission` is `""` ("authenticated only"), in
        // `TOOLS` order: `get_quality`/`list_alert_rules` (T1.1) plus the
        // Tier 2 tools that carry no narrower permission than
        // `RequiresAuth` (C1) — every T2.1 governance read,
        // `run_bronze_maintenance`, `list_workloads`, both gold export
        // tools, and the two rule-level draft tools (`draft_policy` needs
        // `policy:write`, so it is NOT in this list).
        assert_eq!(
            offered_names,
            vec![
                "get_quality".to_owned(),
                "list_alert_rules".to_owned(),
                "get_audit_history".to_owned(),
                "list_classification_rules".to_owned(),
                "list_quality_rules".to_owned(),
                "get_cdc_health".to_owned(),
                "get_maintenance_metrics".to_owned(),
                "run_bronze_maintenance".to_owned(),
                "list_workloads".to_owned(),
                "export_gold_mart".to_owned(),
                "get_gold_export".to_owned(),
                "draft_classification_rule".to_owned(),
                "draft_quality_rule".to_owned(),
            ]
        );
    }

    /// T1.1-T1.4: every new operations tool has exactly the risk and
    /// permission specified in the copilot-operations-handover plan's
    /// section 3.7 C1 table (verified against `policy.rs::POLICY_TABLE`).
    #[test]
    fn tier1_tools_have_the_documented_risk_and_permission() {
        let expected: &[(&str, Risk, &str)] = &[
            ("list_alert_rules", Risk::Read, ""),
            ("create_alert_rule", Risk::WriteLow, "alert:write"),
            ("update_alert_rule", Risk::WriteLow, "alert:write"),
            ("delete_alert_rule", Risk::WriteHigh, "alert:write"),
            ("run_alert_rule", Risk::WriteLow, "alert:write"),
            ("list_connectors", Risk::Read, "connector:manage"),
            ("create_connector", Risk::WriteLow, "connector:manage"),
            ("test_connector", Risk::WriteLow, "connector:manage"),
            ("delete_connector", Risk::WriteHigh, "connector:manage"),
            ("list_pipelines", Risk::Read, "pipeline:read"),
            ("list_pipeline_runs", Risk::Read, "pipeline:read"),
            ("trigger_pipeline", Risk::WriteLow, "pipeline:write"),
            ("retry_pipeline_run", Risk::WriteLow, "pipeline:write"),
            ("pause_pipeline", Risk::WriteHigh, "pipeline:write"),
            ("resume_pipeline", Risk::WriteLow, "pipeline:write"),
            ("cancel_pipeline_run", Risk::WriteHigh, "pipeline:write"),
            ("save_query", Risk::WriteLow, "query:read"),
            ("list_saved_queries", Risk::Read, "query:read"),
            ("run_saved_query", Risk::Read, "query:read"),
        ];
        for (name, risk, permission) in expected {
            let spec = find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(spec.risk, *risk, "{name} risk");
            assert_eq!(spec.permission, *permission, "{name} permission");
        }
    }

    /// T2.1-T2.5: every Tier 2 tool has exactly the risk and permission
    /// specified in the copilot-operations-handover plan's section 3.7 C1
    /// table (verified against `policy.rs::POLICY_TABLE`), with
    /// `run_bronze_maintenance` and `kill_query` per C2/C1 as `WriteHigh`.
    #[test]
    fn tier2_tools_have_the_documented_risk_and_permission() {
        let expected: &[(&str, Risk, &str)] = &[
            ("get_audit_history", Risk::Read, ""),
            ("list_classification_rules", Risk::Read, ""),
            ("list_quality_rules", Risk::Read, ""),
            ("get_cdc_health", Risk::Read, ""),
            ("get_maintenance_metrics", Risk::Read, ""),
            ("run_bronze_maintenance", Risk::WriteHigh, ""),
            ("list_workloads", Risk::Read, ""),
            ("kill_query", Risk::WriteHigh, "workload:cancel"),
            ("export_gold_mart", Risk::WriteLow, ""),
            ("get_gold_export", Risk::Read, ""),
            ("draft_policy", Risk::WriteLow, "policy:write"),
            ("draft_classification_rule", Risk::WriteLow, ""),
            ("draft_quality_rule", Risk::WriteLow, ""),
        ];
        for (name, risk, permission) in expected {
            let spec = find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(spec.risk, *risk, "{name} risk");
            assert_eq!(spec.permission, *permission, "{name} permission");
        }
    }
}
