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

mod audit;
mod gate;
mod registry;
mod tools;

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

const SYSTEM_BUILD_SUFFIX: &str = "\n\nMODE: BUILD. Selain menjawab, kamu bisa MENGOPERASIKAN lakehouse:\n- Untuk \"bangun/segarkan Bronze/Silver/Gold\" atau \"refresh data\":\n  JELASKAN dulu rencananya singkat, lalu panggil trigger_lakehouse_build.\n- Setelah trigger, beri tahu user pipeline berjalan (statusnya tampil live).\n- Untuk \"bikin/tambah chart/dashboard soal X\" (BI lewat chat):\n  panggil describe_mart dulu, lalu create_chart dengan kolom yang benar-benar ada.\n- Untuk \"buatkan/sarankan dashboard soal X\" tanpa detail: panggil suggest_dashboard.\n- Untuk mengelompokkan: create_board dulu, lalu create_chart dengan board=<id>.\n- Untuk mengubah kartu: update_chart (kirim semua field dengan nilai baru).";

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
#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler's iterative \
              tool-calling loop; splitting it up would scatter one \
              sequential loop across helpers with no independent reuse"
)]
pub async fn chat(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> Response {
    let perms = principal.as_ref().map(|Extension(p)| &p.permissions);
    let parsed: ChatBody = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                ApiJson(json!({ "error": "Body harus JSON {messages}" })),
            )
                .into_response();
        }
    };
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
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "messages kosong" })),
        )
            .into_response();
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

    let mut tool_trace: Vec<Value> = Vec::new();
    let mut build_run_id: Option<String> = None;
    let mut chart_created = false;

    for _ in 0..MAX_ITER {
        let msg = match state
            .llm
            .chat_with_tools(&messages, &tools, ChatOptions::default())
            .await
        {
            Ok(m) => m,
            Err(err) => return llm_unavailable(&err),
        };
        messages.push(msg.clone());

        let mut calls: Vec<ToolCall> = msg.tool_calls.clone().unwrap_or_default();
        if let Some(content) = &msg.content {
            calls.extend(parse_minimax_tool_calls(content));
        }
        if calls.is_empty() {
            let answer = strip_tool_xml(msg.content.as_deref().unwrap_or(""));
            return (
                StatusCode::OK,
                ApiJson(chat_response_body(
                    &answer,
                    &tool_trace,
                    build_run_id.as_deref(),
                    chart_created,
                    None,
                )),
            )
                .into_response();
        }

        let mut xml_feedback: Vec<String> = Vec::new();
        for call in &calls {
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
            let (result, outcome) = if let Some(refusal) = gate_result {
                let outcome = gate::outcome_of(&refusal);
                (refusal, outcome)
            } else {
                let clean_args = gate::strip_confirmed(&args);
                let result = tools::run_tool(&state, &call.function.name, &clean_args).await;
                let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
                (result, if ok { "executed" } else { "failed" })
            };
            let (resource_kind, resource_id) =
                audit::resource_for(&call.function.name, &args, &result);
            audit::record(
                state.pg.as_deref(),
                principal.as_ref().map(|Extension(p)| p),
                None,
                &call.function.name,
                resource_kind,
                resource_id.as_deref(),
                &Value::Object(args.clone()),
                outcome,
                None,
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
    messages.push(LlmMessage {
        role: LlmMessageRole::User,
        content: Some("Beri jawaban final ringkas dari hasil di atas.".to_owned()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
    match state
        .llm
        .chat_with_tools(&messages, &[], ChatOptions::default())
        .await
    {
        Ok(final_msg) => (
            StatusCode::OK,
            ApiJson(chat_response_body(
                &final_msg.content.unwrap_or_default(),
                &tool_trace,
                build_run_id.as_deref(),
                chart_created,
                Some("batas iterasi tool tercapai"),
            )),
        )
            .into_response(),
        Err(err) => llm_unavailable(&err),
    }
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
    let (result, outcome) = if let Some(refusal) = gate_result {
        let outcome = gate::outcome_of(&refusal);
        (refusal, outcome)
    } else {
        let clean_args = gate::strip_confirmed(&parsed.args);
        let result = tools::run_tool(&state, &parsed.tool, &clean_args).await;
        let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
        (result, if ok { "executed" } else { "failed" })
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
        ApiJson(json!({
            "error": "AI Copilot tak tersedia",
            "detail": err.to_string(),
            "hint": "Set LLM_KEY (MiniMax) di .env.local.",
        })),
    )
        .into_response()
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
     messages_json String, updated_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0 \
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY id";

static CHAT_SESSION_TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Create the `console` database and `chat_session` table if they don't
/// already exist (idempotent, once per process — mirroring `chat-store.ts`'s
/// module-level `ensured` flag, and [`lakehouse_bi::store::ensure_bi_table`]'s
/// identical pattern).
async fn ensure_chat_session_table(ch: &ChClient) -> Result<(), lakehouse_clickhouse::ChError> {
    CHAT_SESSION_TABLE_ENSURED
        .get_or_try_init(|| async {
            ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
                .await?;
            ch.exec(CREATE_CHAT_SESSION_TABLE, None).await
        })
        .await
        .map(drop)
}

/// `s.replace(/\\/g, "\\\\").replace(/'/g, "''")`.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

/// Query parameters for `GET /api/ai/sessions` (`?id=`) and
/// `DELETE /api/ai/sessions` (`?id=`).
#[derive(Debug, Deserialize)]
pub struct SessionIdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// `GET /api/ai/sessions` (list) or `?id=` (one full session).
pub async fn sessions_get(
    State(state): State<AppState>,
    Query(q): Query<SessionIdQuery>,
) -> Response {
    let ch = &state.clickhouse;
    if let Err(err) = ensure_chat_session_table(ch).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response();
    }
    if let Some(id) = q.id.filter(|s| !s.is_empty()) {
        return match session_detail(ch, &id).await {
            Ok(Some(session)) => {
                (StatusCode::OK, ApiJson(json!({ "session": session }))).into_response()
            }
            Ok(None) => (
                StatusCode::NOT_FOUND,
                ApiJson(json!({ "error": "sesi tidak ditemukan" })),
            )
                .into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(json!({ "error": err.to_string() })),
            )
                .into_response(),
        };
    }
    match session_list(ch, 50).await {
        Ok(sessions) => (StatusCode::OK, ApiJson(json!({ "sessions": sessions }))).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn session_list(
    ch: &ChClient,
    limit: u32,
) -> Result<Vec<Value>, lakehouse_clickhouse::ChError> {
    let limit = limit.clamp(1, 200);
    let rows = ch
        .rows(
            &format!(
                "SELECT id, title, mode, toString(updated_at) AS updated_at FROM console.chat_session FINAL \
                 WHERE is_deleted = 0 ORDER BY updated_at DESC LIMIT {limit}"
            ),
            None,
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get("id"), "title": r.get("title"), "mode": r.get("mode"),
                "updatedAt": r.get("updated_at"),
            })
        })
        .collect())
}

async fn session_detail(
    ch: &ChClient,
    id: &str,
) -> Result<Option<Value>, lakehouse_clickhouse::ChError> {
    let rows = ch
        .rows(
            &format!(
                "SELECT id, title, mode, messages_json, toString(updated_at) AS updated_at \
                 FROM console.chat_session FINAL WHERE is_deleted = 0 AND id='{}' LIMIT 1",
                esc(id)
            ),
            None,
        )
        .await?;
    let Some(row) = rows.first() else {
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

/// `POST /api/ai/sessions` — save/replace a session (id optional → new).
///
/// # Errors
///
/// 400 [`ApiError::BadRequest`] when `messages` is missing/empty; 500
/// [`ApiError::Internal`] on a `ClickHouse` failure.
pub async fn sessions_save(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: SaveSessionBody = serde_json::from_slice(&body).unwrap_or_default();
    let Some(messages) = parsed.messages.filter(|m| !m.is_empty()) else {
        return Err(ApiError::BadRequest("messages kosong".to_owned()).into());
    };
    let ch = &state.clickhouse;
    ensure_chat_session_table(ch)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    let id = parsed
        .id
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or_else(new_session_id);
    let first_user = messages
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"));
    let title_raw = first_user
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("Percakapan");
    let title = collapse_whitespace(&title_raw.chars().take(80).collect::<String>());
    let title = if title.is_empty() {
        "Percakapan".to_owned()
    } else {
        title
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
    let sql = format!(
        "INSERT INTO console.chat_session (id, title, mode, messages_json) VALUES ('{}', '{}', '{}', '{}')",
        esc(&id),
        esc(&title),
        esc(&mode),
        esc(&json_body),
    );
    ch.exec(&sql, None)
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

/// `s.slice(0,80).replace(/\s+/g, " ").trim()`.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `DELETE /api/ai/sessions?id=` — soft-delete a session.
///
/// # Errors
///
/// 400 [`ApiError::BadRequest`] when `id` is missing; 500
/// [`ApiError::Internal`] on a `ClickHouse` failure.
pub async fn sessions_delete(
    State(state): State<AppState>,
    Query(q): Query<SessionIdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = q.id.filter(|s| !s.is_empty()) else {
        return Err(ApiError::BadRequest("id wajib".to_owned()).into());
    };
    let ch = &state.clickhouse;
    ensure_chat_session_table(ch)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let sql = format!(
        "INSERT INTO console.chat_session (id, title, mode, messages_json, is_deleted) VALUES ('{}', '', 'ask', '[]', 1)",
        esc(&id)
    );
    ch.exec(&sql, None)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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
