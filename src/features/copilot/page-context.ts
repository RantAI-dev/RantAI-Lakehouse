/**
 * Page-aware context for the AI Copilot. Derives what the user is likely doing
 * from the current route so the dock/page can tailor its greeting, suggestions,
 * and the system prompt sent to the model. Specific pages (e.g. a dashboard)
 * can override this with richer entity context via setPageContext().
 *
 * The suggestions name no tenant's subject matter: they used to read
 * "Total foreign visitors by region" / "Which datasets are about halal?" /
 * "Describe the wisman dataset" — one deployment's tourism vocabulary,
 * shown as the product's own starter prompts to every other tenant
 * (AGENTS.md rule 12, same finding as the `sdi-primer` namespace id).
 * Anything named here must exist in every deployment: catalog, pipelines,
 * quality, lineage, and the generic shape of a mart.
 */

export type PageSuggest = { ask: string[]; build: string[] };
export type PageContext = {
  key: string;
  title: string; // short greeting subject
  hint: string; // empty-state helper text
  suggest: PageSuggest;
  system: string; // injected into the AI system prompt
};

const GENERIC: PageContext = {
  key: "generic",
  title: "How can I help?",
  hint: "Ask about your data, or switch to Build to create charts, dashboards, or refresh data.",
  suggest: {
    ask: ["Which datasets are in the catalog?", "List the pipelines and their status", "Summarize lakehouse data quality"],
    build: ["Create a revenue-by-channel chart", "Build a revenue dashboard", "Check the latest build status"],
  },
  system: "The user is in the RantAI Lakehouse console.",
};

const MAP: { test: (p: string) => boolean; ctx: PageContext }[] = [
  {
    test: (p) => p === "/",
    ctx: {
      key: "overview",
      title: "What would you like to explore?",
      hint: "Ask about platform health, key metrics, or where to look next.",
      suggest: {
        ask: ["Summarize platform health", "Which datasets are stale?", "Which pipeline ran last?"],
        build: ["Build a revenue dashboard", "Refresh the lakehouse (Bronze→Silver→Gold)"],
      },
      system: "The user is on the Overview page (platform health across storage, pipelines, queries, governance). Help them find insights or navigate.",
    },
  },
  {
    test: (p) => p.startsWith("/dashboards"),
    ctx: {
      key: "dashboards",
      title: "Build or explore a dashboard",
      hint: "Create a chart, start a new dashboard, or ask about what's shown.",
      suggest: {
        ask: ["Explain the charts on this dashboard", "Which mart backs this dashboard?"],
        build: ["Create a chart of revenue by channel", "Add a KPI of total revenue", "Build a new revenue dashboard"],
      },
      system: "The user is on the Dashboards page. They most likely want to create charts, create/edit a dashboard, or understand what's shown. Prefer create_chart / create_board / update_chart.",
    },
  },
  {
    // WEB-3 S2-D5: the Catalog is the one asset list (`/data` only redirects
    // to it); an asset's own page stays at `/data/assets/{id}`.
    test: (p) => p.startsWith("/catalog") || p.startsWith("/data/"),
    ctx: {
      key: "data",
      title: "Find and understand data",
      hint: "Ask which datasets exist, what a table holds, or where it comes from.",
      suggest: {
        ask: ["Which datasets hold customer data?", "Describe this dataset's columns", "Show this dataset's lineage"],
        build: ["Refresh the lakehouse (Bronze→Silver→Gold)", "Check the latest build status"],
      },
      // WEB-3 S3-D1: a column's sensitivity and masks are set on the column, in the Schema tab.
      system: "The user is in the Catalog: every governed table and file, searchable by name, column, tag, description and owner, filterable by namespace and layer, with each asset's own page for schema (each column's sensitivity and masks), quality, lineage and access. Prefer list_datasets / describe_dataset / get_lineage / get_quality.",
    },
  },
  {
    // WEB-3 S2-D5: System status replaced Health, Services, Capacity,
    // Ingestion (CDC) and Table Maintenance; their old addresses redirect
    // here. Every tool named below already exists (no tool change).
    test: (p) => p === "/health",
    ctx: {
      key: "system-status",
      title: "Check the system",
      hint: "Ask what is unhealthy, how much is stored, or what maintenance last did.",
      suggest: {
        ask: ["Summarize platform health", "How much storage is the lakehouse using?", "Are any CDC replication slots lagging?"],
        build: ["Create an alert rule for a failing pipeline"],
      },
      system: "The user is on System status: service probes, open alerts, storage capacity, CDC replication-slot health, the latest table maintenance runs, table freshness and query volume. Prefer lakehouse_overview / get_capacity / get_cdc_health / get_maintenance_metrics. A value the page shows as Not measured is not measured; never estimate it.",
    },
  },
  {
    test: (p) => p.startsWith("/pipelines"),
    ctx: {
      key: "pipelines",
      title: "Run or inspect pipelines",
      hint: "Ask about pipeline runs, or build/refresh the lakehouse.",
      suggest: {
        ask: ["Check the latest build status", "Summarize lakehouse data quality"],
        build: ["Rebuild the lakehouse (Bronze→Silver→Gold)", "Rebuild the Gold marts"],
      },
      system: "The user is on Pipelines. They may want to run pipelines, check build status, or build/refresh data.",
    },
  },
  {
    test: (p) => p.startsWith("/query"),
    ctx: {
      key: "query",
      title: "Write and run SQL",
      hint: "Ask a data question — AI Copilot writes and runs the SQL.",
      suggest: {
        ask: ["Total revenue by channel", "Top 10 products by revenue", "Monthly revenue trend"],
        build: ["Turn this into a chart on a dashboard"],
      },
      system: "The user is in Query Studio. Prefer run_sql; offer to turn results into a chart.",
    },
  },
  {
    // WEB-3 D9: the approvals inbox moved to `/governance/approvals` (D4).
    // It sits before the generic Governance entry so the inbox gets its own
    // context; its suggestions name no removed page.
    test: (p) => p.startsWith("/governance/approvals"),
    ctx: {
      key: "approvals",
      title: "Review what needs you",
      hint: "Ask what a request would let the agent do, or why it was raised.",
      suggest: {
        ask: ["What is waiting for approval?", "What would this approval let the agent do?", "Summarize this week's approval decisions"],
        build: ["Chart approval decisions over the last 30 days"],
      },
      system: "The user is on the Approvals inbox (agent tool calls and catalog access requests). Explain impact and risk; never decide for them.",
    },
  },
  {
    test: (p) => p.startsWith("/governance") || p.startsWith("/lineage") || p.startsWith("/audit"),
    ctx: {
      key: "governance",
      title: "Governance & quality",
      // WEB-3 S3-D5, S3-D10: Data Quality holds the schedules and freshness
      // targets; classification moved to the column (no Governance page).
      hint: "Ask about data quality, freshness, or lineage.",
      suggest: {
        ask: ["Summarize lakehouse data quality", "Show the lineage of a Gold mart", "Which datasets have quality issues?"],
        build: ["Refresh the lakehouse (Bronze→Silver→Gold)"],
      },
      system: "The user is in Governance (access rules, listed read-only and changed on a table's Access tab; data quality with its check schedules and freshness targets, lineage, audit). A column's sensitivity and masks are set on its table's Schema tab. Prefer get_quality / get_lineage.",
    },
  },
];

export function derivePageContext(pathname: string): PageContext {
  return MAP.find((m) => m.test(pathname))?.ctx ?? GENERIC;
}
