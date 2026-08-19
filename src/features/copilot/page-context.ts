/**
 * Page-aware context for the AI Copilot. Derives what the user is likely doing
 * from the current route so the dock/page can tailor its greeting, suggestions,
 * and the system prompt sent to the model. Specific pages (e.g. a dashboard)
 * can override this with richer entity context via setPageContext().
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
    ask: ["Total foreign visitors by region", "Which datasets are about halal?", "Summarize lakehouse data quality"],
    build: ["Create a visitors-by-region chart", "Build a visitors dashboard", "Check the latest build status"],
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
        ask: ["Summarize platform health", "Which datasets are stale?", "Total foreign visitors by region"],
        build: ["Build a visitors dashboard", "Refresh the lakehouse (Bronze→Silver→Gold)"],
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
        ask: ["Explain the charts on this dashboard", "Total foreign visitors by region"],
        build: ["Create a chart of visitors by region", "Add a KPI of total visitors", "Build a new visitors dashboard"],
      },
      system: "The user is on the Dashboards page. They most likely want to create charts, create/edit a dashboard, or understand what's shown. Prefer create_chart / create_board / update_chart.",
    },
  },
  {
    test: (p) => p.startsWith("/data") || p.startsWith("/catalog") || p.startsWith("/storage"),
    ctx: {
      key: "data",
      title: "Explore or build data",
      hint: "Ask about datasets, schemas, lineage — or refresh/build data.",
      suggest: {
        ask: ["What datasets are about halal?", "Describe the wisman dataset", "Show data lineage of visitors by country"],
        build: ["Refresh the lakehouse (Bronze→Silver→Gold)", "Check the latest build status"],
      },
      system: "The user is exploring Data / Catalog / Storage. Help them query, describe datasets, inspect lineage/quality, or build/refresh data.",
    },
  },
  {
    test: (p) => p.startsWith("/pipelines") || p.startsWith("/streaming"),
    ctx: {
      key: "pipelines",
      title: "Run or inspect pipelines",
      hint: "Ask about pipeline runs, or build/refresh the lakehouse.",
      suggest: {
        ask: ["Check the latest build status", "Summarize lakehouse data quality"],
        build: ["Rebuild the lakehouse (Bronze→Silver→Gold)", "Refresh the culinary mart"],
      },
      system: "The user is on Pipelines / Streaming. They may want to run pipelines, check build status, or build/refresh data.",
    },
  },
  {
    test: (p) => p.startsWith("/query"),
    ctx: {
      key: "query",
      title: "Write and run SQL",
      hint: "Ask a data question — Copilot writes and runs the SQL.",
      suggest: {
        ask: ["Total foreign visitors by region", "Top 10 source countries", "Monthly visitor trend"],
        build: ["Turn this into a chart on a dashboard"],
      },
      system: "The user is in Query Studio. Prefer run_sql; offer to turn results into a chart.",
    },
  },
  {
    test: (p) => p.startsWith("/governance") || p.startsWith("/lineage") || p.startsWith("/audit") || p.startsWith("/residency"),
    ctx: {
      key: "governance",
      title: "Governance & quality",
      hint: "Ask about data quality, lineage, or classification.",
      suggest: {
        ask: ["Summarize lakehouse data quality", "Show data lineage of visitors by country", "Which datasets have quality issues?"],
        build: ["Refresh the lakehouse (Bronze→Silver→Gold)"],
      },
      system: "The user is in Governance (quality, lineage, audit, residency). Prefer get_quality / get_lineage.",
    },
  },
];

export function derivePageContext(pathname: string): PageContext {
  return MAP.find((m) => m.test(pathname))?.ctx ?? GENERIC;
}
