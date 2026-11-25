"use client"

import {
  BookOpen,
  Bot,
  Database,
  FileCode2,
  Gauge,
  Keyboard,
  LayoutDashboard,
  ListTree,
  PanelRight,
  Play,
  Plus,
  RefreshCw,
  Rewind,
  Save,
  Settings2,
  Square,
  Undo2,
  WandSparkles,
  type LucideIcon,
} from "lucide-react"

import { MAX_QUERY_TABS } from "@/lib/query-draft"
import { queryEngineLabel } from "@/lib/status"
import type { useQueryStudio } from "./use-query-studio"

/** The side panel's two tabs. */
export type PanelTab = "schema" | "plan"

/**
 * One thing a person can do to the query in the editor. The toolbar's menus
 * and the command menu (⌘⇧P) both read this one list (`UX-9`), so an action
 * is labelled, enabled and explained the same way wherever it is offered,
 * and the command menu cannot drift from the menus it repeats.
 */
export type StudioAction = {
  readonly id: StudioActionId
  readonly label: string
  readonly icon: LucideIcon
  /** The command menu's group. */
  readonly group: "Query" | "Edit" | "Data" | "View"
  /** The key that does the same, shown beside the label. */
  readonly hint?: string
  readonly disabled?: boolean
  /** Why it is disabled, as the item's title. */
  readonly reason?: string
  readonly run: () => void
}

export type StudioActionId =
  | "run"
  | "stop"
  | "save"
  | "save-as-new"
  | "save-source"
  | "new-tab"
  | "format"
  | "format-settings"
  | "explain"
  | "mode"
  | "engine"
  | "time-travel"
  | "unpin"
  | "estimate"
  | "show-schema"
  | "show-plan"
  | "hide-panel"
  | "shortcuts"

/** What the page hands the list: the studio, and the page's own controls. */
export type StudioActionContext = {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly onSave: () => void
  readonly onSaveAsNew: () => void
  /** Present only when the person may author dashboard SQL sources. */
  readonly onSaveAsSource?: () => void
  readonly onFormat: () => void
  readonly onOpenFormatSettings: () => void
  readonly onOpenTimeTravel: () => void
  /** Present only while the SQL holds a version pin. */
  readonly onUnpin?: () => void
  readonly onOpenShortcuts: () => void
  readonly panelOpen: boolean
  readonly onShowPanel: (tab: PanelTab) => void
  readonly onHidePanel: () => void
  /** Run's key as this platform writes it (`useStudioKeys`), once it is known. */
  readonly runKey?: string
}

const SELECT_ONE = "Select one statement"

/** Every editor action, in the order the command menu lists them. */
export function studioActions(ctx: StudioActionContext): StudioAction[] {
  const { studio } = ctx
  const oneStatement = studio.targetStatement !== null
  const busy = studio.running || studio.download.busy !== null
  const otherEngine = studio.engine === "clickhouse" ? "trino" : "clickhouse"
  const actions: (StudioAction | null)[] = [
    busy
      ? {
          id: "stop",
          label: "Stop",
          icon: Square,
          group: "Query",
          run: studio.stopRun,
        }
      : {
          id: "run",
          label: studio.statements.length > 1 ? "Run all" : "Run",
          icon: Play,
          group: "Query",
          hint: ctx.runKey,
          disabled: !studio.runnable,
          run: () => void studio.runSelectionOrAll(),
        },
    {
      id: "save",
      label: "Save",
      icon: Save,
      group: "Query",
      disabled: !oneStatement,
      reason: oneStatement ? undefined : SELECT_ONE,
      run: ctx.onSave,
    },
    // A tab that did not come from a saved query has one save: Save makes
    // the new query. "Save as new" is offered only beside a Save that
    // changes a saved one (`DESIGN.md` §3, one action, one button).
    studio.saveTarget.mode === "update"
      ? {
          id: "save-as-new",
          label: "Save as new…",
          icon: Save,
          group: "Query",
          disabled: !oneStatement,
          reason: oneStatement ? undefined : SELECT_ONE,
          run: ctx.onSaveAsNew,
        }
      : null,
    ctx.onSaveAsSource
      ? {
          id: "save-source",
          label: "Save as SQL source",
          icon: LayoutDashboard,
          group: "Query",
          disabled: !oneStatement,
          reason: oneStatement ? undefined : SELECT_ONE,
          run: ctx.onSaveAsSource,
        }
      : null,
    {
      id: "new-tab",
      label: "New query tab",
      icon: Plus,
      group: "Query",
      disabled: studio.tabs.length >= MAX_QUERY_TABS,
      reason:
        studio.tabs.length >= MAX_QUERY_TABS
          ? `At most ${MAX_QUERY_TABS} query tabs are kept. Close a tab to open another.`
          : undefined,
      run: () => void studio.addTab(),
    },
    {
      id: "format",
      label: "Format",
      icon: WandSparkles,
      group: "Edit",
      disabled: !studio.runnable,
      run: ctx.onFormat,
    },
    {
      id: "format-settings",
      label: "Format settings…",
      icon: Settings2,
      group: "Edit",
      run: ctx.onOpenFormatSettings,
    },
    {
      id: "explain",
      label: studio.explain.busy ? "Explaining…" : "Explain with AI",
      icon: BookOpen,
      group: "Edit",
      disabled: studio.explain.busy || !oneStatement,
      reason: oneStatement ? undefined : SELECT_ONE,
      run: () => void studio.explain.run(),
    },
    {
      id: "mode",
      label: studio.tab === "sql" ? "Switch to Ask" : "Switch to SQL",
      icon: studio.tab === "sql" ? Bot : FileCode2,
      group: "Edit",
      run: () => studio.setTab(studio.tab === "sql" ? "nl" : "sql"),
    },
    {
      id: "engine",
      label: `Run on ${queryEngineLabel(otherEngine)}`,
      icon: Database,
      group: "Data",
      run: () => studio.setEngine(otherEngine),
    },
    {
      id: "time-travel",
      label: "Time travel…",
      icon: Rewind,
      group: "Data",
      run: ctx.onOpenTimeTravel,
    },
    ctx.onUnpin
      ? {
          id: "unpin",
          label: "Read the current version",
          icon: Undo2,
          group: "Data",
          run: ctx.onUnpin,
        }
      : null,
    {
      id: "estimate",
      label: "Estimate again",
      icon: RefreshCw,
      group: "Data",
      disabled:
        !studio.estimateSupported ||
        !studio.estimatable ||
        studio.estimateAct.status === "pending",
      reason: !studio.estimateSupported
        ? "No pre-run estimate for Trino queries."
        : oneStatement
          ? undefined
          : SELECT_ONE,
      run: studio.reEstimate,
    },
    {
      id: "show-schema",
      label: "Show schema",
      icon: ListTree,
      group: "View",
      run: () => ctx.onShowPanel("schema"),
    },
    {
      id: "show-plan",
      label: "Show plan and estimate",
      icon: Gauge,
      group: "View",
      run: () => ctx.onShowPanel("plan"),
    },
    ctx.panelOpen
      ? {
          id: "hide-panel",
          label: "Hide side panel",
          icon: PanelRight,
          group: "View",
          run: ctx.onHidePanel,
        }
      : null,
    {
      id: "shortcuts",
      label: "Keyboard shortcuts",
      icon: Keyboard,
      group: "View",
      run: ctx.onOpenShortcuts,
    },
  ]
  return actions.filter((action): action is StudioAction => action !== null)
}

/** The actions with these ids, in the order the ids are given. */
export function pickActions(
  actions: readonly StudioAction[],
  ids: readonly StudioActionId[]
): StudioAction[] {
  return ids.flatMap((id) => actions.filter((action) => action.id === id))
}
