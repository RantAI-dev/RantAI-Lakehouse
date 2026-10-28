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

use lakehouse_store::chat_term::{ChatTerm, MAX_MEANING_CHARS, MAX_TERM_CHARS};

use crate::config::ReplyLanguage;

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

/// Appended after the mode text when the chat may ask the person a question
/// (`AI_ASK_BACK` on and `ask_user` in the tool list). It is a separate block
/// and not part of [`SYSTEM_BASE`], so a deployment with the switch off
/// keeps the prompt it was measured with, byte for byte.
///
/// The rules turn an unclear word into one of three answers: use the one
/// reading there is and say which, ask once with options, or say the data
/// does not cover it (the coverage rule in [`SYSTEM_BASE`] already says
/// that last one).
pub(super) const ASK_BACK_RULES: &str = "

UNCLEAR WORDS
- If the DATA MAP or THIS USER'S WORDS settles what a word means, use that reading and say in your reply which reading you took.
- If a word fits two or more tables, columns or values and nothing settles it, call ask_user once. Make the options names taken from the DATA MAP. Run no query in that turn.
- Ask only once. If your previous message was a question, take the most likely reading, say which one you took, and answer.
- If the question is not about data, answer it as it is. Do not ask.";

/// The two sentences the console sends with the SQL help. `prepare_chat`
/// matches the Explain one to take the tool out of its turn (plan 17); the
/// test below holds the rules to the same words.
pub(super) const EXPLAIN_REQUEST: &str = "Explain what this query does.";
/// The Fix sentence is named only for the test that holds the rules to it:
/// no product code matches it, so it is not compiled outside tests.
#[cfg(test)]
pub(super) const FIX_REQUEST: &str = "Why does this query fail? Fix it.";

/// Appended after the mode text when `AI_SQL_HELP` is on and `propose_sql` is
/// in the chat's tool list. It is a separate block, so a chat without the
/// tool keeps the prompt it had, byte for byte.
///
/// The rules name no table, column or value: the statement, the engine and
/// the error come in the message, and the rules say only what to do with
/// them.
pub(super) const SQL_HELP_RULES: &str = "

SQL HELP
- A message that starts with \"Why does this query fail? Fix it.\" holds an engine, a statement and the error it raised. Find the cause, then call propose_sql once with the whole corrected statement for that engine and one sentence on what was wrong. Change only what the error requires.
- If propose_sql returns an error, correct the statement and call it again.
- If the statement cannot be fixed from what is known, say what is missing and call nothing.
- A message that starts with \"Explain what this query does.\" holds a statement. Say in plain words what it returns, step by step, and call no tool.
- Never call run_sql for either.";

/// Appended after the mode text (and after [`ASK_BACK_RULES`]) when
/// `AI_SEMANTIC_LAYER` is on, in both modes. It is a separate block and not
/// part of [`SYSTEM_BASE`], so a deployment with the switch off keeps the
/// prompt it was measured with, byte for byte.
///
/// Rule 2 of [`SYSTEM_BASE`] says to `SUM` the measure. A count of distinct
/// things in a table grouped by several columns is no measure: the same
/// thing can sit in more than one row, so a sum counts it more than once.
/// The rules name no table, column or value, because they apply to any
/// dataset; the marker they quote is the one `data_map` writes for a
/// `non_additive` column.
pub(super) const DISTINCT_COUNT_RULES: &str = "

COUNTS IN A GROUPED TABLE
- A count column in a table whose grain has several columns may count the same thing in more than one row, so adding it up counts that thing more than once. Amounts and quantities are added up as before.
- Never add up a column the DATA MAP marks [never SUM across rows], or a count you judge to overlap in this way. If a detail table in the DATA MAP has one row per thing or per line of it, count the distinct things there, with the same filters.
- If there is no such detail table, give the figure per row of the grain and say that a total cannot be read from this table.
- Say in one clause which table the count came from.";

/// Most remembered words carried in one prompt: the newest ones. A person
/// may keep more (`lakehouse_store::chat_term::MAX_TERMS_PER_OWNER`), but a
/// small model's context is better spent on the question.
pub(super) const MAX_USER_WORDS: usize = 30;

/// One line of text: control characters and every run of whitespace
/// (line breaks, tabs, U+2028) become a single space, so a stored value can
/// never start a new line or a new section of the prompt.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The caller's remembered words as a prompt section, the newest
/// [`MAX_USER_WORDS`] of `terms` (which the store lists newest first), one
/// line each: `- "<term>" means <meaning>`. Empty when there is no term, so
/// the section is then absent. Each value is flattened to one line
/// ([`one_line`]) and cut to the length the store enforces.
pub(super) fn user_words_section(terms: &[ChatTerm]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nTHIS USER'S WORDS\n");
    let lines: Vec<String> = terms
        .iter()
        .take(MAX_USER_WORDS)
        .map(|t| {
            let term: String = one_line(&t.term).chars().take(MAX_TERM_CHARS).collect();
            let meaning: String = one_line(&t.meaning)
                .chars()
                .take(MAX_MEANING_CHARS)
                .collect();
            format!("- \"{term}\" means {meaning}")
        })
        .collect();
    out.push_str(&lines.join("\n"));
    out
}

/// The last line of the system prompt, after the DATA MAP: which language
/// to reply in.
///
/// A rule saying "reply in the user's language" was not enough. The DATA
/// MAP carries the catalog's own descriptions, often in Indonesian, and
/// measured on the local stack the model still answered 2 of 23 English
/// questions in Indonesian with that rule placed last. Naming the language
/// outright, from [`reply_language`], leaves nothing for the model to
/// infer, which matters most for small models.
///
/// `default` is the deployment's `AI_DEFAULT_REPLY_LANGUAGE`. It only fills
/// the two cases where the message gives the model too little to go on: a
/// Latin-script message nothing could classify, and a message with no
/// letters. It never overrides a language [`reply_language`] detected, and
/// a message in another script keeps the plain line, since the script
/// already shows the language.
pub(super) fn closing(latest_user_message: &str, default: Option<ReplyLanguage>) -> String {
    match reply_language(latest_user_message) {
        Some(lang) => format!(
            "\n\nLANGUAGE: the user's latest message is in {lang}. Reply in {lang}, even though \
             the data, table names and DATA MAP may be in another language."
        ),
        // Undecided, but written in Latin letters: say so. With only
        // "reply in the user's language" to go on, a model whose own
        // default is another language answered "Explain the charts on
        // \"Main\"" in Chinese (QA, DeepSeek). Naming the script rules
        // that out without guessing which Latin-script language it is.
        None if is_latin_script(latest_user_message) => {
            let fallback = default.map_or("English", ReplyLanguage::name);
            format!(
                "\n\nLANGUAGE: reply in the language of the user's latest message. It is written in \
                 Latin script, so reply in that same language and script ({fallback} if you cannot \
                 tell), never in Chinese or any other script. The language of the data, table \
                 names or DATA MAP does not change this."
            )
        }
        None => {
            let too_short = match default {
                Some(lang) if !has_letters(latest_user_message) => {
                    format!(" If it is too short to tell, reply in {}.", lang.name())
                }
                _ => String::new(),
            };
            format!(
                "\n\nLANGUAGE: reply in the language of the user's latest message.{too_short} The \
                 language of the data, table names or DATA MAP does not change this."
            )
        }
    }
}

/// Whether `text` holds any letter at all. [`is_latin_script`] is `false`
/// both for a message with no letters (`2024?`) and for one in another
/// script, and only the first of those has nothing to show its language.
fn has_letters(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

/// Whether every letter in `text` is a Latin one (ASCII or accented), with
/// at least one letter present. A message in Chinese, Arabic or Cyrillic is
/// not, and keeps the plain "reply in the user's language" line.
fn is_latin_script(text: &str) -> bool {
    let mut letters = text.chars().filter(|c| c.is_alphabetic()).peekable();
    letters.peek().is_some()
        && letters.all(|c| c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c))
}

/// Everyday Indonesian function words: common in any Indonesian sentence,
/// rare in English ones.
pub(super) const INDONESIAN_WORDS: &[&str] = &[
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
pub(super) const ENGLISH_WORDS: &[&str] = &[
    "the",
    "and",
    "what",
    "which",
    "how",
    "many",
    "much",
    "is",
    "are",
    "was",
    "were",
    "do",
    "does",
    "did",
    "of",
    "in",
    "for",
    "to",
    "with",
    "show",
    "me",
    "give",
    "list",
    "create",
    "make",
    "have",
    "has",
    "any",
    "our",
    "we",
    "there",
    // Short prompts ("Explain the charts on Main", "Why did the job fail?")
    // had one word from the list above and were read as undecided.
    "a",
    "an",
    "on",
    "at",
    "by",
    "from",
    "this",
    "that",
    "it",
    "why",
    "where",
    "when",
    "who",
    "can",
    "explain",
    "summarize",
    "tell",
    "add",
    "about",
];

/// The lower-cased runs of letters in `text`, in order. The one tokeniser
/// for [`reply_language`] and for the DATA MAP's reading of a question
/// (`data_map::question_words`), so both see the same words.
pub(super) fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// `"Indonesian"` or `"English"` for `text`, or `None` when neither clearly
/// wins (a one-word message, a table name, another language). Words both
/// languages use ("data") are in neither list; an English message needs
/// two more English words than Indonesian ones to be read as English.
pub(super) fn reply_language(text: &str) -> Option<&'static str> {
    let words: Vec<String> = tokens(text).collect();
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

/// The sentences about query limits, Stop and how fresh a dashboard's results
/// are (`BI-5`, `BI-4`), shared by both modes' suffixes: `get_query_limits`,
/// `stop_query` and the `computedAt` read are offered in Ask mode too
/// (`BI-5` review S9). A macro and not a constant because `concat!` takes only
/// literals; [`SYSTEM_BASE`] stays the measured prompt it was.
macro_rules! limits_and_freshness {
    () => {
        " Charts and Query Studio run under time and row limits an administrator sets: for \"what are my query limits?\" or \"why did this chart time out?\" call get_query_limits and say the limit and where it comes from; you cannot change limits, an administrator does in Admin > Settings > Query limits. To \"stop the running query\", call stop_query (it ends only the person's own running queries). Dashboards remember the results of their charts; each tile result carries computedAt (when the data was computed) and remembered (true when it came from memory), so for \"how fresh is this dashboard?\" read them from the data and say the time."
    };
}

/// Appended in ask mode.
pub(super) const SYSTEM_ASK_SUFFIX: &str = concat!("

MODE: ASK (read-only). You can read and analyse anything, but you cannot change anything: no creating or deleting charts, pipelines, connectors or rules. You can still change the page's view filters with set_board_view_filters, since it changes only this person's view. If the user asks for a change, tell them to switch to Build mode.", limits_and_freshness!());

/// What the model is told when the person cannot change dashboards (`BI-18`
/// part C review fix (SHOULD-FIX) R30). The tools that write are then not in
/// its list because of the person's permissions, and without this line it
/// explained their absence as "not available in this session" or "not wired up
/// here", which is false (principle 2). `None` when the person may write, and
/// when nobody is signed in.
pub(super) fn dashboard_permission_note(
    perms: Option<&lakehouse_auth::PermissionSet>,
) -> Option<&'static str> {
    // Nobody signed in has no account to speak of: the prompt is unchanged.
    if perms.is_none_or(|p| p.has("dashboard:write")) {
        return None;
    }
    Some(
        "[Permissions: the signed-in person's account does not have the dashboard:write \
permission, so the tools that create or change charts, tabs and dashboards are withheld \
from you because of that permission, not because they are unavailable or not set up. \
When asked to change a dashboard, say: \"Your account cannot change dashboards; ask an \
editor or an administrator.\" Reading dashboards and charts still works.]",
    )
}

/// The system prompt with [`dashboard_permission_note`] before its closing
/// language line (which stays last), when the note applies (R30).
pub(super) fn with_permission_note(
    sys: String,
    perms: Option<&lakehouse_auth::PermissionSet>,
    closing: &str,
) -> String {
    let Some(note) = dashboard_permission_note(perms) else {
        return sys;
    };
    match sys.strip_suffix(closing) {
        Some(head) => format!("{head}\n\n{note}{closing}"),
        None => format!("{sys}\n\n{note}"),
    }
}

/// The line added to the newest user message, naming the mode for that
/// message. The history still holds the model's own earlier statements about
/// the mode (for example "I am in ASK mode" after a switch to Build), and the
/// model follows those, so the newest message says which mode applies now.
pub(super) fn mode_note(is_build: bool) -> &'static str {
    if is_build {
        "[Mode for this message: BUILD. Earlier messages about the mode are out of date.]"
    } else {
        "[Mode for this message: ASK. Earlier messages about the mode are out of date.]"
    }
}

/// Appended in build mode.
pub(super) const SYSTEM_BUILD_SUFFIX: &str = concat!("

MODE: BUILD. Besides answering, you can operate the lakehouse:
- Charts and dashboards: call describe_mart first, then create_chart using only columns that exist. For a request without details, call suggest_dashboard. To group charts, create_board first, then create_chart with board=<id>. To split a dashboard into tabs (list_boards shows each dashboard's tabs): on one with no tabs the first create_tab makes the tab that holds every chart already on it, so create a second tab as well, then move_charts_to_tab with the ids from list_charts for the charts the user named (a request such as \"put the map charts in a separate tab\" is create_tab for the rest, create_tab \"Maps\", move_charts_to_tab); rename_tab changes a title; move_tab changes the order of the tabs (give every tab in the new order, or one tab and its position, 1 first); the built-in dashboard has no tabs, so duplicate it first. To change a chart, update_chart with every field. To only format a chart (rupiah or percent or decimals, colours, a target or average line, axis titles, data labels, notes), call format_chart with only what changes: a field you give replaces that field and everything you leave out stays; for \"show values in rupiah, make the bars green, add a target line at 1,000\" send format ({style: currency, currency: IDR}), colors ({series: {<the value's result name>: \"#16a34a\"}}) and referenceLines ([{value: 1000, label: \"Target\"}]) in one call, and keep the chart's id from list_charts. A dashboard's theme (auto, light, dark) and font are set with set_dashboard_theme. To rename a board, call rename_board with the new name. To delete a board, call delete_board: it needs human approval, and the board's charts go with it. To arrange or resize the tiles on a board, call arrange_board with only the tiles that change; the grid has 12 columns and list_boards gives the current layout. To move a chart to another board, call move_chart with the chart id and the new board id. For data that combines several marts, list_sql_sources gives the saved SQL sources; pass one as sqlSource instead of mart. A chart can read only a Gold mart (describe_mart lists its columns) or a saved SQL source (list_sql_sources); a Silver or Bronze table cannot be charted, so say so and offer a mart or a SQL source instead. To put two marts side by side (for example visitors next to hotel capacity by province), call suggest_join, then create_chart with `joins`; a joined table's columns are <table>__<column>, and a joined table's measure should use max or avg. To group a number in ranges (visitors in bins of 500) pass `bin` ({mode: width, width: 500}, or auto, or count) on create_chart; for a map drawn as cells pass `grid`. To gather values under names of the user's own, take the values from the data, then create_value_group, and use its name as dimension. A hierarchy (list_hierarchies, create_hierarchy) lets a viewer click down from province to city; do not invent one. A parameter (list_parameters, create_parameter) is a value a viewer changes on the dashboard: a formula reads it as {name}, `limit` and a reference line (`referenceLines`) can take {param: name}, and a list parameter can switch the dimension, breakdown or a measure's column; to draw a target line, create a number parameter and pass it in referenceLines. Reuse an existing one. To draw two sources on one chart (visitors next to hotel rooms by month), keep the first question as the chart and add the other as `layers` (its own mart or sqlSource, a dimension of the same kind, measures, draw bar or line, axis left or right; right for numbers on another scale); do not join tables that should stay apart, and a chart with layers has no breakdown, 'Other' or steps. A progress bar (kind progress) and a bullet graph (kind bullet) take `goal` (a number, or {measure: <column>} with that column as the second of `measures`); a bullet also takes `bands` (up to three rising limits) and, without a dimension, is one bullet; a histogram (kind histogram) takes one number column as `dimension`, no measures, and optionally `histogram` ({count: 10} or {width: 500}); an image (kind image, with `alt`) or a web page (kind web) needs `url`, an https address on a site an administrator allows in Admin > Settings, and is refused otherwise.. To give a dashboard as a PDF, call export_dashboard with the board id (paper a4 or letter, orientation portrait or landscape, only if asked); to give a chart's data as Excel or CSV, call export_chart with the chart id from list_charts, format xlsx or csv, and values formatted or raw. On a dashboard page pass the active filters (the raw JSON in the page context) as `filters`, and the parameters and grain it lists as `params` and `grain`, so the file matches the screen. Both return a link the person opens; the file is made when they open it, with their own permissions, so say \"here is the link\" and never that the file is already made. Do not write the URL or a Markdown link in your answer: the console shows a Download button from the tool result, so just say it is below. To send a dashboard by email on a schedule (\"send this to me every Monday as a PDF\"), call create_report_subscription with the board id, a schedule (weekly: weekday 1 is Monday, hour, minute) and pdf true or `charts` (chart ids from list_charts with xlsx or csv); it goes to the person alone unless `recipients` names others, and naming any other recipient needs human approval, so say it is waiting for approval and do not say it was sent; an outside address works only on a domain an administrator allowed. list_report_subscriptions shows the person's reports, update_report_subscription changes or pauses one, delete_report_subscription needs approval. The report carries the asking person's own data and masking, so every recipient sees what they see. The page context lists the active filters; set_board_view_filters changes only this person's view and is the default for any filter request; save_board_filters only when the person asks to save for everyone.", limits_and_freshness!(), " To change how long a dashboard's or a chart's results are remembered (\"make this chart always fresh\"), call set_cache_rule with the chart or the board and rule default, fresh or minutes. The install's own rule is an administrator's setting in Admin > Settings.
- Named metrics come first: when the user asks for a number a company defines once (revenue, active customers, total visitors), call list_metrics BEFORE writing a query or a chart. If a metric matches, use it: query_metric gives its value (by one column or by a period, filtered if asked) as the person you are helping, a chart or KPI takes it as the measure {metric: name}, and your answer NAMES the metric you used (its label and name). If none matches, say so in one sentence before you write SQL. create_metric saves a new one (only when asked; the user needs the metric:write permission); update_metric changes every chart that uses it, so say so; delete_metric needs approval.
- Calculated fields (a value the table does not store, such as profit = revenue - cost): list_formula_functions gives the formula language and every function; write a FORMULA, never SQL; validate_formula checks it and returns the position of a mistake; create_calculated_field saves it on a mart or SQL source (then use its name in create_chart like a column); update_calculated_field changes it; list_calculated_fields shows what exists and explains a formula in plain words. Running totals, ranks, percent of total, moving averages, previous period, same period last year and Fixed are formula functions too: a table calculation or period comparison is only a measure of a bar, line, area, stacked, combo, waterfall or grouped table chart (period comparisons need Group by on a date); the server says why when it refuses. Distance(lat1, lon1, lat2, lon2, 'km') measures between two points and RegionOf('<map name>', lat, lon) names the region of an uploaded map that a point falls in (list_maps shows the maps): write them in a formula, never geoDistance or SQL; RegionOf is refused with a sentence for a role that has a data policy, so pass that sentence on instead of trying another way.
- Alerts and digests: list_alert_rules to see existing rules; create_alert_rule / update_alert_rule (alert: mart, measure, agg, op, threshold; digest: board); run_alert_rule sends the webhook or email for real.
- Connectors: list_connectors, create_connector (credentials are only ever a reference the server derives, never a real secret), test_connector for a real connection test.
- Ingest into Bronze: get_ingest_spec to see what a connector ingests, discover_source to list the source's tables, set_ingest_spec to choose tables and their Bronze targets (it cannot change where a connector points: that needs its credentials again and is done in the console), run_ingest to run it now, list_ingest_runs for results. rotate_connector_credential points a connector at a new credential.
- Pipelines: list_pipelines / get_pipeline / list_pipeline_runs for status; create_pipeline to author one (saved as draft), mark_pipeline_ready to make it runnable; set_pipeline_target_lag to set or clear refresh target lag; trigger_pipeline, retry_pipeline_run, resume_pipeline to run; trigger_lakehouse_build rebuilds every layer from what the deployment has (explain the plan in one line first, then report what was launched and what was skipped).
- Iceberg tables: list_iceberg_tables, describe_iceberg_table, get_table_maintenance, set_table_maintenance. Storage: get_capacity.
- Saved queries: save_query, list_saved_queries, run_saved_query.
- Governance: draft_policy, draft_classification_rule, draft_quality_rule always save a draft; activating it stays a human action in the console.
- Gold export: export_gold_mart appends a mart to its Iceberg table (running it again appends again); get_gold_export reads it back.
- These need human approval and do not run immediately: delete_alert_rule, delete_connector, rotate_connector_credential, pause_pipeline, cancel_pipeline_run, delete_chart, delete_tab, delete_board, delete_calculated_field, delete_metric, run_bronze_maintenance, kill_query. Call the tool; the server then creates the approval. Only when a tool result says needs_approval, tell the user the request is waiting in Approvals (/agents/approvals). Never say a request is waiting if you did not call the tool, or if its result did not say needs_approval.
- needs_confirmation and needs_approval are different. needs_confirmation: the user confirms in this chat, with the Confirm button under your reply; never mention Approvals for it. needs_approval: a human must approve it in Approvals (/agents/approvals).
- When a tool result says needs_confirmation, reply with ONE short sentence such as \"The chart draft is ready — review the preview below and confirm.\" Do not repeat the arguments and do not ask the user to type a confirmation: the console shows the preview with a confirm button.");

/// Tools offered on every turn: the data tools, and the read-only listings
/// a general question about the platform needs.
const ALWAYS: &[&str] = &[
    "run_sql",
    // `prepare_chat` takes it out again when `AI_ASK_BACK` is off, and the
    // console's allowlist decides it for a console chat.
    super::registry::ASK_USER,
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
    // BI-2 / AI-7: a question for a number may be answered by a named metric,
    // and the model can only prefer one if it can see them on every turn.
    "list_metrics",
    "describe_metric",
    "query_metric",
];

/// One domain of tools and the words (English and Indonesian, lowercase
/// substrings) that bring it into a turn.
struct Domain {
    words: &'static [&'static str],
    tools: &'static [&'static str],
}

const DOMAINS: &[Domain] = &[
    // BI-1 / AI-10: asking for a file. `list_charts` is here so the model can
    // find the chart id without a dashboard word in the request.
    Domain {
        words: &[
            "pdf",
            "excel",
            "xlsx",
            "csv",
            "download",
            "unduh",
            "export",
            "ekspor",
            "spreadsheet",
        ],
        tools: &["export_dashboard", "export_chart", "list_charts"],
    },
    Domain {
        words: &[
            "subscribe",
            "subscription",
            "schedule a report",
            "scheduled report",
            "every monday",
            "every week",
            "every day",
            "every month",
            "email me",
            "send me",
            "send this",
            "kirim",
            "langganan",
            "laporan terjadwal",
            "tiap senin",
            "setiap",
        ],
        tools: &[
            "create_report_subscription",
            "list_report_subscriptions",
            "update_report_subscription",
            "delete_report_subscription",
            "list_boards",
            "list_charts",
        ],
    },
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
            "metric",
            "metrik",
            "card",
            "kartu",
            "tile",
            "sql source",
            "sumber sql",
            "formula",
            "rumus",
            "calculated",
            "kolom hitung",
            "calculate",
            "profit",
            "margin",
            "minus",
            "ratio",
            "selisih",
            "filter",
            "saring",
            "rename",
            "ganti nama",
            "ubah nama",
            "pindah",
            "move",
            "layout",
            "susun",
            "tata",
            "resize",
            "ukuran",
            "default",
            "bins of",
            "histogram",
            "hierarchy",
            "drill down",
            "parameter",
            "reference line",
            "target line",
            // BI-17 part A / AI-8: formatting.
            "rupiah",
            "currency",
            "decimals",
            "colour",
            "color",
            "palette",
            "theme",
            "dark mode",
            "font",
            "axis",
            "trend line",
            // BI-4 part B: remembered results.
            "always fresh",
            "fresh",
            "cache",
            "remember",
            "stale",
            "up to date",
            "segar",
            // BI-18 part C / AI-9: tabs.
            // Whole words only: a bare "tab" is inside "table".
            "tabs",
            "a tab",
            "new tab",
        ],
        tools: &[
            "list_charts",
            "list_maps",
            "list_sql_sources",
            "suggest_join",
            "suggest_dashboard",
            "create_chart",
            "update_chart",
            "format_chart",
            "set_dashboard_theme",
            "set_cache_rule",
            "create_board",
            "create_tab",
            "rename_tab",
            "move_charts_to_tab",
            "move_tab",
            "delete_tab",
            "delete_chart",
            "rename_board",
            "delete_board",
            "arrange_board",
            "move_chart",
            "set_board_view_filters",
            "save_board_filters",
            "create_metric",
            "update_metric",
            "delete_metric",
            "list_formula_functions",
            "list_calculated_fields",
            "validate_formula",
            "create_calculated_field",
            "update_calculated_field",
            "delete_calculated_field",
            "create_value_group",
            "list_hierarchies",
            "create_hierarchy",
            "delete_hierarchy",
            "list_parameters",
            "create_parameter",
            "update_parameter",
            "delete_parameter",
        ],
    },
    // BI-5 / D7: the query limits and Stop.
    Domain {
        words: &[
            "query limit",
            "time limit",
            "row limit",
            "limits",
            "timed out",
            "time out",
            "timeout",
            "too long",
            "stop the",
            "stop query",
            "stop my",
            "batas waktu",
            "hentikan",
        ],
        tools: &["get_query_limits", "stop_query"],
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
            "target lag",
            "lag",
            "behind",
        ],
        tools: &[
            "get_pipeline",
            "create_pipeline",
            "mark_pipeline_ready",
            "set_pipeline_target_lag",
        ],
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
    fn the_count_rules_are_five_lines_and_name_no_dataset() {
        let lines = DISTINCT_COUNT_RULES
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();
        assert!(lines <= 5, "{lines} lines: {DISTINCT_COUNT_RULES}");
        // A word of one dataset would tie these rules to one deployment.
        let lower = DISTINCT_COUNT_RULES.to_lowercase();
        for word in ["wisman", "pariwisata", "mart_", "outlet", "distributor"] {
            assert!(!lower.contains(word), "`{word}` in {DISTINCT_COUNT_RULES}");
        }
        assert!(
            DISTINCT_COUNT_RULES.contains(crate::routes::ai::data_map::NON_ADDITIVE_MARKER.trim()),
            "the marker the DATA MAP writes is not quoted"
        );
    }

    #[test]
    fn the_sql_help_rules_are_six_lines_and_hold_both_sentences_verbatim() {
        let lines = SQL_HELP_RULES
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();
        assert!(lines <= 6, "{lines} lines: {SQL_HELP_RULES}");
        // The console sends these two sentences; the rules must name them
        // exactly, or the model does not know which branch a message is in.
        assert!(SQL_HELP_RULES.contains(FIX_REQUEST), "{SQL_HELP_RULES}");
        assert!(SQL_HELP_RULES.contains(EXPLAIN_REQUEST), "{SQL_HELP_RULES}");
    }

    #[test]
    fn the_sql_help_rules_name_no_dataset() {
        let lower = SQL_HELP_RULES.to_lowercase();
        for word in [
            "wisman",
            "pariwisata",
            "mart_",
            "kunjungan",
            "serving.",
            "silver.",
        ] {
            assert!(!lower.contains(word), "`{word}` in {SQL_HELP_RULES}");
        }
    }

    #[test]
    fn propose_sql_is_offered_only_by_an_allowlist_never_by_default() {
        // The chat's own list and the eval's plain chat keep today's tools:
        // the tool is in no domain and not in `ALWAYS`.
        assert!(!ALWAYS.contains(&super::super::registry::PROPOSE_SQL));
        for domain in DOMAINS {
            assert!(!domain.tools.contains(&super::super::registry::PROPOSE_SQL));
        }
        for recent in [
            vec!["Why does this query fail? Fix it."],
            vec!["Explain what this query does."],
        ] {
            assert!(!select_tools(&recent).contains(super::super::registry::PROPOSE_SQL));
        }
    }

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
            // `propose_sql` is offered only when the console's allowlist names
            // it (plan 15 decision: the eval's plain chat keeps today's tools),
            // so it is reachable by that path and by no domain.
            if spec.name == super::super::registry::PROPOSE_SQL {
                continue;
            }
            let reachable =
                ALWAYS.contains(&spec.name) || DOMAINS.iter().any(|d| d.tools.contains(&spec.name));
            assert!(reachable, "{} can never be offered to the model", spec.name);
        }
    }

    /// Step 2 of plan 20: the six new dashboard tools must be in the
    /// dashboard domain, so a conversation that mentions a chart, a
    /// board, a layout, a filter or a rename offers them.
    #[test]
    fn each_new_dashboard_board_tool_is_in_the_dashboard_domain() {
        // The dashboard domain is the one that already offers the
        // existing chart tools; the new six sit beside them.
        let dashboard = DOMAINS
            .iter()
            .find(|d| d.tools.contains(&"create_chart"))
            .unwrap_or_else(|| panic!("dashboard domain not found"));
        for tool in [
            "rename_board",
            "delete_board",
            "arrange_board",
            "move_chart",
            "set_board_view_filters",
            "save_board_filters",
        ] {
            assert!(
                dashboard.tools.contains(&tool),
                "{tool} is not in the dashboard domain"
            );
        }
    }

    /// Step 2 of plan 20: a neutral filter word brings in the
    /// dashboard domain, so `set_board_view_filters` is offered for
    /// "filter to last year" — no dataset words, no year words.
    #[test]
    fn a_filter_word_brings_in_set_board_view_filters() {
        let tools = select_tools(&["filter to last year"]);
        assert!(
            tools.contains("set_board_view_filters"),
            "filter word should offer set_board_view_filters: {tools:?}"
        );
    }

    /// Step 2 of plan 20: `delete_board` is in the approval list of
    /// [`SYSTEM_BUILD_SUFFIX`], and the rule sentence around it is
    /// preserved (the list still says "Call the tool; the server then
    /// creates the approval.").
    #[test]
    fn delete_board_is_in_the_approval_list() {
        // Search the approval LINE, not the whole suffix: `delete_board`
        // is also named in the charts line ("To delete a board, call
        // delete_board"), so a suffix-wide `contains` passes even when
        // the approval list has lost the name.
        let line = SYSTEM_BUILD_SUFFIX
            .lines()
            .find(|l| l.contains("These need human approval"))
            .unwrap_or_else(|| panic!("the approval line is not in the build suffix"));
        assert!(
            line.contains("delete_board"),
            "delete_board is not in the approval line: {line}"
        );
        assert!(
            line.contains("Call the tool; the server then creates the approval."),
            "the approval-list rule sentence is no longer present"
        );
    }

    /// `BI-16` part C3 (D10): the build prompt tells the assistant the two
    /// spatial functions and that a data policy refuses `RegionOf`.
    #[test]
    fn the_build_prompt_names_distance_and_region_of() {
        let line = SYSTEM_BUILD_SUFFIX
            .lines()
            .find(|l| l.contains("Calculated fields (a value"))
            .unwrap_or_else(|| panic!("the calculated-fields line is not in the build suffix"));
        for needle in [
            "Distance(",
            "RegionOf(",
            "list_maps",
            "never geoDistance",
            "data policy",
        ] {
            assert!(
                line.contains(needle),
                "{needle} is missing from the calculated-fields line"
            );
        }
    }

    fn term(term: &str, meaning: &str) -> ChatTerm {
        ChatTerm {
            term: term.to_owned(),
            meaning: meaning.to_owned(),
            question: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn no_terms_means_no_words_section() {
        assert_eq!(user_words_section(&[]), "");
    }

    #[test]
    fn each_term_is_one_line_in_a_fixed_format_under_one_header() {
        let section = user_words_section(&[term("hotel", "the lodging table"), term("a", "b")]);
        assert_eq!(
            section,
            "\n\nTHIS USER'S WORDS\n- \"hotel\" means the lodging table\n- \"a\" means b"
        );
    }

    #[test]
    fn line_breaks_and_control_characters_become_single_spaces() {
        let section = user_words_section(&[term(
            "ho\ntel\r\n\u{2028}x",
            "one\n\nTHIS USER'S WORDS\n- \"z\" means\t\u{7}two   three\u{85}",
        )]);
        assert_eq!(
            section,
            "\n\nTHIS USER'S WORDS\n- \"ho tel x\" means one THIS USER'S WORDS - \"z\" means two three"
        );
        assert_eq!(section.lines().filter(|l| !l.is_empty()).count(), 2);
    }

    #[test]
    fn a_value_is_cut_to_its_stored_rule() {
        let section = user_words_section(&[term(&"t".repeat(80), &"m".repeat(300))]);
        let line = section.lines().next_back().unwrap_or_default();
        assert_eq!(
            line,
            format!("- \"{}\" means {}", "t".repeat(60), "m".repeat(200))
        );
    }

    #[test]
    fn only_the_first_thirty_terms_of_a_newest_first_list_are_carried() {
        let terms: Vec<ChatTerm> = (0..35).map(|i| term(&format!("t{i}"), "m")).collect();
        let section = user_words_section(&terms);
        let lines: Vec<&str> = section.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 30);
        assert_eq!(lines[0], "- \"t0\" means m");
        assert_eq!(lines[29], "- \"t29\" means m");
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
        assert!(closing("How many rows are there?", None).contains("Reply in English"));
        assert!(closing("ok", None).contains("language of the user's latest message"));
    }

    #[test]
    fn a_short_english_prompt_is_read_as_english() {
        // The prompt that came back in Chinese: one listed word ("the")
        // used to leave it undecided.
        assert_eq!(
            reply_language("Explain the charts on \"Main\""),
            Some("English")
        );
        assert_eq!(
            reply_language("Why did gold_export_job fail?"),
            Some("English")
        );
        // One Indonesian word is still not enough to decide.
        assert_eq!(reply_language("Jelaskan chart ini"), None);
    }

    #[test]
    fn an_undecided_latin_script_message_is_never_answered_in_another_script() {
        for text in ["ok", "Jelaskan chart ini", "mart_wisman"] {
            let line = closing(text, None);
            assert!(line.contains("Latin script"), "{text}: {line}");
            assert!(line.contains("never in Chinese"), "{text}: {line}");
        }
        // A message in another script keeps the plain rule: the user's
        // language is the one to reply in.
        for text in ["这个图表是什么", "что это"] {
            let line = closing(text, None);
            assert!(!line.contains("Latin script"), "{text}: {line}");
            assert!(
                line.contains("language of the user's latest message"),
                "{text}"
            );
        }
        // No letters at all is not "Latin".
        assert!(!closing("123 ?", None).contains("Latin script"));
    }

    // The two `None` lines of `closing` as they were before the default
    // reply language existed. Copied from the code, not recomputed, so
    // "unchanged" is checked against the old text.
    const LATIN_SCRIPT_LINE: &str = "\n\nLANGUAGE: reply in the language of the user's latest message. It is written in Latin script, so reply in that same language and script (English if you cannot tell), never in Chinese or any other script. The language of the data, table names or DATA MAP does not change this.";
    const PLAIN_LINE: &str = "\n\nLANGUAGE: reply in the language of the user's latest message. The language of the data, table names or DATA MAP does not change this.";

    #[test]
    fn without_a_default_the_undecided_lines_are_unchanged() {
        assert_eq!(closing("Jelaskan chart ini", None), LATIN_SCRIPT_LINE);
        assert_eq!(closing("123 ?", None), PLAIN_LINE);
        assert_eq!(closing("这个图表是什么", None), PLAIN_LINE);
    }

    #[test]
    fn an_indonesian_default_replaces_english_as_the_latin_script_fallback() {
        let line = closing("Jelaskan chart ini", Some(ReplyLanguage::Indonesian));
        assert!(line.contains("Indonesian if you cannot tell"), "{line}");
        assert!(line.contains("Latin script"), "{line}");
        assert!(line.contains("never in Chinese"), "{line}");
        assert!(!line.contains("English"), "{line}");
    }

    #[test]
    fn an_english_default_leaves_the_latin_script_line_as_it_was() {
        assert_eq!(
            closing("Jelaskan chart ini", Some(ReplyLanguage::English)),
            LATIN_SCRIPT_LINE
        );
    }

    #[test]
    fn a_detected_language_wins_over_the_default() {
        let line = closing("How many rows are there?", Some(ReplyLanguage::Indonesian));
        assert!(line.contains("Reply in English"), "{line}");
        assert!(!line.contains("Indonesian"), "{line}");
        let line = closing(
            "Berapa jumlah data di tabel ini?",
            Some(ReplyLanguage::English),
        );
        assert!(line.contains("Reply in Indonesian"), "{line}");
        assert!(!line.contains("English"), "{line}");
    }

    #[test]
    fn a_message_with_no_letters_is_answered_in_the_default() {
        let line = closing("123 ?", Some(ReplyLanguage::Indonesian));
        assert!(
            line.contains("If it is too short to tell, reply in Indonesian."),
            "{line}"
        );
        assert!(!line.contains("Latin script"), "{line}");
        assert_eq!(
            line,
            "\n\nLANGUAGE: reply in the language of the user's latest message. If it is too short to tell, reply in Indonesian. The language of the data, table names or DATA MAP does not change this."
        );
    }

    #[test]
    fn a_message_in_another_script_ignores_the_default() {
        assert_eq!(
            closing("这个图表是什么", Some(ReplyLanguage::Indonesian)),
            PLAIN_LINE
        );
    }

    /// `BI-18` part C review fix (SHOULD-FIX) R30.
    #[test]
    fn a_person_without_dashboard_write_gets_the_permission_sentence_in_the_context() {
        use lakehouse_auth::PermissionSet;
        let viewer = PermissionSet::parse("dashboard:read");
        let note = dashboard_permission_note(Some(&viewer)).unwrap_or_default();
        assert!(
            note.contains("does not have the dashboard:write permission"),
            "{note}"
        );
        assert!(
            note.contains("Your account cannot change dashboards"),
            "{note}"
        );
        assert!(note.contains("not because they are unavailable"), "{note}");
        assert!(dashboard_permission_note(None).is_none());
        let full = with_permission_note(
            "HEAD\n\nLANGUAGE: x".to_owned(),
            Some(&viewer),
            "\n\nLANGUAGE: x",
        );
        assert!(
            full.starts_with("HEAD\n\n[Permissions:") && full.ends_with("\n\nLANGUAGE: x"),
            "{full}"
        );
        let editor = PermissionSet::parse("dashboard:write, dashboard:read");
        assert!(dashboard_permission_note(Some(&editor)).is_none());
    }
}
