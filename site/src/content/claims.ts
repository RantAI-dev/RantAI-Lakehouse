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
  /** Backlog rows, each with its status on 2026-11-04. */
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
  { key: "home.depths.layers.silver.body", backlog: ["DATA-14 Released"], note: "quality checks with anomaly detection" },
  { key: "home.depths.layers.gold.body", backlog: ["BLD-2 In Acceptance"], note: "tables that refresh themselves within a target lag" },
  // Assistant section (Query Studio, D7)
  { key: "home.assistant.sub", backlog: ["QS-1 In Acceptance", "QS-4 In Acceptance"], note: "autocomplete, tabs, query history" },
  // Dashboards
  {
    key: "home.dashboards.sub",
    backlog: [
      "BI-16 Building (part C2 merged in #246/#265): your own map boundaries, not every chart type",
      "BI-8 In Acceptance",
      "BI-11 In Acceptance",
      "BI-2 In Acceptance",
      "BI-4 In Acceptance",
      "BI-19 In Acceptance",
      "BI-1 In Acceptance",
      "RPT-1 In Acceptance",
      "BI-26 In Acceptance",
      "SEC-12 In Acceptance",
    ],
    note: "boundaries (BI-16 C2); calculated fields, parameters, metrics (BI-8, BI-11, BI-2); results remembered and refreshed (BI-4); comments with @mentions, alerts on a chart, PNG export, public links with expiry and password (BI-19); PDF and Excel (BI-1); scheduled reports (RPT-1); embeds (BI-26, SEC-12)",
  },
  // Governance
  { key: "home.governance.sub", backlog: ["DATA-13 Released"], note: "column-level lineage" },
  { key: "home.governance.facts.2.text", backlog: ["DATA-13 Released"], note: "column-level lineage" },
  { key: "home.governance.facts.3.text", backlog: ["DATA-11 Released", "DATA-12 Released"], note: "catalog search; certified and deprecated marks; governed tags" },
  { key: "home.governance.facts.4.text", backlog: ["BI-19 In Acceptance"], note: "permissions per dashboard and folder" },
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
