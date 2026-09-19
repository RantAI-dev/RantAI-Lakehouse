//! `POST /api/ai/chat`, `GET/POST/DELETE /api/ai/sessions`,
//! `GET /api/ai/build-status` — the AI Copilot: agentic tool-calling chat,
//! its chat-history store, and live pipeline-run status polling.
//!
//! Ports `src/app/api/ai/chat/route.ts`, `src/app/api/ai/sessions/route.ts`,
//! `src/app/api/ai/build-status/route.ts`, and the tool registry from
//! `src/services/clients/ai-tools.ts`. Model *text* is inherently
//! non-deterministic and is not chased for byte parity (see
//! `rust/tests/parity/README.md`); the request/response *structure*,
//! validation, tool dispatch, and mode-based tool filtering are ported
//! faithfully.

// T0.5: `audit`, `registry`, and `tools` are `pub(in crate::routes)` (not
// merely `pub(super)`) so `routes::agents::decide_approval` — a SIBLING
// module of `routes::ai`, not a descendant — can reach the tool registry
// (to check the approver's own permission for the tool being approved),
// the shared redact/audit-record helpers, and the SAME tool dispatch
// (`tools::run_tool`) the chat loop and `POST /api/ai/tool` use, so an
// approved `WriteHigh` call executes through the identical code path
// rather than a second, divergent one. `gate` stays private: approval
// CREATION only ever happens from inside this module's own dispatch.
pub(in crate::routes) mod audit;
mod gate;
pub(in crate::routes) mod registry;
pub(in crate::routes) mod tools;

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use lakehouse_llm::{ChatOptions, LlmMessage, LlmMessageRole, ToolCall, ToolCallFunction};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::agent::schema_context;
use crate::state::AppState;

// ── POST /api/ai/build-status ───────────────────────────────────────────

/// Query parameters for `GET /api/ai/build-status`.
#[derive(Debug, Deserialize)]
pub struct BuildStatusQuery {
    #[serde(default, rename = "runId")]
    run_id: Option<String>,
}

/// `GET /api/ai/build-status?runId=` — live per-step status of one
/// `Dagster` run, polled by the AI Copilot's pipeline tree UI.
pub async fn build_status(
    State(state): State<AppState>,
    Query(q): Query<BuildStatusQuery>,
) -> Response {
    let Some(run_id) = q.run_id.filter(|s| !s.is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "runId wajib" })),
        )
            .into_response();
    };
    match state.dagster.pipeline_run_status(&run_id).await {
        Ok(Some(info)) => {
            let steps: Vec<Value> = info
                .steps
                .iter()
                .map(|s| json!({ "key": s.key, "status": s.status }))
                .collect();
            (
                StatusCode::OK,
                ApiJson(json!({ "runId": run_id, "status": info.status, "steps": steps })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "run tidak ditemukan", "status": "unknown", "steps": [] })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": err.to_string(), "status": "unknown", "steps": [] })),
        )
            .into_response(),
    }
}

// ── POST /api/ai/chat ───────────────────────────────────────────────────

const SYSTEM_BASE: &str = "Kamu AI Copilot untuk lakehouse pariwisata DKI Jakarta (RantAI Lakehouse).\n\nPanduan umum:\n- Untuk pertanyaan angka/data: pakai run_sql (SELECT ClickHouse). Cari tabel dulu\n  via list_datasets/describe_dataset kalau belum tahu skema. Utamakan serving.mart_*.\n- Untuk \"ada data apa / soal X\": list_datasets atau describe_dataset.\n- Untuk silsilah data: get_lineage. Untuk kualitas: get_quality.\n- Answer CONCISELY in English (Markdown allowed: tables, bold, lists),\n  berdasarkan HASIL TOOL yang nyata. JANGAN mengarang angka atau tabel.\n  Kalau tool error, katakan apa adanya.";

const SYSTEM_ASK_SUFFIX: &str = "\n\nMODE: ASK (read-only). Kamu HANYA menjawab & menganalisis data — tidak\nmengubah/membangun apa pun (termasuk TIDAK membuat/menghapus chart). Kamu boleh\nmelihat dashboard (describe_mart/list_charts). Kalau user minta membangun data\natau membuat chart, sarankan pindah ke mode Build.";

const SYSTEM_BUILD_SUFFIX: &str = "\n\nMODE: BUILD. Selain menjawab, kamu bisa MENGOPERASIKAN lakehouse:\n- Untuk \"bangun/segarkan Bronze/Silver/Gold\" atau \"refresh data\":\n  JELASKAN dulu rencananya singkat, lalu panggil trigger_lakehouse_build.\n- Setelah trigger, beri tahu user pipeline berjalan (statusnya tampil live).\n- Untuk \"bikin/tambah chart/dashboard soal X\" (BI lewat chat):\n  panggil describe_mart dulu, lalu create_chart dengan kolom yang benar-benar ada.\n- Untuk \"buatkan/sarankan dashboard soal X\" tanpa detail: panggil suggest_dashboard.\n- Untuk mengelompokkan: create_board dulu, lalu create_chart dengan board=<id>.\n- Untuk mengubah kartu: update_chart (kirim semua field dengan nilai baru).\n- Untuk alert/digest: list_alert_rules untuk lihat yang ada, create_alert_rule/\n  update_alert_rule untuk membuat/mengubah (tentukan mart, measure, agg, op,\n  threshold untuk alert; board untuk digest), run_alert_rule untuk menjalankan\n  satu rule sekarang (webhook/email BENERAN terkirim). delete_alert_rule\n  BUTUH PERSETUJUAN MANUSIA dulu sebelum benar-benar terhapus.\n- Untuk connector: list_connectors untuk lihat yang ada, create_connector untuk\n  mendaftarkan baru (secretRef WAJIB berupa referensi seperti \"env:NAMA\" atau\n  \"vault://...\", JANGAN PERNAH kredensial asli), test_connector untuk menguji\n  koneksi nyata. delete_connector BUTUH PERSETUJUAN MANUSIA dulu.\n- Untuk pipeline individual (bukan build utama): list_pipelines/\n  list_pipeline_runs untuk lihat status, trigger_pipeline/retry_pipeline_run\n  untuk menjalankan, resume_pipeline untuk mengaktifkan jadwal lagi.\n  pause_pipeline dan cancel_pipeline_run BUTUH PERSETUJUAN MANUSIA dulu.\n- Untuk saved query: save_query untuk menyimpan SQL bernama, list_saved_queries\n  untuk lihat daftar, run_saved_query untuk menjalankan ulang (hanya query baca\n  yang diizinkan, sama seperti Query Studio).\n- Untuk governance: get_audit_history (riwayat audit), list_classification_rules\n  dan list_quality_rules untuk lihat aturan yang ada, get_cdc_health untuk\n  kesehatan replication slot CDC, get_maintenance_metrics untuk riwayat\n  maintenance Bronze. draft_policy/draft_classification_rule/draft_quality_rule\n  untuk MENULIS aturan/kebijakan baru — SELALU tersimpan sebagai draft/belum\n  dievaluasi, mengaktifkan tetap aksi manusia di console.\n- Untuk maintenance Bronze: run_bronze_maintenance MENERAPKAN perubahan\n  (menghapus file data/manifest Iceberg yatim) — ini BUKAN dry run, dan BUTUH\n  PERSETUJUAN MANUSIA dulu sebelum benar-benar jalan.\n- Untuk workload ClickHouse: list_workloads untuk lihat query yang sedang\n  berjalan, kill_query untuk menghentikan paksa satu query (BUTUH PERSETUJUAN\n  MANUSIA dulu — ini KILL QUERY sungguhan).\n- Untuk Gold export: export_gold_mart untuk mengekspor satu mart ke Iceberg\n  (APPEND-ONLY — menjalankan ulang menambah baris, bukan menggantikan),\n  get_gold_export untuk membaca balik jumlah baris & format version-nya.\n- Tindakan yang butuh persetujuan manusia (delete_alert_rule, delete_connector,\n  pause_pipeline, cancel_pipeline_run, delete_chart, run_bronze_maintenance,\n  kill_query) TIDAK langsung jalan — beri tahu user bahwa permintaan sudah\n  masuk antrean persetujuan di /agents/approvals.\n- Kalau hasil tool berisi needs_confirmation: jawab SATU kalimat singkat saja,\n  mis. \"The chart draft is ready — review the preview below and confirm.\"\n  JANGAN mengulang konfigurasi/argumen (tipe chart, mart, kolom, judul, span,\n  limit) dan jangan minta user mengetik konfirmasi: UI sudah menampilkan\n  pratinjau lengkap beserta tombol konfirmasinya.";

const MAX_ITER: u32 = 8;

/// A single incoming `{role, content}` chat turn (only `user`/`assistant`
/// are kept, matching the `TypeScript`'s `.filter`).
#[derive(Debug, Deserialize)]
struct IncomingMessage {
    role: String,
    #[serde(default)]
    content: String,
}

/// `POST /api/ai/chat` request body.
#[derive(Debug, Default, Deserialize)]
struct ChatBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    messages: Vec<IncomingMessage>,
    /// Answer as an NDJSON progress stream instead of one JSON body; see
    /// [`stream_chat`].
    #[serde(default)]
    stream: bool,
}

/// `POST /api/ai/chat` — the agentic tool-calling loop: the LLM decides
/// which tool(s) to call, the server executes them and feeds results back,
/// up to [`MAX_ITER`] rounds, until the model answers without a tool call.
///
/// # Errors
///
/// Returns 400 [`ApiError::BadRequest`] on an unparseable body or empty
/// `messages`. A downstream LLM failure is NOT an [`ApiResult`] error path
/// — it renders its own 503 body directly (see [`chat`]'s body), matching
/// the `TypeScript`'s single `catch` around the whole loop.
///
/// # Absent-principal behaviour (T0.2)
///
/// `principal` follows the exact `Option<Extension<Principal>>` pattern
/// used by `routes::gold::export` and `routes::alerts::run` — it is
/// populated by the auth middleware whenever a principal was resolved.
/// `POST /api/ai/chat` is `Policy::RequiresAuth` (`policy.rs`), so in
/// practice a principal is always present here; the middleware would have
/// already rejected an unauthenticated request before this handler runs.
///
/// Nonetheless `chat` treats `None` deliberately, rather than assuming it
/// can't happen: [`gate::decide`] (and [`registry::tool_schemas_for`]) is
/// handed `principal.as_ref().map(|Extension(p)| &p.permissions)`, and an
/// absent `PermissionSet` is treated as "authenticated, but zero grants" —
/// fail closed. Every tool whose `permission` is non-empty is refused;
/// only tools with an empty `permission` (`""`, "authenticated only" — see
/// [`registry::ToolSpec::permission`]) still run. This mirrors how the
/// rest of the API treats a policy layer bug or a middleware gap: never
/// silently grant, always require the narrower of "no principal" and "no
/// permissions".
pub async fn chat(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> Response {
    let Ok(parsed) = serde_json::from_slice::<ChatBody>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "Body harus JSON {messages}" })),
        )
            .into_response();
    };
    let principal = principal.map(|Extension(p)| p);
    let stream = parsed.stream;
    let run = match prepare_chat(&state, principal.as_ref(), parsed).await {
        Ok(run) => run,
        Err(response) => return response,
    };
    if stream {
        return stream_chat(state, principal, run);
    }
    match run_chat(&state, principal.as_ref(), run, None).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => llm_unavailable(&err),
    }
}

/// What [`run_chat`] needs: the full prompt and the tools the model may call.
struct PreparedChat {
    messages: Vec<LlmMessage>,
    tools: Vec<Value>,
    is_build: bool,
}

/// Validates the body and assembles the system prompt, history and the tool
/// list for this principal and mode. A bad body is a ready 400 response.
async fn prepare_chat(
    state: &AppState,
    principal: Option<&Principal>,
    parsed: ChatBody,
) -> Result<PreparedChat, Response> {
    let perms = principal.map(|p| &p.permissions);
    let history: Vec<LlmMessage> = parsed
        .messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| LlmMessage {
            role: if m.role == "user" {
                LlmMessageRole::User
            } else {
                LlmMessageRole::Assistant
            },
            content: Some(m.content.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
        .collect();
    if history.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "messages kosong" })),
        )
            .into_response());
    }

    let is_build = parsed.mode.as_deref() == Some("build");
    let schema = schema_context(&state.clickhouse).await.unwrap_or_default();
    let base = if is_build {
        format!("{SYSTEM_BASE}{SYSTEM_BUILD_SUFFIX}")
    } else {
        format!("{SYSTEM_BASE}{SYSTEM_ASK_SUFFIX}")
    };
    let page_ctx: String = parsed
        .context
        .unwrap_or_default()
        .chars()
        .take(800)
        .collect();
    let ctx_line = if page_ctx.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nCURRENT PAGE CONTEXT: {page_ctx}\nTailor your help, wording, and suggestions to where the user currently is."
        )
    };
    let sys = if schema.is_empty() {
        base
    } else {
        format!("{base}\n\nSKEMA TERSEDIA:\n{schema}")
    } + &ctx_line;

    let allow: Option<std::collections::HashSet<String>> = parsed
        .tools
        .filter(|t| !t.is_empty())
        .map(|t| t.into_iter().collect());
    let tools: Vec<Value> = registry::tool_schemas_for(perms)
        .into_iter()
        .filter(|t| {
            let name = t["function"]["name"].as_str().unwrap_or("");
            let is_write =
                registry::find(name).is_some_and(|spec| spec.risk != registry::Risk::Read);
            if !is_build && is_write {
                return false;
            }
            if let Some(allow) = &allow
                && !allow.contains(name)
            {
                return false;
            }
            true
        })
        .collect();

    let mut messages = vec![LlmMessage {
        role: LlmMessageRole::System,
        content: Some(sys),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }];
    messages.extend(history);
    Ok(PreparedChat {
        messages,
        tools,
        is_build,
    })
}

/// Progress events for `stream: true`. `None` for a plain JSON request.
type Progress<'a> = Option<&'a tokio::sync::mpsc::UnboundedSender<Value>>;

/// Sends one progress event; `false` once the client has gone (Stop, or a
/// closed tab), so the loop can end instead of running more rounds for
/// nobody.
fn report(progress: Progress<'_>, event: Value) -> bool {
    progress.is_none_or(|tx| tx.send(event).is_ok())
}

/// The agentic loop: ask the model, run the tools it calls (through
/// [`gate::decide_by_name`]), feed the results back, until it answers
/// without a tool call or [`MAX_ITER`] rounds pass. Returns the response
/// body `{ answer, toolTrace, buildRunId?, chartCreated, note? }`.
#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler's iterative \
              tool-calling loop; splitting it up would scatter one \
              sequential loop across helpers with no independent reuse"
)]
async fn run_chat(
    state: &AppState,
    principal: Option<&Principal>,
    run: PreparedChat,
    progress: Progress<'_>,
) -> Result<Value, lakehouse_llm::LlmError> {
    let perms = principal.map(|p| &p.permissions);
    let PreparedChat {
        mut messages,
        tools,
        is_build,
    } = run;
    let mut tool_trace: Vec<Value> = Vec::new();
    let mut build_run_id: Option<String> = None;
    let mut chart_created = false;

    for _ in 0..MAX_ITER {
        if !report(progress, json!({ "type": "status", "phase": "thinking" })) {
            return Ok(chat_response_body(
                "",
                &tool_trace,
                build_run_id.as_deref(),
                chart_created,
                Some("stopped"),
            ));
        }
        let msg = state
            .llm
            .chat_with_tools(&messages, &tools, ChatOptions::default())
            .await?;
        messages.push(msg.clone());

        let mut calls: Vec<ToolCall> = msg.tool_calls.clone().unwrap_or_default();
        if let Some(content) = &msg.content {
            calls.extend(parse_minimax_tool_calls(content));
        }
        if calls.is_empty() {
            let answer = strip_tool_xml(msg.content.as_deref().unwrap_or(""));
            return Ok(chat_response_body(
                &answer,
                &tool_trace,
                build_run_id.as_deref(),
                chart_created,
                None,
            ));
        }

        let mut xml_feedback: Vec<String> = Vec::new();
        for call in &calls {
            if !report(
                progress,
                json!({ "type": "tool", "tool": call.function.name }),
            ) {
                return Ok(chat_response_body(
                    "",
                    &tool_trace,
                    build_run_id.as_deref(),
                    chart_created,
                    Some("stopped"),
                ));
            }
            let args: Map<String, Value> =
                serde_json::from_str(&call.function.arguments).unwrap_or_default();
            // D3: `ask` mode filters non-Read tools, and permission
            // filters tools the principal lacks, out of the schema
            // ADVERTISED to the model (above, building `tools`), but that
            // alone is not enforcement — a `MiniMax` XML `<invoke
            // name="delete_chart">` embedded in model TEXT (parsed by
            // `parse_minimax_tool_calls` regardless of what was advertised)
            // or a hallucinated `tool_calls` entry reaches this dispatch
            // loop exactly like any legitimate call. Re-check both the
            // risk tier AND the permission HERE, at the one place
            // execution actually happens ([`gate::decide`]), and refuse
            // rather than execute — the model must see the refusal (not
            // have the call silently dropped) so it can tell the user
            // instead of assuming a write (or a read it wasn't allowed)
            // it never got.
            let gate_result = gate::decide_by_name(is_build, perms, &call.function.name, &args);
            let (result, outcome, run_id, approval_id) = if let Some(refusal) = gate_result {
                if gate::is_write_high_pending(&refusal) {
                    let Some(spec) = registry::find(&call.function.name) else {
                        unreachable!("is_write_high_pending only set for a registered tool")
                    };
                    let (_, resource_id) =
                        audit::resource_for(&call.function.name, &args, &json!({}));
                    let redacted = audit::redact(&Value::Object(args.clone()));
                    let actor =
                        principal.map_or_else(|| "unknown".to_owned(), |p| p.display_name.clone());
                    let approval_result = gate::create_write_high_approval(
                        state.pg.as_deref(),
                        &actor,
                        spec,
                        &args,
                        &redacted,
                        resource_id.as_deref(),
                    )
                    .await;
                    let (run_id, approval_id) = if approval_result
                        .get("needs_approval")
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        (
                            approval_result["run_id"].as_str().map(str::to_owned),
                            approval_result["approval_id"].as_str().map(str::to_owned),
                        )
                    } else {
                        (None, None)
                    };
                    let outcome = if run_id.is_some() {
                        "needs_approval"
                    } else {
                        "failed"
                    };
                    (approval_result, outcome, run_id, approval_id)
                } else {
                    let outcome = gate::outcome_of(&refusal);
                    (refusal, outcome, None, None)
                }
            } else {
                let clean_args = gate::strip_confirmed(&args);
                let result = tools::run_tool(state, &call.function.name, &clean_args).await;
                let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
                (result, if ok { "executed" } else { "failed" }, None, None)
            };
            let (resource_kind, resource_id) =
                audit::resource_for(&call.function.name, &args, &result);
            audit::record(
                state.pg.as_deref(),
                principal,
                None,
                &call.function.name,
                resource_kind,
                resource_id.as_deref(),
                &Value::Object(args.clone()),
                outcome,
                None,
                run_id.as_deref(),
                approval_id.as_deref(),
            )
            .await;
            let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
            tool_trace.push(json!({
                "tool": call.function.name, "args": args, "ok": ok, "result": result,
            }));
            if let Value::Object(m) = &result
                && let Some(Value::String(rid)) = m.get("runId")
            {
                build_run_id = Some(rid.clone());
            }
            if call.function.name == "create_chart" && ok {
                chart_created = true;
            }
            let payload: String = serde_json::to_string(&result)
                .unwrap_or_default()
                .chars()
                .take(8000)
                .collect();
            if call.id.starts_with("mmx-") {
                xml_feedback.push(format!("Hasil {}: {payload}", call.function.name));
            } else {
                messages.push(LlmMessage {
                    role: LlmMessageRole::Tool,
                    content: Some(payload),
                    tool_calls: None,
                    tool_call_id: Some(call.id.clone()),
                    name: Some(call.function.name.clone()),
                });
            }
        }
        if !xml_feedback.is_empty() {
            messages.push(LlmMessage {
                role: LlmMessageRole::User,
                content: Some(format!(
                    "HASIL TOOL:\n{}\n\nLanjutkan: pakai hasil ini untuk menjawab, atau panggil tool lain bila perlu.",
                    xml_feedback.join("\n")
                )),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
    }

    // Iteration budget exhausted — ask once more for a final answer, tool-free.
    report(progress, json!({ "type": "status", "phase": "thinking" }));
    messages.push(LlmMessage {
        role: LlmMessageRole::User,
        content: Some("Beri jawaban final ringkas dari hasil di atas.".to_owned()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
    let final_msg = state
        .llm
        .chat_with_tools(&messages, &[], ChatOptions::default())
        .await?;
    Ok(chat_response_body(
        &final_msg.content.unwrap_or_default(),
        &tool_trace,
        build_run_id.as_deref(),
        chart_created,
        Some("batas iterasi tool tercapai"),
    ))
}

/// `stream: true`: the same loop, answered as NDJSON — one JSON object per
/// line. `{"type":"status","phase":"thinking"}` before each model round and
/// `{"type":"tool","tool":…}` before each tool call let the client say what
/// is happening; the last line is `{"type":"done","body":…}` (the plain
/// response body) or `{"type":"error","status":…,"body":…}`. When the
/// client disconnects, the loop stops before its next round or tool.
fn stream_chat(state: AppState, principal: Option<Principal>, run: PreparedChat) -> Response {
    use tokio::io::AsyncWriteExt as _;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let (mut writer, reader) = tokio::io::duplex(16 * 1024);

    tokio::spawn(async move {
        let last = match run_chat(&state, principal.as_ref(), run, Some(&tx)).await {
            Ok(body) => json!({ "type": "done", "body": body }),
            Err(err) => json!({
                "type": "error",
                "status": StatusCode::SERVICE_UNAVAILABLE.as_u16(),
                "body": llm_unavailable_body(&err),
            }),
        };
        let _ = tx.send(last);
    });
    // Dropping `rx` when a write fails (client gone) makes the loop's next
    // `report` return false.
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let line = format!("{event}\n");
            if writer.write_all(line.as_bytes()).await.is_err() || writer.flush().await.is_err() {
                break;
            }
            if event["type"] == "done" || event["type"] == "error" {
                break;
            }
        }
    });

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/x-ndjson"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(reader)),
    )
        .into_response()
}

// ── POST /api/ai/tool ───────────────────────────────────────────────────

/// `POST /api/ai/tool` request body.
#[derive(Debug, Deserialize)]
struct ToolCallBody {
    tool: String,
    #[serde(default)]
    args: Map<String, Value>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
}

/// `POST /api/ai/tool` — executes exactly ONE gated tool call and returns
/// its result (T0.4 of the copilot-operations-handover plan).
///
/// This exists so the frontend's Confirm button (on a `needs_confirmation`
/// result from [`chat`]) doesn't have to re-prompt the LLM to re-emit the
/// same call: it resends the identical `{tool, args}` here with
/// `confirmed: true` merged in.
///
/// This is NOT a bypass of [`gate::decide`] — it is the exact same
/// ask-mode check, permission check, and confirmation check the chat loop
/// runs, with the SAME refusal/`needs_confirmation` shapes. The only thing
/// that lets a call through here that the chat loop would have blocked a
/// moment ago is `args["confirmed"] == true`, which is precisely the
/// signal a `WriteLow` call needs to actually execute (see [`gate::decide`]).
///
/// # Errors
///
/// Returns 400 for an unparseable body or an unregistered `tool` name.
pub async fn tool_call(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> Response {
    let perms = principal.as_ref().map(|Extension(p)| &p.permissions);
    let parsed: ToolCallBody = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                ApiJson(json!({ "error": "Body harus JSON {tool, args}" })),
            )
                .into_response();
        }
    };
    let Some(spec) = registry::find(&parsed.tool) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("tool tak dikenal: {}", parsed.tool) })),
        )
            .into_response();
    };
    let is_build = parsed.mode.as_deref() == Some("build");

    let gate_result = gate::decide(is_build, perms, spec, &parsed.args);
    let (result, outcome, run_id, approval_id) = if let Some(refusal) = gate_result {
        if gate::is_write_high_pending(&refusal) {
            let (_, resource_id) = audit::resource_for(&parsed.tool, &parsed.args, &json!({}));
            let redacted = audit::redact(&Value::Object(parsed.args.clone()));
            let actor = principal.as_ref().map_or_else(
                || "unknown".to_owned(),
                |Extension(p)| p.display_name.clone(),
            );
            let approval_result = gate::create_write_high_approval(
                state.pg.as_deref(),
                &actor,
                spec,
                &parsed.args,
                &redacted,
                resource_id.as_deref(),
            )
            .await;
            let (run_id, approval_id) = if approval_result
                .get("needs_approval")
                .and_then(Value::as_bool)
                == Some(true)
            {
                (
                    approval_result["run_id"].as_str().map(str::to_owned),
                    approval_result["approval_id"].as_str().map(str::to_owned),
                )
            } else {
                (None, None)
            };
            let outcome = if run_id.is_some() {
                "needs_approval"
            } else {
                "failed"
            };
            (approval_result, outcome, run_id, approval_id)
        } else {
            let outcome = gate::outcome_of(&refusal);
            (refusal, outcome, None, None)
        }
    } else {
        let clean_args = gate::strip_confirmed(&parsed.args);
        let result = tools::run_tool(&state, &parsed.tool, &clean_args).await;
        let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
        (result, if ok { "executed" } else { "failed" }, None, None)
    };
    let (resource_kind, resource_id) = audit::resource_for(&parsed.tool, &parsed.args, &result);
    audit::record(
        state.pg.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
        parsed.session_id.as_deref(),
        &parsed.tool,
        resource_kind,
        resource_id.as_deref(),
        &Value::Object(parsed.args.clone()),
        outcome,
        None,
        run_id.as_deref(),
        approval_id.as_deref(),
    )
    .await;

    (
        StatusCode::OK,
        ApiJson(json!({ "tool": parsed.tool, "outcome": outcome, "result": result })),
    )
        .into_response()
}

/// Builds the `/api/ai/chat` response body, matching the TypeScript's
/// `{ answer, toolTrace, buildRunId, chartCreated, note? }` object literal:
/// when `build_run_id` is `None` (the TS-side `undefined`), the key is
/// omitted entirely rather than serialized as `null` — `JSON.stringify`
/// drops `undefined`-valued object keys, so a bare `Option<String>` field
/// in a `json!` macro call (which always emits `null`) would diverge.
fn chat_response_body(
    answer: &str,
    tool_trace: &[Value],
    build_run_id: Option<&str>,
    chart_created: bool,
    note: Option<&str>,
) -> Value {
    let mut body = Map::new();
    body.insert("answer".to_owned(), json!(answer));
    body.insert("toolTrace".to_owned(), json!(tool_trace));
    if let Some(rid) = build_run_id {
        body.insert("buildRunId".to_owned(), json!(rid));
    }
    body.insert("chartCreated".to_owned(), json!(chart_created));
    if let Some(note) = note {
        body.insert("note".to_owned(), json!(note));
    }
    Value::Object(body)
}

fn llm_unavailable(err: &lakehouse_llm::LlmError) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        ApiJson(llm_unavailable_body(err)),
    )
        .into_response()
}

fn llm_unavailable_body(err: &lakehouse_llm::LlmError) -> Value {
    json!({
        "error": "AI Copilot tak tersedia",
        "detail": err.to_string(),
        "hint": "Set LLM_KEY (MiniMax) di .env.local.",
    })
}

/// `MiniMax-M2` sometimes emits a tool call as XML in `content` rather than
/// the standard `OpenAI` `tool_calls` field:
/// `<minimax:tool_call><invoke name="run_sql"><parameter
/// name="sql">SELECT ...</parameter></invoke></minimax:tool_call>`. Parse
/// that into standard [`ToolCall`]s so the loop keeps working, matching
/// `parseMinimaxToolCalls` in `ai/chat/route.ts` verbatim.
fn parse_minimax_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut idx = 0usize;
    let mut pos = 0usize;
    while let Some(start_rel) = content[pos..].find("<invoke name=\"") {
        let name_start = pos + start_rel + "<invoke name=\"".len();
        let Some(name_end_rel) = content[name_start..].find('"') else {
            break;
        };
        let name = &content[name_start..name_start + name_end_rel];
        let Some(tag_close_rel) = content[name_start + name_end_rel..].find('>') else {
            break;
        };
        let body_start = name_start + name_end_rel + tag_close_rel + 1;
        let Some(end_rel) = content[body_start..].find("</invoke>") else {
            break;
        };
        let body = &content[body_start..body_start + end_rel];

        // `serde_json::Map` (not `HashMap`): the workspace's `preserve_order`
        // feature makes this an order-preserving map, so the `arguments`
        // JSON string and `toolTrace` reflect the XML's actual `<parameter>`
        // order — a `HashMap` here randomized that key order across runs,
        // unlike the rest of the codebase's `serde_json::Map` convention.
        let mut args = Map::new();
        let mut param_pos = 0usize;
        while let Some(p_start_rel) = body[param_pos..].find("<parameter name=\"") {
            let p_name_start = param_pos + p_start_rel + "<parameter name=\"".len();
            let Some(p_name_end_rel) = body[p_name_start..].find('"') else {
                break;
            };
            let p_name = &body[p_name_start..p_name_start + p_name_end_rel];
            let Some(p_tag_close_rel) = body[p_name_start + p_name_end_rel..].find('>') else {
                break;
            };
            let p_body_start = p_name_start + p_name_end_rel + p_tag_close_rel + 1;
            let Some(p_end_rel) = body[p_body_start..].find("</parameter>") else {
                break;
            };
            let p_value = body[p_body_start..p_body_start + p_end_rel].trim();
            args.insert(p_name.to_owned(), Value::String(p_value.to_owned()));
            param_pos = p_body_start + p_end_rel + "</parameter>".len();
        }

        calls.push(ToolCall {
            id: format!("mmx-{idx}"),
            kind: "function".to_owned(),
            function: ToolCallFunction {
                name: name.to_owned(),
                arguments: serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_owned()),
            },
        });
        idx += 1;
        pos = body_start + end_rel + "</invoke>".len();
    }
    calls
}

/// `s.replace(/<minimax:tool_call>[\s\S]*?<\/minimax:tool_call>/gi,
/// "").replace(/<\/?think>/gi, "").trim()`.
fn strip_tool_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let lower = s.to_ascii_lowercase();
    let mut pos = 0usize;
    loop {
        let Some(open_rel) = lower[pos..].find("<minimax:tool_call>") else {
            out.push_str(&s[pos..]);
            break;
        };
        let open = pos + open_rel;
        let after_open = open + "<minimax:tool_call>".len();
        let Some(close_rel) = lower[after_open..].find("</minimax:tool_call>") else {
            out.push_str(&s[pos..]);
            break;
        };
        out.push_str(&s[pos..open]);
        pos = after_open + close_rel + "</minimax:tool_call>".len();
    }
    strip_ci(&strip_ci(&out, "<think>"), "</think>")
        .trim()
        .to_owned()
}

/// Case-insensitively removes every occurrence of `needle` from `s`,
/// matching the `/gi` flags on the TypeScript's `/<\/?think>/gi` — a
/// literal-case `.replace("<think>", "")` chain (ASCII-only lower/upper
/// variants) would leave a mixed-case tag like `<Think>` in the output.
/// `needle` must be ASCII (true of `<think>`/`</think>`, the only callers).
fn strip_ci(s: &str, needle: &str) -> String {
    let lower_s = s.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut pos = 0usize;
    while let Some(rel) = lower_s[pos..].find(&lower_needle) {
        let start = pos + rel;
        out.push_str(&s[pos..start]);
        pos = start + needle.len();
    }
    out.push_str(&s[pos..]);
    out
}

// ── /api/ai/sessions ────────────────────────────────────────────────────

const CREATE_CHAT_SESSION_TABLE: &str = "CREATE TABLE IF NOT EXISTS console.chat_session (\
     id String, title String, mode String DEFAULT 'ask', \
     messages_json String, updated_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0, \
     owner_id String DEFAULT '', title_locked UInt8 DEFAULT 0 \
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY id";

/// Columns added after the table first shipped; `IF NOT EXISTS` makes them
/// safe on both fresh and existing tables.
const CHAT_SESSION_MIGRATIONS: [&str; 2] = [
    "ALTER TABLE console.chat_session ADD COLUMN IF NOT EXISTS owner_id String DEFAULT ''",
    "ALTER TABLE console.chat_session ADD COLUMN IF NOT EXISTS title_locked UInt8 DEFAULT 0",
];

/// Longest title derived from a conversation's first message.
const TITLE_MAX_CHARS: usize = 80;

static CHAT_SESSION_TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Create the `console` database and `chat_session` table if they don't
/// already exist, and add later columns (idempotent, once per process —
/// mirroring [`lakehouse_bi::store::ensure_bi_table`]'s pattern).
async fn ensure_chat_session_table(ch: &ChClient) -> Result<(), lakehouse_clickhouse::ChError> {
    CHAT_SESSION_TABLE_ENSURED
        .get_or_try_init(|| async {
            ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
                .await?;
            ch.exec(CREATE_CHAT_SESSION_TABLE, None).await?;
            for sql in CHAT_SESSION_MIGRATIONS {
                ch.exec(sql, None).await?;
            }
            Ok(())
        })
        .await
        .map(drop)
}

/// `s.replace(/\\/g, "\\\\").replace(/'/g, "''")`.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

/// The signed-in user every session operation is scoped to.
///
/// Sessions hold whatever was asked and answered, so they are private to
/// their owner: before `owner_id` existed, every user listed, opened and
/// deleted everyone else's. Rows written before then have an empty owner
/// and belong to nobody — they are hidden rather than guessed at.
fn session_owner(principal: Option<&Extension<Principal>>) -> Result<String, ApiError> {
    principal
        .map(|Extension(p)| p.id.uuid().to_string())
        .ok_or_else(|| ApiError::Unauthorized("sign in required".to_owned()))
}

fn internal(err: &impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        ApiJson(json!({ "error": err.to_string() })),
    )
        .into_response()
}

/// Query parameters for `GET /api/ai/sessions`: `?id=` for one session,
/// otherwise a page of the list.
#[derive(Debug, Default, Deserialize)]
pub struct SessionsQuery {
    #[serde(default)]
    id: Option<String>,
    /// Matched against the title and the conversation text.
    #[serde(default)]
    q: Option<String>,
    /// `ask` or `build`; anything else lists both.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

/// Query parameters for `DELETE /api/ai/sessions?id=`.
#[derive(Debug, Deserialize)]
pub struct SessionIdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// The list filters as a `WHERE` clause — owner always, then mode and search.
fn session_filter(owner: &str, mode: Option<&str>, q: Option<&str>) -> String {
    let mut conditions = vec![
        "is_deleted = 0".to_owned(),
        format!("owner_id = '{}'", esc(owner)),
    ];
    if let Some(m) = mode.filter(|m| matches!(*m, "ask" | "build")) {
        conditions.push(format!("mode = '{m}'"));
    }
    if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
        let q = esc(q);
        conditions.push(format!(
            "(positionCaseInsensitiveUTF8(title, '{q}') > 0 \
             OR positionCaseInsensitiveUTF8(messages_json, '{q}') > 0)"
        ));
    }
    conditions.join(" AND ")
}

/// `GET /api/ai/sessions` — a page of the caller's sessions, or `?id=` for
/// one of them in full. Another user's session answers 404, not 403, so ids
/// can't be probed.
pub async fn sessions_get(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<SessionsQuery>,
) -> Response {
    let owner = match session_owner(principal.as_ref()) {
        Ok(owner) => owner,
        Err(err) => return crate::error::ApiRejection::from(err).into_response(),
    };
    let ch = &state.clickhouse;
    if let Err(err) = ensure_chat_session_table(ch).await {
        return internal(&err);
    }
    if let Some(id) = q.id.as_deref().filter(|s| !s.is_empty()) {
        return match session_detail(ch, &owner, id).await {
            Ok(Some(session)) => {
                (StatusCode::OK, ApiJson(json!({ "session": session }))).into_response()
            }
            Ok(None) => (
                StatusCode::NOT_FOUND,
                ApiJson(json!({ "error": "sesi tidak ditemukan" })),
            )
                .into_response(),
            Err(err) => internal(&err),
        };
    }
    match session_list(ch, &owner, &q).await {
        Ok(page) => (StatusCode::OK, ApiJson(page)).into_response(),
        Err(err) => internal(&err),
    }
}

/// One page of sessions, newest first, plus per-mode counts for the same
/// search so a filter can show how many each choice holds.
async fn session_list(
    ch: &ChClient,
    owner: &str,
    q: &SessionsQuery,
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).min(100_000);
    let filter = session_filter(owner, q.mode.as_deref(), q.q.as_deref());
    // One row past the page says whether another page exists.
    let rows = ch
        .rows(
            &format!(
                "SELECT id, title, mode, toString(updated_at) AS updated_at, \
                 positionCaseInsensitive(messages_json, '\"chartCreated\":true') > 0 AS chart_created, \
                 substringUTF8(JSONExtractString(messages_json, -1, 'content'), 1, 240) AS preview \
                 FROM console.chat_session FINAL WHERE {filter} \
                 ORDER BY updated_at DESC LIMIT {} OFFSET {offset}",
                limit + 1
            ),
            None,
        )
        .await?;
    let has_more = rows.len() > limit as usize;
    let sessions: Vec<Value> = rows
        .iter()
        .take(limit as usize)
        .map(|r| {
            json!({
                "id": r.get("id"), "title": r.get("title"), "mode": r.get("mode"),
                "updatedAt": r.get("updated_at"),
                "chartCreated": r.get("chart_created").and_then(json_u64).unwrap_or(0) > 0,
                "preview": r.get("preview"),
            })
        })
        .collect();

    let count_filter = session_filter(owner, None, q.q.as_deref());
    let count_rows = ch
        .rows(
            &format!(
                "SELECT mode, count() AS n FROM console.chat_session FINAL \
                 WHERE {count_filter} GROUP BY mode"
            ),
            None,
        )
        .await?;
    let mut counts = Map::new();
    for r in &count_rows {
        if let Some(mode) = r.get("mode").and_then(Value::as_str) {
            counts.insert(
                mode.to_owned(),
                json!(r.get("n").and_then(json_u64).unwrap_or(0)),
            );
        }
    }
    Ok(json!({
        "sessions": sessions,
        "hasMore": has_more,
        "nextOffset": has_more.then(|| offset + limit),
        "counts": counts,
    }))
}

/// `ClickHouse` returns `UInt64` as a JSON string and smaller ints as numbers.
fn json_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

async fn session_row(
    ch: &ChClient,
    owner: &str,
    id: &str,
) -> Result<Option<Map<String, Value>>, lakehouse_clickhouse::ChError> {
    let rows = ch
        .rows(
            &format!(
                "SELECT id, title, mode, messages_json, title_locked, toString(updated_at) AS updated_at \
                 FROM console.chat_session FINAL \
                 WHERE is_deleted = 0 AND id = '{}' AND owner_id = '{}' LIMIT 1",
                esc(id),
                esc(owner)
            ),
            None,
        )
        .await?;
    Ok(rows.into_iter().next())
}

async fn session_detail(
    ch: &ChClient,
    owner: &str,
    id: &str,
) -> Result<Option<Value>, lakehouse_clickhouse::ChError> {
    let Some(row) = session_row(ch, owner, id).await? else {
        return Ok(None);
    };
    let messages_json = row
        .get("messages_json")
        .and_then(Value::as_str)
        .unwrap_or("[]");
    let messages: Value = serde_json::from_str(messages_json).unwrap_or_else(|_| json!([]));
    Ok(Some(json!({
        "id": row.get("id"), "title": row.get("title"), "mode": row.get("mode"),
        "messages": messages, "updatedAt": row.get("updated_at"),
    })))
}

/// `POST /api/ai/sessions` request body.
#[derive(Debug, Default, Deserialize)]
struct SaveSessionBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    messages: Option<Vec<Value>>,
}

/// A title from the first user message: whitespace collapsed, cut at
/// [`TITLE_MAX_CHARS`] with an ellipsis so a cut title reads as one.
fn derive_title(messages: &[Value]) -> String {
    let raw = messages
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let collapsed = collapse_whitespace(raw);
    if collapsed.is_empty() {
        return "Percakapan".to_owned();
    }
    if collapsed.chars().count() <= TITLE_MAX_CHARS {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(TITLE_MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Insert one full version of a session row (`ReplacingMergeTree` keeps the
/// newest by `updated_at`).
async fn write_session(
    ch: &ChClient,
    row: &SessionWrite<'_>,
) -> Result<(), lakehouse_clickhouse::ChError> {
    let sql = format!(
        "INSERT INTO console.chat_session \
         (id, title, mode, messages_json, owner_id, title_locked, is_deleted) \
         VALUES ('{}', '{}', '{}', '{}', '{}', {}, {})",
        esc(row.id),
        esc(row.title),
        esc(row.mode),
        esc(row.messages_json),
        esc(row.owner),
        u8::from(row.title_locked),
        u8::from(row.deleted),
    );
    ch.exec(&sql, None).await
}

struct SessionWrite<'a> {
    id: &'a str,
    owner: &'a str,
    title: &'a str,
    title_locked: bool,
    mode: &'a str,
    messages_json: &'a str,
    deleted: bool,
}

/// `POST /api/ai/sessions` — save/replace one of the caller's sessions (id
/// optional → new). A renamed session keeps its title.
///
/// # Errors
///
/// 400 when `messages` is missing/empty; 401 without a signed-in user; 404
/// when `id` names a session the caller doesn't own; 500 on a `ClickHouse`
/// failure.
pub async fn sessions_save(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let parsed: SaveSessionBody = serde_json::from_slice(&body).unwrap_or_default();
    let Some(messages) = parsed.messages.filter(|m| !m.is_empty()) else {
        return Err(ApiError::BadRequest("messages kosong".to_owned()).into());
    };
    let ch = &state.clickhouse;
    ensure_chat_session_table(ch)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    let requested = parsed
        .id
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let existing = match &requested {
        Some(id) => {
            let row = session_row(ch, &owner, id)
                .await
                .map_err(|err| ApiError::Internal(err.to_string()))?;
            // Never overwrite someone else's session under its id.
            Some(row.ok_or_else(|| ApiError::NotFound("sesi tidak ditemukan".to_owned()))?)
        }
        None => None,
    };
    let id = requested.unwrap_or_else(new_session_id);
    let locked = existing
        .as_ref()
        .and_then(|r| r.get("title_locked"))
        .and_then(json_u64)
        .is_some_and(|v| v > 0);
    let title = if locked {
        existing
            .as_ref()
            .and_then(|r| r.get("title"))
            .and_then(Value::as_str)
            .map_or_else(|| derive_title(&messages), ToOwned::to_owned)
    } else {
        derive_title(&messages)
    };

    let mut json_body = serde_json::to_string(&messages).unwrap_or_else(|_| "[]".to_owned());
    if json_body.len() > 200_000 {
        let trimmed: Vec<Value> = messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.get("role"), "content": m.get("content"),
                    "buildRunId": m.get("buildRunId"), "chartCreated": m.get("chartCreated"),
                })
            })
            .collect();
        json_body = serde_json::to_string(&trimmed).unwrap_or_else(|_| "[]".to_owned());
    }

    let mode = parsed.mode.unwrap_or_else(|| "ask".to_owned());
    write_session(
        ch,
        &SessionWrite {
            id: &id,
            owner: &owner,
            title: &title,
            title_locked: locked,
            mode: &mode,
            messages_json: &json_body,
            deleted: false,
        },
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "id": id, "title": title })))
}

/// `PATCH /api/ai/sessions` request body.
#[derive(Debug, Default, Deserialize)]
struct RenameSessionBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

/// `PATCH /api/ai/sessions` — rename one of the caller's sessions. The new
/// title is locked, so later saves no longer re-derive it.
///
/// # Errors
///
/// 400 when `id` or `title` is missing; 401 without a signed-in user; 404
/// when the caller doesn't own the session; 500 on a `ClickHouse` failure.
pub async fn sessions_rename(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let parsed: RenameSessionBody = serde_json::from_slice(&body).unwrap_or_default();
    let id = parsed
        .id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("id wajib".to_owned()))?;
    let title = parsed
        .title
        .map(|t| collapse_whitespace(&t))
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::BadRequest("judul wajib".to_owned()))?;
    let title: String = title.chars().take(120).collect();
    let ch = &state.clickhouse;
    ensure_chat_session_table(ch)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let row = session_row(ch, &owner, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?
        .ok_or_else(|| ApiError::NotFound("sesi tidak ditemukan".to_owned()))?;
    let field = |k: &str| {
        row.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    write_session(
        ch,
        &SessionWrite {
            id: &id,
            owner: &owner,
            title: &title,
            title_locked: true,
            mode: &field("mode"),
            messages_json: &field("messages_json"),
            deleted: false,
        },
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "id": id, "title": title })))
}

/// `c_<8 random hex chars>` — matching `` `c_${randomUUID().slice(0, 8)}` ``.
fn new_session_id() -> String {
    use std::fmt::Write as _;

    use rand::RngCore;
    let mut bytes = [0_u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut hex = String::with_capacity(8);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    format!("c_{hex}")
}

/// `s.replace(/\s+/g, " ").trim()`.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `DELETE /api/ai/sessions?id=` — soft-delete one of the caller's sessions.
///
/// # Errors
///
/// 400 when `id` is missing; 401 without a signed-in user; 404 when the
/// caller doesn't own the session; 500 on a `ClickHouse` failure.
pub async fn sessions_delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<SessionIdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let owner = session_owner(principal.as_ref())?;
    let Some(id) = q.id.filter(|s| !s.is_empty()) else {
        return Err(ApiError::BadRequest("id wajib".to_owned()).into());
    };
    let ch = &state.clickhouse;
    ensure_chat_session_table(ch)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    if session_row(ch, &owner, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?
        .is_none()
    {
        return Err(ApiError::NotFound("sesi tidak ditemukan".to_owned()).into());
    }
    write_session(
        ch,
        &SessionWrite {
            id: &id,
            owner: &owner,
            title: "",
            title_locked: false,
            mode: "ask",
            messages_json: "[]",
            deleted: true,
        },
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn session_filter_always_scopes_to_the_owner() {
        let f = session_filter("u-1", None, None);
        assert_eq!(f, "is_deleted = 0 AND owner_id = 'u-1'");
    }

    #[test]
    fn session_filter_whitelists_mode_and_escapes_search() {
        let f = session_filter("u-1", Some("build"), Some(" it's "));
        assert!(f.contains("mode = 'build'"));
        assert!(f.contains("positionCaseInsensitiveUTF8(title, 'it''s')"));
        let f = session_filter("u-1", Some("x' OR 1=1 --"), None);
        assert!(
            !f.contains("mode"),
            "unknown modes are dropped, not quoted in"
        );
    }

    #[test]
    fn derive_title_marks_a_cut_with_an_ellipsis() {
        let long = "kata ".repeat(40);
        let t = derive_title(&[json!({ "role": "user", "content": long })]);
        assert_eq!(t.chars().count(), TITLE_MAX_CHARS);
        assert!(t.ends_with('…'));
        let short = derive_title(&[json!({ "role": "user", "content": "  halo   dunia " })]);
        assert_eq!(short, "halo dunia");
        assert_eq!(
            derive_title(&[json!({ "role": "assistant", "content": "x" })]),
            "Percakapan"
        );
    }

    #[test]
    fn parse_minimax_tool_calls_extracts_name_and_args() {
        let content = r#"<minimax:tool_call><invoke name="run_sql"><parameter name="sql">SELECT 1</parameter></invoke></minimax:tool_call>"#;
        let calls = parse_minimax_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "run_sql");
        assert_eq!(calls[0].id, "mmx-0");
        let args: Map<String, Value> = serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(args.get("sql").and_then(Value::as_str), Some("SELECT 1"));
    }

    #[test]
    fn parse_minimax_tool_calls_handles_multiple_invokes() {
        let content = r#"<invoke name="a"><parameter name="x">1</parameter></invoke><invoke name="b"></invoke>"#;
        let calls = parse_minimax_tool_calls(content);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "mmx-0");
        assert_eq!(calls[1].id, "mmx-1");
        assert_eq!(calls[1].function.name, "b");
    }

    #[test]
    fn parse_minimax_tool_calls_empty_for_plain_text() {
        assert!(parse_minimax_tool_calls("just a normal answer").is_empty());
    }

    #[test]
    fn strip_tool_xml_removes_tool_call_block_entirely() {
        let s = "before <minimax:tool_call>xyz</minimax:tool_call> after";
        assert_eq!(strip_tool_xml(s), "before  after");
    }

    #[test]
    fn strip_tool_xml_removes_think_tags_but_keeps_their_content() {
        // `.replace(/<\/?think>/gi, "")` strips only the tags, not the
        // text between them — a deliberate TS quirk (think content is
        // usually already empty/whitespace by the time this runs, since
        // `chat_with_tools` already stripped full <think>...</think>
        // blocks upstream; this is a defense-in-depth pass for whatever
        // slips through).
        let s = "before <think>hmm</think>after";
        assert_eq!(strip_tool_xml(s), "before hmmafter");
    }

    #[test]
    fn strip_tool_xml_trims_result() {
        assert_eq!(strip_tool_xml("  hello  "), "hello");
    }

    /// D4 regression: the TS strips `<think>`/`</think>` case-insensitively
    /// (`/<\/?think>/gi`); the Rust port used a literal-case `.replace`
    /// chain that only covered all-lowercase and all-uppercase, so a
    /// mixed-case tag like `<Think>` survived into the answer.
    #[test]
    fn strip_tool_xml_removes_think_tags_case_insensitively() {
        let s = "before <Think>hmm</Think>after";
        assert_eq!(strip_tool_xml(s), "before hmmafter");
        let s = "before <ThInK>hmm</thINK>after";
        assert_eq!(strip_tool_xml(s), "before hmmafter");
    }

    /// D4 regression: `parse_minimax_tool_calls`' `args` map used to be a
    /// `std::collections::HashMap`, randomizing the `arguments` JSON
    /// string's key order across runs. `serde_json::Map` (with the
    /// workspace's `preserve_order` feature) must preserve XML parameter
    /// order instead.
    #[test]
    fn parse_minimax_tool_calls_preserves_parameter_order() {
        let content = r#"<invoke name="create_chart"><parameter name="title">T</parameter><parameter name="kind">kpi</parameter><parameter name="mart">m</parameter></invoke>"#;
        let calls = parse_minimax_tool_calls(content);
        assert_eq!(
            calls[0].function.arguments,
            r#"{"title":"T","kind":"kpi","mart":"m"}"#
        );
    }

    /// D4 regression: `buildRunId` must be omitted from the response body
    /// when absent, matching the TS's `JSON.stringify` dropping an
    /// `undefined`-valued key — a bare `Option<String>` field in a `json!`
    /// macro call always serializes as `"buildRunId": null` instead.
    #[test]
    fn chat_response_body_omits_build_run_id_when_none() {
        let body = chat_response_body("ok", &[], None, false, None);
        let obj = body.as_object().unwrap();
        assert!(!obj.contains_key("buildRunId"));
        assert!(!obj.contains_key("note"));
    }

    #[test]
    fn chat_response_body_includes_build_run_id_when_some() {
        let body = chat_response_body(
            "ok",
            &[],
            Some("r_1"),
            true,
            Some("batas iterasi tool tercapai"),
        );
        let obj = body.as_object().unwrap();
        assert_eq!(obj.get("buildRunId").unwrap(), "r_1");
        assert_eq!(obj.get("chartCreated").unwrap(), true);
        assert_eq!(obj.get("note").unwrap(), "batas iterasi tool tercapai");
    }

    #[test]
    fn collapse_whitespace_joins_runs_of_whitespace() {
        assert_eq!(collapse_whitespace("a   b\n\tc"), "a b c");
    }

    #[test]
    fn new_session_id_has_expected_shape() {
        let id = new_session_id();
        assert!(id.starts_with("c_"));
        assert_eq!(id.len(), 10);
    }

    #[test]
    fn esc_doubles_backslashes_and_quotes() {
        assert_eq!(esc("O'Brien\\x"), "O''Brien\\\\x");
    }
}
