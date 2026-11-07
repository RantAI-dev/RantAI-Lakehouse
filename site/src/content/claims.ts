/**
 * Where every product claim the WEB-5 refresh added comes from (D7).
 *
 * A claim may only name what `docs/core/BACKLOG.md` marks Released, Done or
 * In Acceptance: PRODUCT §2's BI rows are stale, so the backlog is the
 * source. Each entry names the message key that makes the claim and the
 * backlog rows behind it, with their status when this was written
 * (2026-11-04), so a reviewer can check one row per line. `claims.test.ts`
 * fails if a key is missing in either language, or if a phrase from the
 * "do not add" list appears anywhere in the messages.
 *
 * The rule of WEB-1's copy stands: no logos, numbers, testimonials, prices
 * or "trusted by".
 */

export type Claim = {
  /** Dotted message path; array items by index (`home.sources.items.9`). */
  key: string;
  /** Backlog rows, each with its status when the claim was added (2026-11-04; the FAQ 2026-11-07). */
  backlog: readonly string[];
  /** What part of the text the rows back, when the key holds more than one claim. */
  note?: string;
};

export const CLAIMS: readonly Claim[] = [
  // Sources
  { key: "home.sources.sub", backlog: ["SRC-3 Released"], note: "business apps" },
  { key: "home.sources.items.9", backlog: ["SRC-3 Released"], note: "Salesforce" },
  { key: "home.sources.items.10", backlog: ["SRC-3 Released"], note: "ServiceNow" },
  { key: "home.sources.items.11", backlog: ["SRC-3 Released"], note: "Google Analytics 4" },
  { key: "home.sources.items.12", backlog: ["SRC-3 Released"], note: "HubSpot" },
  { key: "home.sources.items.13", backlog: ["SRC-3 Released"], note: "Zendesk" },
  { key: "home.sources.items.14", backlog: ["SRC-3 Released"], note: "Confluence" },
  { key: "home.sources.items.15", backlog: ["SRC-3 Released"], note: "Dynamics 365" },
  { key: "home.sources.items.16", backlog: ["SRC-3 Released"], note: "SharePoint" },
  { key: "home.sources.items.17", backlog: ["DATA-17 Released"], note: "JSON, JSON Lines and Avro uploads" },
  { key: "home.sources.items.18", backlog: ["DATA-17 Released"], note: "gzip and zip archives" },
  {
    key: "home.sources.caption",
    backlog: ["SRC-10 Released", "SRC-13 Released", "DATA-17 Released", "DATA-35 Released"],
    note: "dedupe and keep history (SCD2) on load; table discovery for every source; 10 files at once; folders. Live change capture for PostgreSQL is WEB-1's claim, kept (D7)",
  },
  // Depths
  // O21 (2026-11-07): each depth shows what it does to the data, on a card of
  // fictional rows labelled "Illustration" (components/home/depth-records.ts).
  // The words of each line and of each card label are claims.
  { key: "home.depths.layers.bronze.body", backlog: ["WEB-1 In Acceptance", "BLD-3 Done"], note: "every source lands untouched in open Iceberg tables (WEB-1's copy; BLD-3: Bronze is Iceberg)" },
  { key: "home.depths.card.bronzeNote", backlog: ["WEB-1 In Acceptance", "BLD-3 Done"], note: "Bronze stores what the source sent; nothing is changed or dropped" },
  {
    key: "home.depths.layers.silver.body",
    backlog: ["WEB-1 In Acceptance", "SRC-10 Released", "DATA-14 Released"],
    note: "pipelines clean and join the raw data (WEB-1's copy); dropping duplicates is also a load mode (SRC-10); quality checks flag what fails (DATA-14)",
  },
  {
    key: "home.depths.card.checks.duplicates",
    backlog: ["DATA-14 Released"],
    note: "duplicate count is one of DATA-14's ready checks (features/data-quality-in-depth.md, decision 1)",
  },
  { key: "home.depths.card.checks.city", backlog: ["DATA-14 Released"], note: "accepted values is one of DATA-14's ready checks" },
  {
    key: "home.depths.card.checks.amount",
    backlog: ["DATA-14 Released"],
    note: "null count is one of DATA-14's ready checks; a failing check is flagged and never blocks the load (decision 4), so the row stays",
  },
  { key: "home.depths.layers.gold.body", backlog: ["BLD-2 In Acceptance", "AI-18 In Acceptance", "WEB-1 In Acceptance"], note: "tables that refresh themselves within a target lag (BLD-2); dashboards and the assistant read Gold (WEB-1's copy; the assistant's describe_mart tool, AI-18)" },
  { key: "home.depths.card.lag", backlog: ["BLD-2 In Acceptance"], note: "a table refreshing within its target lag" },
  { key: "home.depths.card.feeds", backlog: ["WEB-1 In Acceptance", "AI-18 In Acceptance"], note: "dashboards and the assistant read Gold" },
  // Assistant section (Query Studio, D7)
  { key: "home.assistant.sub", backlog: ["QS-1 In Acceptance", "QS-4 In Acceptance"], note: "autocomplete, tabs, query history" },
  // O19 (2026-11-07): the assistant panel is an illustration, but every
  // behaviour it plays is the console's own.
  { key: "home.assistant.placeholder", backlog: ["AI-18 In Acceptance"], note: "the console composer's own placeholder (src/features/copilot/chat-composer.tsx)" },
  { key: "home.assistant.modes.build", backlog: ["AI-18 In Acceptance"], note: "the composer's Ask/Build switch; Build can change things, asking first" },
  { key: "home.assistant.steps.read", backlog: ["AI-18 In Acceptance", "AI-24 Done"], note: "a turn shows its tool steps as they run and keeps them (tool-step.tsx; AI-24)" },
  {
    key: "home.assistant.approvalLabel",
    backlog: ["AI-18 In Acceptance"],
    note: "pause_pipeline is a WriteHigh tool: \"Needs human approval before it runs\" (registry.rs); it waits in Approvals and nothing executes until a person approves (gate.rs)",
  },
  { key: "home.assistant.impact", backlog: ["AI-18 In Acceptance"], note: "pause_pipeline pauses the pipeline's schedule; the 02:00 run is the illustration's" },
  { key: "home.assistant.approvedBy", backlog: ["AI-18 In Acceptance", "SEC-57 Done"], note: "another person approves: the one who asked cannot approve their own request (SEC-57)" },
  // Dashboards
  // O10 (2026-11-07): the dashboards' feature list became one line and a row of chips; each chip is its own claim.
  {
    key: "home.dashboards.sub",
    backlog: ["WEB-1 In Acceptance", "BI-16 Building (part C2 merged in #246/#265): your own map boundaries, not every chart type"],
    note: "about two dozen chart types (WEB-1's claim); maps on your own boundaries (BI-16 C2)",
  },
  { key: "home.dashboards.features.0", backlog: ["BI-8 In Acceptance"], note: "calculated fields" },
  { key: "home.dashboards.features.1", backlog: ["BI-11 In Acceptance"], note: "parameters" },
  { key: "home.dashboards.features.2", backlog: ["BI-2 In Acceptance"], note: "reusable metrics" },
  { key: "home.dashboards.features.3", backlog: ["BI-4 In Acceptance"], note: "results remembered and refreshed" },
  { key: "home.dashboards.features.4", backlog: ["BI-19 In Acceptance"], note: "comments with @mentions" },
  { key: "home.dashboards.features.5", backlog: ["BI-19 In Acceptance"], note: "alerts on a chart" },
  { key: "home.dashboards.features.6", backlog: ["BI-19 In Acceptance", "BI-1 In Acceptance"], note: "PNG export (BI-19); PDF and Excel (BI-1)" },
  { key: "home.dashboards.features.7", backlog: ["RPT-1 In Acceptance"], note: "scheduled reports" },
  { key: "home.dashboards.features.8", backlog: ["BI-19 In Acceptance", "SEC-12 In Acceptance"], note: "public links with an expiry and a password (BI-19 A)" },
  { key: "home.dashboards.features.9", backlog: ["BI-26 In Acceptance", "SEC-12 In Acceptance"], note: "embeds" },
  // Governance
  // The hero's reassurance line (D21, 2026-11-07): each part is a claim.
  { key: "home.hero.reassurance.0", backlog: ["WEB-1 In Acceptance"], note: "free and open source: the repository is public under the AGPL-3.0, with no price (the open-source card; JSON-LD isAccessibleForFree)" },
  { key: "home.hero.reassurance.1", backlog: ["WEB-1 In Acceptance"], note: "the repository's LICENSE is the GNU AGPL v3" },
  { key: "home.hero.reassurance.2", backlog: ["WEB-1 In Acceptance", "REL-2 Done"], note: "the 'Your servers or ours' card; the stack runs from docker-compose.yml (REL-2)" },
  // Governance (O16 + O17, 2026-11-07): the one-line lead and the bento's lines.
  { key: "home.governance.sub", backlog: ["WEB-1 In Acceptance"], note: "roles decide who sees what; columns are classified and masked (WEB-1's claim, the table shows it)" },
  { key: "home.governance.bento.lineage.line", backlog: ["DATA-13 Released"], note: "column-level lineage" },
  { key: "home.governance.bento.proof.line", backlog: ["WEB-1 In Acceptance"], note: "masking proven end to end in an acceptance gate (WEB-1's governance fact, kept; the gate is ops/g*)" },
  { key: "home.governance.bento.audit.line", backlog: ["AI-18 In Acceptance", "SEC-57 Done"], note: "assistant actions logged; risky ones wait for a person's approval" },
  { key: "home.governance.bento.catalog.line", backlog: ["DATA-11 Released", "DATA-12 Released"], note: "catalog search; certified and deprecated marks; governed tags (the tag chip)" },
  { key: "home.governance.bento.access.line", backlog: ["BI-19 In Acceptance"], note: "permissions per dashboard and per folder" },
  // The real console (D19, 2026-11-07): a crop of rantai-page's
  // public/products/lake-home.webp; the strings are the console's own
  // (src/lib/home-prompts.ts, src/features/home/home-shortcuts.ts).
  {
    key: "home.open.console.caption",
    backlog: ["AI-18 In Acceptance"],
    note: "ask in plain language on the console's home (Copilot Chat); the three shortcuts are the home page's own",
  },
  // Get started (D20, 2026-11-07): the README's Quickstart, step by step.
  { key: "home.getStarted.sub", backlog: ["WEB-1 In Acceptance"], note: "README Quickstart: \"Assumes you have Docker installed and nothing else\"" },
  { key: "home.getStarted.steps.0.note", backlog: ["WEB-1 In Acceptance"], note: "README Quickstart step 1: git clone the public repository" },
  { key: "home.getStarted.steps.1.note", backlog: ["SEC-18 Done", "REL-2 Done"], note: "README step 2: ops/init-env.sh writes .env with generated values (no default credentials, SEC-18); AUTH_BOOTSTRAP_EMAIL / _PASSWORD create the first admin" },
  { key: "home.getStarted.steps.2.note", backlog: ["REL-2 Done"], note: "README step 3: scripts/compose.sh up --build; the console listens on :3000 once the API is healthy (REL-2 slice 1: the console in the stack)" },
  // The footer's prompt (O7, 2026-11-07): example requests a visitor can send; each names something built.
  { key: "common.footer.cta.examples.0", backlog: ["WEB-1 In Acceptance", "REL-2 Done"], note: "runs on your own infrastructure (the 'Your servers or ours' card)" },
  { key: "common.footer.cta.examples.1", backlog: ["WEB-1 In Acceptance"], note: "PostgreSQL is a source on the page (sources section)" },
  { key: "common.footer.cta.examples.2", backlog: ["SRC-3 Released"], note: "Salesforce connector" },
  { key: "common.footer.cta.examples.3", backlog: ["WEB-1 In Acceptance"], note: "a walkthrough is one of the contact form's topics (D15); no product claim" },
  // Contact page FAQ (D15, 2026-11-07): every answer is a claim. Most repeat
  // what the home page already says (WEB-1's copy, "limited to what is
  // built", In Acceptance), so they trace to the same rows.
  {
    key: "contact.faq.items.0.a",
    backlog: ["WEB-1 In Acceptance", "SRC-3 Released", "AI-18 In Acceptance", "QS-1 In Acceptance"],
    note: "one lakehouse with dashboards; databases, files, streams and business apps; open Iceberg tables; plain-language answers (assistant); SQL in Query Studio",
  },
  {
    key: "contact.faq.items.1.a",
    backlog: ["WEB-1 In Acceptance"],
    note: "the open-source card (AGPL-3.0); the repository's LICENSE is the GNU AGPL v3 and the repository is public",
  },
  {
    key: "contact.faq.items.2.a",
    backlog: ["WEB-1 In Acceptance", "REL-2 Done"],
    note: "the 'Your servers or ours' card; the stack runs from docker-compose.yml with its object storage (rustfs) and Iceberg catalog (lakekeeper) in the default profile (REL-2 slice 1: the console in the stack)",
  },
  {
    key: "contact.faq.items.3.a",
    backlog: ["WEB-1 In Acceptance", "SRC-3 Released", "DATA-17 Released"],
    note: "the sources section's list, word for word: databases, Kafka, REST, files (WEB-1); the eight business apps (SRC-3); JSON, JSON Lines, Avro, archives (DATA-17); change capture for PostgreSQL only (D7)",
  },
  {
    key: "contact.faq.items.4.a",
    backlog: ["WEB-1 In Acceptance", "DATA-14 Released", "BLD-2 In Acceptance"],
    note: "the depths section: raw in Iceberg (WEB-1); quality checks (DATA-14); refresh within a target lag (BLD-2)",
  },
  {
    key: "contact.faq.items.5.a",
    backlog: ["AI-18 In Acceptance", "SEC-57 Done"],
    note: "the console's own tools; risky actions wait for approval (AI-18; the pause example is the home page's illustration); the asker cannot approve their own request (SEC-57)",
  },
  {
    key: "contact.faq.items.6.a",
    backlog: ["WEB-1 In Acceptance", "BLD-3 Done", "DATA-1 In Acceptance"],
    note: "raw data in Iceberg, readable by any Iceberg engine (the open-tables card; BLD-3: Bronze is Iceberg); Gold published as open tables (DATA-1)",
  },
  {
    key: "contact.faq.items.7.a",
    backlog: ["WEB-1 In Acceptance", "REL-2 Done"],
    note: "README Quickstart: clone, `sh ops/init-env.sh`, `scripts/compose.sh up` (Docker Compose); 'Install it' links there (D8). Not the one-step installer: REL-2's release is a draft until the owner publishes it",
  },
];

/**
 * D7's "do not add" list, as patterns no message may match: change capture
 * for MySQL or SQL Server (SRC-4 Planned, VER-1 Todo), SFTP (VER-1 Todo),
 * and "every chart type" (BI-16 Building), in both languages.
 */
export const FORBIDDEN: readonly RegExp[] = [
  // English and Indonesian ("change capture untuk MySQL").
  /change capture (?:\S+ )?(?:for|untuk) (?:[^.]*\b)?(?:MySQL|SQL Server)/i,
  /\bSFTP\b/i,
  /every chart type|(?:semua|setiap) jenis chart/i,
];
