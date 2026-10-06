import { BarChart3, Database, FileCode2, GitBranch, Plug } from "lucide-react"

import type { Mode } from "@/features/copilot/use-copilot"
import type { ShortcutId } from "@/lib/home-layout"

/**
 * The "create" shortcuts under Home's composer, by id (`lib/home-layout`
 * owns which ids exist and which three are the default). Manual first:
 * `href` is the flow a person opens, and `aiPrompt` is what the sparkle
 * hands to Copilot (in `aiMode`, Build when not stated). A shortcut with no
 * `aiPrompt` renders without a sparkle.
 */
export type ShortcutDef = {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  href: string
  aiPrompt?: string
  aiMode?: Mode
}

export const SHORTCUTS: Record<ShortcutId, ShortcutDef> = {
  "connect-source": {
    icon: Plug,
    title: "Connect a source",
    description:
      "Register a database, stream or bucket. Credentials stay as secret references.",
    href: "/connectors/create",
    aiPrompt: "Guide me through connecting a new data source.",
  },
  "create-pipeline": {
    icon: GitBranch,
    title: "Create a pipeline",
    description:
      "Choose a source and a target layer, then review before it runs.",
    href: "/pipelines/create",
    aiPrompt:
      "Help me create a new pipeline. Ask me what to ingest and where it should land.",
  },
  "build-dashboard": {
    icon: BarChart3,
    title: "Build a dashboard",
    description:
      "Pick a mart and chart it. Every chart is added only after you confirm.",
    href: "/dashboards",
    aiPrompt: "Suggest a dashboard for the data we have.",
  },
  "new-query": {
    icon: FileCode2,
    title: "Write a query",
    description: "Open Query Studio to run SQL on your data.",
    href: "/query-studio",
    aiPrompt:
      "Help me write a SQL query over the data we have. Ask me what I want to find out.",
  },
  "browse-catalog": {
    icon: Database,
    title: "Browse the catalog",
    description: "See which datasets exist, where they come from and what they hold.",
    href: "/catalog",
    // A question, not a change: Ask mode, so nothing is built from it.
    aiPrompt: "Which datasets are in the catalog, and what is each one for?",
    aiMode: "ask",
  },
}
