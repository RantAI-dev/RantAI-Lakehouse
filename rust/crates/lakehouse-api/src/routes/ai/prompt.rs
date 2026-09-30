//! The copilot's system prompt and the per-turn tool selection.
//!
//! # Why the prompt was rewritten
//!
//! The previous prompt was mostly Indonesian, hardcoded to one tenant's
//! domain and region (`AGENTS.md` rule 12 forbids that in code), and never
//! explained the lakehouse layers, so "what data do we have?" came back in
//! source-tier words (`primer`/`sekunder`) instead of Bronze/Silver/Gold.
//! Measured with `ops/ai_eval/ai_eval.py` on the local stack it also:
//!
//! - answered English questions in Indonesian and Indonesian ones in
//!   English;
//! - did arithmetic in its head (a total off by exactly 1,000 went out
//!   bold), where the same number from SQL would have been right and
//!   checkable;
//! - answered "which connectors are registered?" as if the empty list
//!   were a failure to look.
//!
//! The rewrite is written for small models as much as large ones: short
//! numbered rules in plain English, one instruction per line, and the
//! decisions spelled out (compute in SQL; decline and say what exists when
//! the data does not cover a question) rather than left to judgement.
//!
//! # Why tools are selected per turn
//!
//! Every tool the principal may use used to be advertised on every turn:
//! 23 in ask mode, up to 47 in build mode. Tool-selection accuracy falls
//! steeply with the number of tools offered, most of all for small models
//! (the Berkeley function-calling leaderboard and later tool-retrieval
//! work report large drops past roughly 15-20 tools). [`select_tools`]
//! keeps the data tools and the read-only listings on every turn and adds
//! the rest by the domain the conversation mentions, so a typical data
//! question sees about a dozen tools. It narrows only what is offered:
//! [`super::gate`] still decides what may run.

/// The system prompt shared by both modes.
pub(super) const SYSTEM_BASE: &str = "\
You are the AI Copilot of RantAI Lakehouse, a data lakehouse console. You help people find, understand and analyse the data in the lakehouse, and operate the platform.

HOW THE LAKEHOUSE IS ORGANISED
- Bronze: raw data landed from source systems, stored as Iceberg tables. Every registered dataset starts here.
- Silver: cleaned, typed detail tables in the ClickHouse database `silver`.
- Gold: aggregated serving marts in the ClickHouse database `serving` (`serving.mart_*`). Dashboards and most answers read Gold.
- A dataset's \"primary source\" / \"secondary source\" label says where the data comes from. It is NOT a layer: never describe data as primer/sekunder layers.

ANSWERING QUESTIONS ABOUT DATA
1. Find the right table in the DATA MAP below. It lists every Gold and Silver table with its columns, what they mean, value ranges and the actual text values stored. Text values may be in another language than the question (a country may be stored in Indonesian): use the stored value in SQL.
2. A table has one row per combination of its grain columns (the DATA MAP gives each table's grain). A single row is NOT a total: for a year, a month, a country or any category, SUM the measure and GROUP BY exactly the columns asked about.
3. Get numbers with run_sql (a ClickHouse SELECT). Let SQL do all arithmetic: totals, counts, averages, percentages, shares, growth and rankings are computed in the query (round to 1-2 decimals). Never add, subtract or divide numbers yourself.
4. If run_sql returns an error, read it, fix the query and run it again.
5. Check coverage first. If the data does not contain what was asked (a metric that does not exist, a year or place outside the ranges in the DATA MAP), say plainly that the data does not include it, say what the data does cover, and give no estimate.
6. Answer every part of the question. Start with the direct answer and its number. Add a short table only when comparing several items. End with one short line naming the table you used.

ANSWERING QUESTIONS ABOUT THE PLATFORM
- For pipelines, connectors and ingest, Iceberg tables, storage capacity, lineage, dashboards, alerts, data quality, audit, CDC replication, maintenance and running queries, call the matching tool. Never answer these from memory.
- Name the specific items you report on (the pipeline, connector, slot, table or rule), not only their status.
- Which pipelines are failing or healthy: list_pipelines (each pipeline's latest status). get_build_status is only a log of the latest runs across all jobs.
- If a tool returns an empty list, the answer is that there are none (say so directly, e.g. \"There are no connectors registered.\"). If a tool fails, say it could not be checked and why, in one sentence.

RULES
- Only state numbers that appear in a tool result in this conversation.
- Be concise. Use Markdown: short paragraphs, bold for the key figure, tables for comparisons.";

/// The last line of the system prompt, after the DATA MAP: which language
/// to reply in.
///
/// A rule saying "reply in the user's language" was not enough. The DATA
/// MAP carries the catalog's own descriptions, often in Indonesian, and
/// measured on the local stack the model still answered 2 of 23 English
/// questions in Indonesian with that rule placed last. Naming the language
/// outright, from [`reply_language`], leaves nothing for the model to
/// infer, which matters most for small models.
pub(super) fn closing(latest_user_message: &str) -> String {
    match reply_language(latest_user_message) {
        Some(lang) => format!(
            "\n\nLANGUAGE: the user's latest message is in {lang}. Reply in {lang}, even though \
             the data, table names and DATA MAP may be in another language."
        ),
        None => "\n\nLANGUAGE: reply in the language of the user's latest message. The language \
                 of the data, table names or DATA MAP does not change this."
            .to_owned(),
    }
}

/// Everyday Indonesian function words: common in any Indonesian sentence,
/// rare in English ones.
const INDONESIAN_WORDS: &[&str] = &[
    "yang",
    "dan",
    "dengan",
    "adalah",
    "untuk",
    "dari",
    "pada",
    "tidak",
    "ini",
    "itu",
    "berapa",
    "apa",
    "mana",
    "siapa",
    "kapan",
    "bagaimana",
    "kenapa",
    "mengapa",
    "ada",
    "punya",
    "saya",
    "kita",
    "kami",
    "tolong",
    "buat",
    "buatkan",
    "tampilkan",
    "jumlah",
    "tahun",
    "bulan",
    "paling",
    "banyak",
    "di",
    "ke",
    "sudah",
    "belum",
    "bisa",
    "apakah",
];

/// Common English function words, for the same test the other way.
const ENGLISH_WORDS: &[&str] = &[
    "the", "and", "what", "which", "how", "many", "much", "is", "are", "was", "were", "do", "does",
    "did", "of", "in", "for", "to", "with", "show", "me", "give", "list", "create", "make", "have",
    "has", "any", "our", "we", "there",
];

/// `"Indonesian"` or `"English"` for `text`, or `None` when neither clearly
/// wins (a one-word message, a table name, another language). Words both
/// languages use ("data") are in neither list; an English message needs
/// two more English words than Indonesian ones to be read as English.
pub(super) fn reply_language(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let count = |list: &[&str]| words.iter().filter(|w| list.contains(&w.as_str())).count();
    let (id, en) = (count(INDONESIAN_WORDS), count(ENGLISH_WORDS));
    if id >= 2 && id > en {
        Some("Indonesian")
    } else if en >= 2 && en >= id + 2 {
        Some("English")
    } else {
        None
    }
}

/// Appended in ask mode.
pub(super) const SYSTEM_ASK_SUFFIX: &str = "

MODE: ASK (read-only). You can read and analyse anything, but you cannot change anything: no creating or deleting charts, pipelines, connectors or rules. If the user asks for a change, tell them to switch to Build mode.";

/// Appended in build mode.
pub(super) const SYSTEM_BUILD_SUFFIX: &str = "

MODE: BUILD. Besides answering, you can operate the lakehouse:
- Charts and dashboards: call describe_mart first, then create_chart using only columns that exist. For a request without details, call suggest_dashboard. To group charts, create_board first, then create_chart with board=<id>. To change a chart, update_chart with every field. For data that combines several marts, list_sql_sources gives the saved SQL sources; pass one as sqlSource instead of mart.
- Alerts and digests: list_alert_rules to see existing rules; create_alert_rule / update_alert_rule (alert: mart, measure, agg, op, threshold; digest: board); run_alert_rule sends the webhook or email for real.
- Connectors: list_connectors, create_connector (credentials are only ever a reference the server derives, never a real secret), test_connector for a real connection test.
- Ingest into Bronze: get_ingest_spec to see what a connector ingests, discover_source to list the source's tables, set_ingest_spec to choose tables and their Bronze targets, run_ingest to run it now, list_ingest_runs for results. rotate_connector_credential points a connector at a new credential.
- Pipelines: list_pipelines / get_pipeline / list_pipeline_runs for status; create_pipeline to author one (saved as draft), mark_pipeline_ready to make it runnable; trigger_pipeline, retry_pipeline_run, resume_pipeline to run; trigger_lakehouse_build rebuilds every layer from what the deployment has (explain the plan in one line first, then report what was launched and what was skipped).
- Iceberg tables: list_iceberg_tables, describe_iceberg_table, get_table_maintenance, set_table_maintenance. Storage: get_capacity.
- Saved queries: save_query, list_saved_queries, run_saved_query.
- Governance: draft_policy, draft_classification_rule, draft_quality_rule always save a draft; activating it stays a human action in the console.
- Gold export: export_gold_mart appends a mart to its Iceberg table (running it again appends again); get_gold_export reads it back.
- These need human approval and do not run immediately: delete_alert_rule, delete_connector, rotate_connector_credential, pause_pipeline, cancel_pipeline_run, delete_chart, run_bronze_maintenance, kill_query. Tell the user the request is waiting in Approvals (/agents/approvals).
- needs_confirmation and needs_approval are different. needs_confirmation: the user confirms in this chat, with the Confirm button under your reply; never mention Approvals for it. needs_approval: a human must approve it in Approvals (/agents/approvals).
- When a tool result says needs_confirmation, reply with ONE short sentence such as \"The chart draft is ready — review the preview below and confirm.\" Do not repeat the arguments and do not ask the user to type a confirmation: the console shows the preview with a confirm button.";

/// Tools offered on every turn: the data tools, and the read-only listings
/// a general question about the platform needs.
const ALWAYS: &[&str] = &[
    "run_sql",
    "lakehouse_overview",
    "list_datasets",
    "describe_dataset",
    "describe_mart",
    "get_lineage",
    "get_quality",
    "list_pipelines",
    "list_connectors",
    "list_boards",
    "list_alert_rules",
    "get_cdc_health",
];

/// One domain of tools and the words (English and Indonesian, lowercase
/// substrings) that bring it into a turn.
struct Domain {
    words: &'static [&'static str],
    tools: &'static [&'static str],
}

const DOMAINS: &[Domain] = &[
    Domain {
        words: &[
            "pipeline", "job", "dagster", "schedul", "jadwal", "ingest", "refresh", "rebuild",
            "build", "fail", "gagal", "run ", "runs", "jalankan", "retry", "pause", "resume",
        ],
        tools: &[
            "list_pipeline_runs",
            "get_build_status",
            "trigger_pipeline",
            "retry_pipeline_run",
            "pause_pipeline",
            "resume_pipeline",
            "cancel_pipeline_run",
            "trigger_lakehouse_build",
        ],
    },
    Domain {
        words: &[
            "connector",
            "konektor",
            "connection",
            "koneksi",
            "source system",
            "sumber data",
        ],
        tools: &["create_connector", "test_connector", "delete_connector"],
    },
    Domain {
        words: &[
            "dashboard",
            "chart",
            "grafik",
            "diagram",
            "board",
            "visual",
            "plot",
            "kpi",
            "card",
            "kartu",
            "tile",
            "sql source",
            "sumber sql",
        ],
        tools: &[
            "list_charts",
            "list_sql_sources",
            "suggest_dashboard",
            "create_chart",
            "update_chart",
            "create_board",
            "delete_chart",
        ],
    },
    Domain {
        words: &[
            "alert",
            "notif",
            "threshold",
            "digest",
            "peringatan",
            "ambang",
            "webhook",
            "email",
        ],
        tools: &[
            "create_alert_rule",
            "update_alert_rule",
            "delete_alert_rule",
            "run_alert_rule",
        ],
    },
    Domain {
        words: &["saved", "simpan", "tersimpan", "save "],
        tools: &["save_query", "list_saved_queries", "run_saved_query"],
    },
    Domain {
        words: &[
            "quality", "kualitas", "valid", "complete", "duplicat", "null", "check", "cek",
        ],
        tools: &["list_quality_rules", "draft_quality_rule"],
    },
    Domain {
        words: &[
            "audit",
            "history",
            "riwayat",
            "who ",
            "siapa",
            "policy",
            "policies",
            "kebijakan",
            "classif",
            "klasifikasi",
            "mask",
            "pii",
            "sensitive",
            "sensitif",
            "govern",
            "access",
            "akses",
        ],
        tools: &[
            "get_audit_history",
            "list_classification_rules",
            "draft_policy",
            "draft_classification_rule",
        ],
    },
    Domain {
        words: &[
            "maintenance",
            "orphan",
            "snapshot",
            "compaction",
            "iceberg",
            "perawatan",
        ],
        tools: &["get_maintenance_metrics", "run_bronze_maintenance"],
    },
    Domain {
        words: &[
            "workload",
            "running quer",
            "kill",
            "slow",
            "lambat",
            "berjalan",
        ],
        tools: &["list_workloads", "kill_query"],
    },
    Domain {
        words: &["export", "ekspor"],
        tools: &["export_gold_mart", "get_gold_export"],
    },
    Domain {
        words: &[
            "pipeline",
            "transform",
            "buat pipeline",
            "create a pipeline",
        ],
        tools: &["get_pipeline", "create_pipeline", "mark_pipeline_ready"],
    },
    Domain {
        words: &[
            "ingest",
            "discover",
            "tarik",
            "sync",
            "load",
            "credential",
            "kredensial",
            "password",
            "secret",
            "rotate",
            "rotasi",
            "connector",
            "konektor",
            "source table",
            "sumber",
        ],
        tools: &[
            "get_ingest_spec",
            "set_ingest_spec",
            "discover_source",
            "run_ingest",
            "list_ingest_runs",
            "rotate_connector_credential",
        ],
    },
    Domain {
        words: &[
            "iceberg",
            "snapshot",
            "partition",
            "namespace",
            "warehouse",
            "lakekeeper",
            "bronze",
            "maintenance",
            "compaction",
            "orphan",
            "perawatan",
        ],
        tools: &[
            "list_iceberg_tables",
            "describe_iceberg_table",
            "get_table_maintenance",
            "set_table_maintenance",
        ],
    },
    Domain {
        words: &[
            "capacity",
            "kapasitas",
            "storage",
            "penyimpanan",
            "disk",
            "bucket",
            "size",
            "ukuran",
        ],
        tools: &["get_capacity"],
    },
];

/// The tool names worth offering for a turn whose recent user messages are
/// `recent` (latest last). The data tools and read-only listings in
/// [`ALWAYS`] are always included; a domain's other tools are added when
/// any of its words appears in the latest two user messages, so a
/// follow-up ("and pause it") keeps the previous turn's domain.
pub(super) fn select_tools(recent: &[&str]) -> std::collections::HashSet<&'static str> {
    let text = recent
        .iter()
        .rev()
        .take(2)
        .map(|m| m.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out: std::collections::HashSet<&'static str> = ALWAYS.iter().copied().collect();
    for domain in DOMAINS {
        if domain.words.iter().any(|w| text.contains(w)) {
            out.extend(domain.tools.iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_question_gets_only_the_data_tools_and_listings() {
        let tools = select_tools(&["How many foreign tourists came in 2024?"]);
        assert_eq!(tools.len(), ALWAYS.len());
        assert!(tools.contains("run_sql"));
        assert!(!tools.contains("create_chart"));
    }

    #[test]
    fn a_domain_word_brings_in_that_domain_only() {
        let tools = select_tools(&["Make a bar chart of events per year"]);
        assert!(tools.contains("create_chart"));
        assert!(!tools.contains("kill_query"));
        let tools = select_tools(&["Kenapa pipeline gold gagal?"]);
        assert!(tools.contains("list_pipeline_runs"));
    }

    #[test]
    fn a_chart_request_can_reach_the_sql_sources() {
        // Merge of main's per-domain tool selection with SQL sources: a
        // tool missing from every domain is never offered, so chart-from-
        // SQL-source requests would have silently lost list_sql_sources.
        let tools = select_tools(&["Buat grafik dari sumber sql material"]);
        assert!(tools.contains("list_sql_sources"));
        assert!(tools.contains("create_chart"));
    }

    #[test]
    fn a_follow_up_keeps_the_previous_turns_domain() {
        let tools = select_tools(&["Which pipelines failed?", "ok, retry the second one"]);
        assert!(tools.contains("retry_pipeline_run"));
    }

    #[test]
    fn every_selectable_tool_is_a_registered_tool() {
        for name in ALWAYS
            .iter()
            .chain(DOMAINS.iter().flat_map(|d| d.tools.iter()))
        {
            assert!(
                super::super::registry::find(name).is_some(),
                "{name} is not in the tool registry"
            );
        }
    }

    #[test]
    fn every_registered_tool_is_reachable_from_some_domain() {
        for spec in super::super::registry::TOOLS {
            let reachable =
                ALWAYS.contains(&spec.name) || DOMAINS.iter().any(|d| d.tools.contains(&spec.name));
            assert!(reachable, "{} can never be offered to the model", spec.name);
        }
    }

    #[test]
    fn the_prompt_explains_the_layers_and_the_source_label() {
        for layer in ["Bronze", "Silver", "Gold"] {
            assert!(SYSTEM_BASE.contains(layer));
        }
        assert!(SYSTEM_BASE.contains("It is NOT a layer"));
    }

    #[test]
    fn the_reply_language_is_read_from_the_latest_message() {
        assert_eq!(
            reply_language("How many foreign tourist visits were recorded in 2024?"),
            Some("English")
        );
        assert_eq!(
            reply_language("Which data source connectors are registered?"),
            Some("English")
        );
        assert_eq!(
            reply_language(
                "Provinsi mana yang punya usaha kuliner terdaftar paling banyak, dan berapa jumlahnya?"
            ),
            Some("Indonesian")
        );
        assert_eq!(
            reply_language("Berapa jumlah data di tabel ini?"),
            Some("Indonesian")
        );
        assert_eq!(reply_language("mart_wisman"), None);
        assert!(closing("How many rows are there?").contains("Reply in English"));
        assert!(closing("ok").contains("language of the user's latest message"));
    }
}
