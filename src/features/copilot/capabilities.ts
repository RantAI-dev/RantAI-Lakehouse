import type { LucideIcon } from "lucide-react";
import { Database, BarChart3, GitBranch } from "lucide-react";
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
    desc: "Build/refresh Bronze→Silver→Gold",
    icon: GitBranch,
    write: true,
    tools: ["get_build_status", "trigger_lakehouse_build"],
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
