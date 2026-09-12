import type { LucideIcon } from "lucide-react";
import { Database, BarChart3, GitBranch, Bell, Plug, Bookmark, ShieldCheck, Server } from "lucide-react";
import type { Mode } from "./use-copilot";

/**
 * Kapabilitas Copilot tingkat-tinggi (SESUAI MENU) — biar user tak bingung
 * dengan 15 nama tool teknis. Tiap kapabilitas membungkus beberapa tool nyata
 * di baliknya. Menu "Tools" di composer menampilkan ini, bukan tool mentah.
 * `write: true` = mengubah sesuatu → hanya di mode Build.
 */
export type Capability = {
  key: string;
  label: string;
  desc: string;
  icon: LucideIcon;
  write?: boolean;
  tools: string[]; // nama tool di services/clients/ai-tools.ts
};

export const CAPABILITIES: Capability[] = [
  {
    key: "data",
    label: "Query Data",
    desc: "Query & explore catalog, lineage, quality",
    icon: Database,
    tools: ["run_sql", "list_datasets", "describe_dataset", "get_lineage", "get_quality", "describe_mart"],
  },
  {
    key: "dashboard",
    label: "Dashboard Builder",
    desc: "Create & manage charts / boards",
    icon: BarChart3,
    write: true,
    tools: ["describe_mart", "list_charts", "list_boards", "suggest_dashboard", "create_chart", "update_chart", "delete_chart", "create_board"],
  },
  {
    key: "pipeline",
    label: "Pipeline Builder",
    desc: "Build/refresh Bronze→Silver→Gold, and manage individual pipelines & runs",
    icon: GitBranch,
    write: true,
    tools: [
      "get_build_status",
      "trigger_lakehouse_build",
      "list_pipelines",
      "list_pipeline_runs",
      "trigger_pipeline",
      "retry_pipeline_run",
      "pause_pipeline",
      "resume_pipeline",
      "cancel_pipeline_run",
    ],
  },
  {
    key: "alerts",
    label: "Alerts",
    desc: "View, create & run threshold alerts and digests",
    icon: Bell,
    write: true,
    tools: [
      "list_alert_rules",
      "create_alert_rule",
      "update_alert_rule",
      "delete_alert_rule",
      "run_alert_rule",
    ],
  },
  {
    key: "connectors",
    label: "Connectors",
    desc: "Register, test & remove source/sink connectors",
    icon: Plug,
    write: true,
    tools: ["list_connectors", "create_connector", "test_connector", "delete_connector"],
  },
  {
    key: "queries",
    label: "Saved Queries",
    desc: "Save & re-run named SQL queries",
    icon: Bookmark,
    write: true,
    tools: ["save_query", "list_saved_queries", "run_saved_query"],
  },
  {
    key: "governance",
    label: "Governance",
    desc: "Audit history, quality/classification rules, CDC & maintenance health, draft new policies/rules",
    icon: ShieldCheck,
    write: true,
    tools: [
      "get_audit_history",
      "list_classification_rules",
      "list_quality_rules",
      "get_cdc_health",
      "get_maintenance_metrics",
      "run_bronze_maintenance",
      "draft_policy",
      "draft_classification_rule",
      "draft_quality_rule",
    ],
  },
  {
    key: "operations",
    label: "Operations",
    desc: "Inspect running workloads, kill queries, export & read back Gold marts",
    icon: Server,
    write: true,
    tools: ["list_workloads", "kill_query", "export_gold_mart", "get_gold_export"],
  },
];

/** Kapabilitas yang tersedia untuk sebuah mode (Ask sembunyikan yang menulis). */
export function capsForMode(mode: Mode): Capability[] {
  return mode === "build" ? CAPABILITIES : CAPABILITIES.filter((c) => !c.write);
}

/** Union nama tool dari kapabilitas yang aktif & sesuai mode. */
export function toolsFromCaps(enabled: Set<string>, mode: Mode): string[] {
  const names = new Set<string>();
  for (const cap of capsForMode(mode)) {
    if (enabled.has(cap.key)) cap.tools.forEach((t) => names.add(t));
  }
  return [...names];
}

export const ALL_CAP_KEYS = CAPABILITIES.map((c) => c.key);
