"use client"

import * as React from "react"

import { SqlEditor } from "@/components/sql-editor"
import type { QueryEngine } from "@/services/contracts/queries"
import { QueryParameterFields } from "./query-parameter-fields"
import { StudioStatusBar } from "./studio-status-bar"
import { SQL_PLACEHOLDER, type useQueryStudio } from "./use-query-studio"

/**
 * The SQL editor, its `{{name}}` fields and the status bar under it, as one
 * box (`UX-9`, layout A with mockup B's status bar). What you can do to the
 * SQL is the toolbar's (`studio-toolbar.tsx`) and the command menu's.
 *
 * Run is bound to ⌘/Ctrl+Enter as well as the button — in an editor, the
 * keyboard is where the hands already are.
 */
export function SqlPanel({
  studio,
  onOpenPlan,
  availableEngines = ["clickhouse", "trino"],
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  /** Opens the side panel's Plan tab, from the estimate in the status bar. */
  readonly onOpenPlan?: () => void
  /**
   * The engines this installation can run (`BI-44`). A `bi-only` install
   * has exactly one; the default keeps every other caller's both-engine
   * selector unchanged. The API still refuses a stale engine at run time.
   */
  readonly availableEngines?: readonly QueryEngine[]
}) {
  const {
    sql,
    setSql,
    parameters,
    parameterValues,
    setParameterValue,
    setParameterDefinition,
    invalidParameters,
    activeTabId,
    editorExtensions,
    running,
    batchRefusal,
    runSelectionOrAll,
  } = studio

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      // Either modifier, like the command palette and the sidebar
      // shortcut: which one a keyboard has is not worth detecting.
      if (!(event.metaKey || event.ctrlKey) || event.key !== "Enter") return
      // The editor's own binding runs the selection and marks the key as
      // handled. Running the batch here as well would run it twice.
      if (event.defaultPrevented) return
      event.preventDefault()
      if (!running) void runSelectionOrAll()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [runSelectionOrAll, running])

  return (
    <div className="space-y-2">
      <div className="overflow-hidden rounded-xl bg-muted/35 dark:bg-card">
        {/* UX-4: the editor is this panel's one required input, so no star and
            no legend; the editor keeps its `required` association. UX-9: the
            label is for a reader who cannot see the box; the box and its
            line numbers already say what it is. */}
        <span id="query-sql-label" className="sr-only">
          SQL
        </span>
        {/* The editor gets the console's own completion source, not the
            language extension's schema: the tables and columns come from the
            catalog, under the names a query can actually use. */}
        <SqlEditor
          value={sql}
          onChange={setSql}
          placeholder={SQL_PLACEHOLDER}
          extensions={editorExtensions}
          onView={(view) => studio.setEditorView(view)}
          labelledBy="query-sql-label"
          required
          minHeight="196px"
          className="rounded-none border-0 bg-transparent [&_.cm-editor]:bg-transparent [&_.cm-gutters]:border-r-0 [&_.cm-gutters]:bg-transparent"
        />
        {/* One field per `{{name}}` in the SQL (`QS-5`); none, and nothing is drawn.
            The tab is the key: another tab starts its fields over. */}
        <QueryParameterFields
          key={activeTabId}
          definitions={parameters}
          values={parameterValues}
          invalid={invalidParameters}
          onValueChange={setParameterValue}
          onDefinitionChange={setParameterDefinition}
          className="px-3 pb-2"
        />
        <StudioStatusBar
          studio={studio}
          onOpenPlan={onOpenPlan ?? (() => {})}
          availableEngines={availableEngines}
        />
      </div>
      {/* QS-11 (LH-QS-1-12), D9: a batch over the limit is refused here, in
          the page, not in a toast that disappears before it is read. */}
      {batchRefusal ? (
        <p role="alert" className="text-sm text-destructive">
          {batchRefusal}
        </p>
      ) : null}
    </div>
  )
}
