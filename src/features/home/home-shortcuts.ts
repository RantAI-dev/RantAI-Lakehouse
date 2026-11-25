import { BarChart3, Database, FileCode2, GitBranch, Plug } from "lucide-react"

import type { ShortcutId } from "@/lib/home-layout"

/**
 * The "create" shortcuts on Home, by id (`lib/home-layout` owns which ids
 * exist and which three are the default): `href` is the manual flow a
 * person opens, `description` its tooltip. Each one used to carry its own
 * "let AI do it" button and prompt; since `WEB-17` the composer above them
 * is the one place AI starts, so they are plain links, shown compactly.
 */
export type ShortcutDef = {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  href: string
}

export const SHORTCUTS: Record<ShortcutId, ShortcutDef> = {
  "connect-source": {
    icon: Plug,
    title: "Connect a source",
    description:
      "Register a database, stream or bucket. Credentials stay as secret references.",
    href: "/connectors/create",
  },
  "create-pipeline": {
    icon: GitBranch,
    title: "Create a pipeline",
    description:
      "Choose a source and a target layer, then review before it runs.",
    href: "/pipelines/create",
  },
  "build-dashboard": {
    icon: BarChart3,
    title: "Build a dashboard",
    description:
      "Pick a mart and chart it. Every chart is added only after you confirm.",
    href: "/dashboards",
  },
  "new-query": {
    icon: FileCode2,
    title: "Write a query",
    description: "Open Query Studio to run SQL on your data.",
    href: "/query-studio",
  },
  "browse-catalog": {
    icon: Database,
    title: "Browse the catalog",
    description: "See which datasets exist, where they come from and what they hold.",
    href: "/catalog",
  },
}
