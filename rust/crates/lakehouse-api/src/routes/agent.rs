//! `POST /api/agent/ask`, `POST /api/agent/query`, `POST /api/agent/text-to-sql`
//! — the three LLM-backed agent endpoints.
//!
//! Ports `src/app/api/agent/ask/route.ts`, `src/app/api/agent/query/route.ts`,
//! `src/app/api/agent/text-to-sql/route.ts`, and the schema-grounding /
//! `SQL`-extraction helpers from `src/services/clients/agent-tools.ts`. Model
//! *text* is inherently non-deterministic and is not chased for byte parity
//! (see `rust/tests/parity/README.md`); the request/response *structure*,
//! validation, and guard behavior are ported faithfully.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use lakehouse_llm::{ChatMessage, ChatOptions, ChatRole};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// `{question}` request body shared by all three routes.
#[derive(Debug, Default, Deserialize)]
struct QuestionBody {
    #[serde(default)]
    question: Option<String>,
}

fn user_msg(content: String) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content,
    }
}

fn system_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: ChatRole::System,
        content: content.to_owned(),
    }
}

// ── shared: schema grounding ────────────────────────────────────────────

/// `schemaContext()` in `agent-tools.ts` (and duplicated verbatim in
/// `text-to-sql/route.ts`): a plain-text description of every `serving.mart_*`
/// table's columns, plus the list of `silver.*` table names, so the LLM
/// never invents a table/column that doesn't exist.
pub(crate) async fn schema_context(ch: &ChClient) -> Result<String, lakehouse_clickhouse::ChError> {
    let mart_rows = ch.rows("SHOW TABLES FROM serving", None).await?;
    let mart_tables: Vec<String> = mart_rows
        .iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str))
        .filter(|n| n.starts_with("mart_") && !n.ends_with("_baru"))
        .map(ToOwned::to_owned)
        .collect();

    let mut mart_descs = Vec::with_capacity(mart_tables.len());
    for t in &mart_tables {
        let cols = ch.rows(&format!("DESCRIBE serving.`{t}`"), None).await?;
        let cols_desc = cols
            .iter()
            .filter_map(|c| {
                let name = c.get("name").and_then(Value::as_str)?;
                if name.starts_with('_') {
                    return None;
                }
                let ty = c.get("type").and_then(Value::as_str).unwrap_or("");
                Some(format!("{name} {ty}"))
            })
            .collect::<Vec<_>>()
            .join(", ");
        mart_descs.push(format!("serving.{t}({cols_desc})"));
    }

    let silver_rows = ch.rows("SHOW TABLES FROM silver", None).await?;
    let silver: Vec<String> = silver_rows
        .iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str))
        .take(60)
        .map(ToOwned::to_owned)
        .collect();

    Ok(format!(
        "TABEL MART (Gold, utama untuk agregasi):\n{}\n\nTABEL SILVER (detail per dataset, akses: silver.`<nama>`):\n{}",
        mart_descs.join("\n"),
        silver.join(", "),
    ))
}

/// Extracted `{sql, explanation, assumptions}` from an LLM's JSON reply.
struct SqlAnswer {
    sql: String,
    explanation: String,
    assumptions: Vec<String>,
}

/// `extractSqlJson`: pull a ` ```json {...}``` ` fenced block, or the first
/// bare `{...}` object, out of `text` and parse it as `{sql, explanation?,
/// assumptions?}`. Returns `None` if nothing parses or `sql` isn't a string.
fn extract_sql_json(text: &str) -> Option<SqlAnswer> {
    let raw = extract_fenced_or_bare_json(text)?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let sql = parsed.get("sql")?.as_str()?.trim().to_owned();
    let explanation = parsed
        .get("explanation")
        .map(js_stringish)
        .unwrap_or_default();
    let assumptions = parsed
        .get("assumptions")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(js_stringish).collect())
        .unwrap_or_default();
    Some(SqlAnswer {
        sql,
        explanation,
        assumptions,
    })
}

/// `String(v)` for a `serde_json::Value` that came from an LLM's JSON
/// reply (only ever a string/number/bool/null in practice).
fn js_stringish(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// ` ```(?:json)?\s*(\{[\s\S]*?\})\s*``` ` fenced block first, else the
/// first bare `{...}` object in the text.
fn extract_fenced_or_bare_json(text: &str) -> Option<String> {
    if let Some(fence_start) = text.find("```") {
        let after_fence = &text[fence_start + 3..];
        let after_lang = after_fence.strip_prefix("json").unwrap_or(after_fence);
        if let Some(obj_start) = after_lang.find('{') {
            let rest = &after_lang[obj_start..];
            if let Some(obj) = extract_first_brace_object(rest) {
                return Some(obj);
            }
        }
    }
    extract_first_brace_object(text)
}

/// The first balanced `{...}` substring in `text` (brace-depth counting;
/// ignores braces inside quoted strings).
fn extract_first_brace_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..=i].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// `/^\s*(with|select)\b/i.test(sql) && !/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)`
/// — `isReadOnlySql` in `agent-tools.ts`, used by `agent/query`. Distinct
/// from `query/run`'s guard (`routes::query::is_read_only`): only
/// `with`/`select` are allowed here (no `show`/`describe`/`explain`).
pub(crate) fn is_read_only_sql(sql: &str) -> bool {
    starts_with_with_or_select(sql) && !contains_denied_keyword_full(sql)
}

fn starts_with_with_or_select(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    ["with", "select"].iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    })
}

fn contains_denied_keyword_full(sql: &str) -> bool {
    const DENIED: [&str; 12] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename", "attach",
        "detach", "grant", "revoke",
    ];
    word_occurs_any(sql, &DENIED)
}

/// `/^\s*(with|select)\b/i.test(sql) || /\b(insert|alter|drop|delete|update|create|truncate)\b/i.test(sql)`
/// — the *narrower* guard `text-to-sql/route.ts` hand-rolls inline (only 7
/// denied keywords, missing `rename`/`attach`/`detach`/`grant`/`revoke`
/// compared to `agent-tools.ts`'s `isReadOnlySql`). Reproduced as its own
/// distinct function rather than reusing [`is_read_only_sql`]: this
/// narrower list is a genuine TypeScript inconsistency across the two
/// routes, not a mistake to "fix" here.
fn is_read_only_sql_text_to_sql(sql: &str) -> bool {
    const DENIED: [&str; 7] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate",
    ];
    starts_with_with_or_select(sql) && !word_occurs_any(sql, &DENIED)
}

fn word_occurs_any(sql: &str, denied: &[&str]) -> bool {
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    denied.iter().any(|kw| word_occurs(&chars, kw))
}

fn word_occurs(chars: &[char], word: &str) -> bool {
    let word_chars: Vec<char> = word.chars().collect();
    let n = word_chars.len();
    if n == 0 || chars.len() < n {
        return false;
    }
    for start in 0..=(chars.len() - n) {
        if chars[start..start + n] == word_chars[..] {
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let after_ok = start + n == chars.len() || !is_word_char(chars[start + n]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn parse_question_body(body: &Bytes) -> Result<QuestionBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("Body harus JSON {question}".to_owned()))
}

// ── POST /api/agent/ask ─────────────────────────────────────────────────

/// A catalog hit: one dataset matched against the question's keywords.
struct CatalogHit {
    slug: String,
    title: String,
    description: String,
    tier: String,
    total: i64,
}

/// `searchCatalog`: score every catalog row by how many of the question's
/// lowercased alphanumeric "words" (len > 2) it contains, keep the top 12
/// by score (ties broken by original row order — `Array.prototype.sort` is
/// stable), score > 0 only.
async fn search_catalog(
    ch: &ChClient,
    question: &str,
) -> Result<Vec<CatalogHit>, lakehouse_clickhouse::ChError> {
    let rows = ch
        .rows(
            "SELECT c.slug slug, c.title title, c.description description, c.tier tier, \
             toString(coalesce(s.total,0)) total FROM ( \
               SELECT slug,title,description,tier FROM lake.`bronze_meta.dataset_catalog` \
               UNION ALL SELECT slug,title,description,tier FROM lake.`bronze_meta_sec.dataset_catalog` \
             ) c LEFT JOIN ( \
               SELECT slug,total FROM lake.`bronze_meta.dataset_sync` \
               UNION ALL SELECT slug,total FROM lake.`bronze_meta_sec.dataset_sync` \
             ) s ON c.slug = s.slug",
            None,
        )
        .await?;

    let terms: Vec<String> = question
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(ToOwned::to_owned)
        .collect();

    let mut scored: Vec<(i64, &serde_json::Map<String, Value>)> = rows
        .iter()
        .map(|r| {
            let title = r.get("title").and_then(Value::as_str).unwrap_or("");
            let description = r.get("description").and_then(Value::as_str).unwrap_or("");
            let slug = r.get("slug").and_then(Value::as_str).unwrap_or("");
            let hay = format!("{title} {description} {slug}").to_lowercase();
            let score = i64::try_from(terms.iter().filter(|t| hay.contains(t.as_str())).count())
                .unwrap_or(i64::MAX);
            (score, r)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    scored.truncate(12);

    Ok(scored
        .into_iter()
        .map(|(_, r)| CatalogHit {
            slug: r
                .get("slug")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            title: r
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            description: r
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            tier: r
                .get("tier")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            total: r
                .get("total")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        })
        .collect())
}

/// `POST /api/agent/ask` — catalog Q&A (RAG-lite): find relevant datasets,
/// then let the LLM summarize an answer grounded in only those datasets.
///
/// # Errors
///
/// 400 on an unparseable body or a missing/blank `question`; 503 if the
/// catalog search itself fails (the LLM failing does NOT error — it falls
/// back to a plain retrieval listing, matching the `TypeScript`).
pub async fn ask(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_question_body(&body)?;
    let question = match parsed.question {
        Some(q) if !q.trim().is_empty() => q,
        _ => return Err(ApiError::BadRequest("question wajib".to_owned()).into()),
    };

    let hits = search_catalog(&state.clickhouse, &question)
        .await
        .map_err(|err| ApiError::Unprocessable(format!("Gagal cari katalog: Error: {err}")))?;

    let context = if hits.is_empty() {
        "(tidak ada dataset yang cocok)".to_owned()
    } else {
        hits.iter()
            .map(|h| {
                format!(
                    "- {} ({}, {} baris) — {}",
                    h.title, h.tier, h.total, h.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let system = "Kamu asisten katalog data lakehouse pariwisata DKI. Jawab RINGKAS dalam Bahasa \
                  Indonesia berdasarkan HANYA daftar dataset yang diberikan. Sebutkan dataset yang \
                  relevan + jumlah barisnya. Jangan mengarang dataset di luar daftar.";
    let messages = [
        system_msg(system),
        user_msg(format!(
            "DATASET RELEVAN:\n{context}\n\nPERTANYAAN: {question}"
        )),
    ];
    let answer = match state
        .llm
        .chat(
            &messages,
            ChatOptions {
                temperature: Some(0.2),
                max_tokens: None,
            },
        )
        .await
    {
        Ok(a) => a,
        // LLM down: return the retrieval result honestly, without a summary.
        Err(err) => format!("Agent LLM tak tersedia ({err}). Dataset yang cocok:\n{context}"),
    };

    Ok(ApiJson(json!({
        "answer": answer,
        "datasets": hits.iter().map(|h| json!({
            "id": h.slug, "title": h.title, "tier": h.tier, "rows": h.total,
        })).collect::<Vec<_>>(),
    })))
}

// ── POST /api/agent/query ───────────────────────────────────────────────

const GEN_SYSTEM: &str = "Kamu ahli SQL ClickHouse untuk lakehouse pariwisata DKI Jakarta.\nUbah pertanyaan jadi SATU query SELECT ClickHouse valid.\nAturan:\n- HANYA gunakan tabel/kolom dari skema yang diberikan. Jangan mengarang.\n- Utamakan serving.mart_* untuk agregasi.\n- SELECT saja (baca). LIMIT wajar (<=100).\n- Balas HANYA JSON: {\"sql\":\"...\",\"explanation\":\"...\",\"assumptions\":[\"...\"]}";

const FIX_SYSTEM: &str = "Query ClickHouse gagal. Perbaiki SQL berdasarkan pesan error\ndan skema. Balas HANYA JSON: {\"sql\":\"...\",\"explanation\":\"...\",\"assumptions\":[]}";

const MAX_FIX: u32 = 2;

/// `POST /api/agent/query` — self-correcting NL-to-SQL: generate → run →
/// on failure, feed the error back to the LLM and retry (up to
/// [`MAX_FIX`] times) → summarize the final result in natural language.
///
/// # Errors
///
/// 400 on an unparseable body or missing/blank `question`; 503 if reading
/// the schema fails, or if the LLM is unreachable for the initial
/// generation; 422 if the LLM never produces valid SQL, or if every
/// attempt (including corrections) fails to execute.
#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single TS handler's generate -> \
              run -> self-correct -> summarize pipeline; splitting it up \
              would scatter one sequential loop across helpers with no \
              independent reuse"
)]
pub async fn query(State(state): State<AppState>, body: Bytes) -> Response {
    let parsed = match parse_question_body(&body) {
        Ok(p) => p,
        Err(err) => return err_response(err),
    };
    let question = match parsed.question {
        Some(q) if !q.trim().is_empty() => q,
        _ => return err_response(ApiError::BadRequest("question wajib".to_owned())),
    };

    let mut steps = vec![
        json!({ "step": "skema", "detail": "Membaca skema mart Gold + Silver dari lakehouse." }),
    ];
    let schema = match schema_context(&state.clickhouse).await {
        Ok(s) => s,
        Err(err) => {
            return err_response(ApiError::Unprocessable(format!(
                "Gagal baca skema: Error: {err}"
            )));
        }
    };

    let gen_messages = [
        system_msg(GEN_SYSTEM),
        user_msg(format!("SKEMA:\n{schema}\n\nPERTANYAAN: {question}")),
    ];
    let gen_out = match state
        .llm
        .chat(
            &gen_messages,
            ChatOptions {
                temperature: Some(0.0),
                max_tokens: None,
            },
        )
        .await
    {
        Ok(out) => out,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({
                    "error": "Agent LLM tak tersedia",
                    "detail": err.to_string(),
                    "hint": "Set LLM_KEY (MiniMax) di .env.local.",
                })),
            )
                .into_response();
        }
    };
    let Some(gen_res) = extract_sql_json(&gen_out) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            ApiJson(json!({ "error": "LLM tak menghasilkan SQL valid" })),
        )
            .into_response();
    };
    steps.push(json!({ "step": "generate", "detail": gen_res.sql }));

    let mut sql = gen_res.sql.clone();
    let mut result: Option<lakehouse_clickhouse::ChResult> = None;
    let mut last_error = String::new();

    for attempt in 0..=MAX_FIX {
        if !is_read_only_sql(&sql) {
            "SQL ditolak (hanya SELECT diizinkan).".clone_into(&mut last_error);
            break;
        }
        match state.clickhouse.query(&sql, None).await {
            Ok(r) => {
                let step = if attempt == 0 {
                    "jalankan".to_owned()
                } else {
                    format!("koreksi-{attempt}")
                };
                steps.push(json!({ "step": step, "detail": format!("OK, {} baris", r.rows) }));
                result = Some(r);
                break;
            }
            Err(err) => {
                last_error = err.to_string();
                let truncated: String = last_error.chars().take(160).collect();
                steps.push(json!({ "step": format!("error-{attempt}"), "detail": truncated }));
                if attempt == MAX_FIX {
                    break;
                }
                let fix_messages = [
                    system_msg(FIX_SYSTEM),
                    user_msg(format!(
                        "SKEMA:\n{schema}\n\nPERTANYAAN: {question}\n\nSQL GAGAL:\n{sql}\n\nERROR:\n{last_error}"
                    )),
                ];
                let Ok(fix_out) = state
                    .llm
                    .chat(
                        &fix_messages,
                        ChatOptions {
                            temperature: Some(0.0),
                            max_tokens: None,
                        },
                    )
                    .await
                else {
                    break;
                };
                let Some(fixed) = extract_sql_json(&fix_out) else {
                    break;
                };
                sql = fixed.sql;
                steps.push(json!({ "step": format!("perbaiki-{}", attempt + 1), "detail": sql }));
            }
        }
    }

    let Some(result) = result else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            ApiJson(json!({
                "sql": sql,
                "error": "Query gagal setelah koreksi",
                "detail": last_error,
                "steps": steps,
                "explanation": gen_res.explanation,
            })),
        )
            .into_response();
    };

    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    let rows: Vec<Value> = result
        .data
        .iter()
        .take(100)
        .map(|r| Value::Object(r.clone()))
        .collect();

    let mut answer = gen_res.explanation.clone();
    let preview =
        serde_json::to_string(&rows.iter().take(15).collect::<Vec<_>>()).unwrap_or_default();
    let summarize_messages = [
        system_msg(
            "Ringkas jawaban dari hasil query dalam Bahasa Indonesia, 1-3 kalimat, berdasarkan HANYA \
             data yang diberikan. Sebut angka kuncinya. Jangan mengarang.",
        ),
        user_msg(format!(
            "PERTANYAAN: {question}\nKOLOM: {}\nDATA(≤15): {preview}",
            columns.join(", ")
        )),
    ];
    if let Ok(summarized) = state
        .llm
        .chat(
            &summarize_messages,
            ChatOptions {
                temperature: Some(0.2),
                max_tokens: None,
            },
        )
        .await
    {
        answer = summarized;
    }

    (
        StatusCode::OK,
        ApiJson(json!({
            "question": question,
            "sql": sql,
            "columns": columns,
            "rows": rows,
            "rowCount": result.rows,
            "answer": answer,
            "assumptions": gen_res.assumptions,
            "steps": steps,
        })),
    )
        .into_response()
}

fn err_response(err: ApiError) -> Response {
    crate::error::ApiRejection(err).into_response()
}

// ── POST /api/agent/text-to-sql ─────────────────────────────────────────

const TEXT_TO_SQL_SYSTEM: &str = "Kamu ahli SQL ClickHouse untuk lakehouse pariwisata DKI Jakarta.\nUbah pertanyaan pengguna jadi SATU query SELECT ClickHouse yang valid.\nAturan:\n- HANYA gunakan tabel/kolom dari skema yang diberikan. Jangan mengarang.\n- Utamakan tabel serving.mart_* untuk agregasi.\n- Selalu SELECT (baca saja). Jangan INSERT/ALTER/DROP.\n- Batasi hasil dengan LIMIT wajar (<=100) kecuali diminta lain.\n- Balas HANYA JSON valid: {\"sql\": \"...\", \"explanation\": \"...\", \"assumptions\": [\"...\"]}";

/// `{question, run?}` — the `POST /api/agent/text-to-sql` body shape.
#[derive(Debug, Default, Deserialize)]
struct TextToSqlBody {
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    run: bool,
}

/// `POST /api/agent/text-to-sql` — NL question → grounded `SQL`, optionally
/// executed.
///
/// # Errors
///
/// 400 on an unparseable body or missing/blank `question`; 503 if reading
/// the schema fails, or the LLM is unreachable/doesn't return valid `SQL`
/// JSON; 422 if the generated `SQL` fails the read-only guard (a failure to
/// *execute* the generated `SQL`, when `run: true`, is reported inline as
/// `runError` at 200 instead — matching the `TypeScript`).
pub async fn text_to_sql(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed: TextToSqlBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest("Body harus JSON {question, run?}".to_owned()))?;
    let question = match parsed.question {
        Some(q) if !q.trim().is_empty() => q,
        _ => return Err(ApiError::BadRequest("question wajib".to_owned()).into()),
    };

    let schema = schema_context(&state.clickhouse)
        .await
        .map_err(|err| ApiError::Unprocessable(format!("Gagal baca skema: Error: {err}")))?;

    let messages = [
        system_msg(TEXT_TO_SQL_SYSTEM),
        user_msg(format!("SKEMA:\n{schema}\n\nPERTANYAAN: {question}")),
    ];
    let content = state
        .llm
        .chat(&messages, ChatOptions { temperature: Some(0.0), max_tokens: None })
        .await
        .map_err(|err| {
            ApiError::Unprocessable(format!(
                "Agent LLM tak tersedia: detail={err} hint=Set env LLM_URL/LLM_MODEL ke node yang aktif (llm-node)."
            ))
        })?;
    let Some(out) = extract_sql_json(&content) else {
        return Err(ApiError::Unprocessable(
            "Agent LLM tak tersedia: LLM tak mengembalikan JSON SQL yang valid.".to_owned(),
        )
        .into());
    };

    if !is_read_only_sql_text_to_sql(&out.sql) {
        return Ok(ApiJson(json!({
            "sql": out.sql,
            "explanation": out.explanation,
            "assumptions": out.assumptions,
            "error": "SQL ditolak (hanya SELECT diizinkan).",
        })));
    }

    let mut resp = json!({
        "sql": out.sql,
        "explanation": out.explanation,
        "assumptions": out.assumptions,
    });
    if parsed.run {
        match state.clickhouse.query(&out.sql, None).await {
            Ok(r) => {
                let columns: Vec<String> = r.meta.iter().map(|m| m.name.clone()).collect();
                let rows: Vec<Value> = r
                    .data
                    .iter()
                    .take(100)
                    .map(|row| Value::Object(row.clone()))
                    .collect();
                resp["columns"] = json!(columns);
                resp["rows"] = json!(rows);
                resp["rowCount"] = json!(r.rows);
            }
            Err(err) => resp["runError"] = json!(err.to_string()),
        }
    }
    Ok(ApiJson(resp))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn extract_sql_json_parses_fenced_block() {
        let text = "berikut jawabannya:\n```json\n{\"sql\":\"SELECT 1\",\"explanation\":\"e\",\"assumptions\":[\"a\"]}\n```\nsemoga membantu";
        let out = extract_sql_json(text).unwrap();
        assert_eq!(out.sql, "SELECT 1");
        assert_eq!(out.explanation, "e");
        assert_eq!(out.assumptions, vec!["a".to_owned()]);
    }

    #[test]
    fn extract_sql_json_parses_bare_object() {
        let text = "{\"sql\":\"SELECT 2\"}";
        let out = extract_sql_json(text).unwrap();
        assert_eq!(out.sql, "SELECT 2");
        assert_eq!(out.explanation, "");
        assert!(out.assumptions.is_empty());
    }

    #[test]
    fn extract_sql_json_none_when_no_sql_field() {
        assert!(extract_sql_json("{\"foo\":\"bar\"}").is_none());
    }

    #[test]
    fn extract_sql_json_none_for_non_json_text() {
        assert!(extract_sql_json("no json here").is_none());
    }

    #[test]
    fn extract_sql_json_trims_sql() {
        let out = extract_sql_json("{\"sql\":\"  SELECT 1  \"}").unwrap();
        assert_eq!(out.sql, "SELECT 1");
    }

    #[test]
    fn is_read_only_sql_allows_with_and_select_only() {
        assert!(is_read_only_sql("SELECT 1"));
        assert!(is_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(!is_read_only_sql("SHOW TABLES"));
        assert!(!is_read_only_sql("EXPLAIN SELECT 1"));
    }

    #[test]
    fn is_read_only_sql_rejects_full_dml_list() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename",
            "attach", "detach", "grant", "revoke",
        ] {
            assert!(!is_read_only_sql(&format!("SELECT 1; {kw} x")));
        }
    }

    #[test]
    fn text_to_sql_guard_is_narrower_than_agent_query_guard() {
        // `rename` is denied by agent/query's guard but NOT by
        // text-to-sql's narrower inline guard — a genuine TS
        // inconsistency, reproduced here.
        assert!(!is_read_only_sql("SELECT 1; RENAME TABLE a TO b"));
        assert!(is_read_only_sql_text_to_sql(
            "SELECT 1; RENAME TABLE a TO b"
        ));
    }

    #[test]
    fn text_to_sql_guard_rejects_its_own_seven_keywords() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate",
        ] {
            assert!(!is_read_only_sql_text_to_sql(&format!("SELECT 1; {kw} x")));
        }
    }
}
