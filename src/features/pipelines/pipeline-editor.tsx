"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter, useSearchParams } from "next/navigation"
import { AlertCircleIcon, CodeIcon, DatabaseIcon, ListChecksIcon, PlayIcon, PlusIcon, TableIcon } from "lucide-react"
import { FormPage } from "@/components/patterns/form-page"
import { EmptyState } from "@/components/patterns/page-states"
import { SelectField } from "@/components/patterns/select-field"
import { SqlColumnChips, SqlRowsTable } from "@/components/patterns/sql-rows-table"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  MAX_INPUTS,
  MAX_STEPS,
  READS_AS_YOU_NOTICE,
  addInput,
  addStep,
  canAddInput,
  canAddStep,
  columnsRead,
  definitionBody,
  definitionChanged,
  editorProblems,
  editorStateFrom,
  effectiveInputs,
  errorItemId,
  errorTarget,
  hasProblems,
  moveStep,
  previewAsRecords,
  previewBody,
  previewKey,
  readableNames,
  readsAsYouAfterSave,
  removeItem,
  renameItem,
  setInputTable,
  updateInput,
  updateStep,
  type Column,
  type EditorSelection,
  type EditorState,
} from "@/lib/pipeline-editor"
import { cn } from "@/lib/utils"
import { connectorService, lakehouseService, pipelineService } from "@/services"
import type { Pipeline, PipelineDetail, PipelinePreview, PipelinePreviewInput } from "@/services/contracts/pipelines"
import { connectorLoadTriggerOn, withConnectorLoadTrigger } from "./pipeline-connector-trigger"
import { InputPane, OutputPane, PaneProblem, SqlStepPane, TransformStepPane, type ConnectorSource } from "./pipeline-editor-panes"
import { isSchedulable, PipelineScheduleField } from "./pipeline-schedule-field"
import { BRONZE_ZONE, reuploadTriggerOn, showsReuploadSwitch, withReuploadTrigger } from "./pipeline-reupload-trigger"

/** The key a selection is filed under: the item's id, or `output`. */
function selectionKey(selection: EditorSelection): string {
  return selection.kind === "output" ? "output" : selection.id
}

/**
 * The pipeline editor behind both `/pipelines/create` and
 * `/pipelines/[pipelineId]/edit` (BLD-1 part 1 D17, decision 7). It replaces
 * the five-step wizard: a pipeline is inputs, steps and an output (D1), and
 * the page edits them as such.
 *
 * - `FormPage` is the shell; two tabs, **Build** and **Schedule and
 *   settings**. Build is a rail (Inputs, Steps, Output) and, beside it, the
 *   pane of the selected item above its preview, the chart builder's layout.
 * - Every save sends `inputs` and `steps`, never the fields of before (D3).
 *   A create sends `kind: "batch"` and an edit sends the stored kind back:
 *   kind and the incremental column left the page (decision 18), because
 *   nothing reads them and every run rebuilds the output (`BLD-12`).
 * - A refusal that names `inputs[i]` or `steps[i]` selects that item and
 *   shows the message in its pane (D7).
 * - The preview runs the selected input or step as the person looking
 *   (decision 13, D16) and says when the definition changed since it ran.
 * - Saving a change to the inputs or steps of a pipeline that reads as
 *   someone else says first that it will read as you from now on (D17a,
 *   decision 14).
 *
 * The connector (`BLD-7`, `BLD-9`) and the two trigger switches (`DATA-19`
 * D3, `BLD-7` decision 4) are on the second tab and concern the first
 * input, as the API's rules do (D12).
 */
export function PipelineEditor({ existing }: { existing?: PipelineDetail }) {
  const router = useRouter()
  const searchParams = useSearchParams()
  const { user } = useAuth()
  const def = existing?.definition ?? null
  const [tab, setTab] = React.useState<"build" | "settings">("build")
  const [name, setName] = React.useState(existing?.name ?? "")
  const [description, setDescription] = React.useState(existing?.description ?? "")
  const [editor, setEditor] = React.useState<EditorState>(() => editorStateFrom(def))
  const [selection, setSelection] = React.useState<EditorSelection>(() => {
    const first = editorStateFrom(def).inputs[0]
    return first ? { kind: "input", id: first.id } : { kind: "output" }
  })
  const [targetZone, setTargetZone] = React.useState(def?.targetZone ?? "silver")
  const [targetTable, setTargetTable] = React.useState(def?.targetTable ?? "")
  // BLD-10 D2: a copy with no step is legitimate, but it has to be chosen.
  // A stored pipeline with no step IS a plain copy, so it starts on; a new
  // one starts off.
  const [plainCopy, setPlainCopy] = React.useState(() => Boolean(existing) && (def?.steps.length ?? 0) === 0)
  // A cron by default: the free-text default used to be "Every hour", which
  // reads like a schedule but is not one the orchestrator can run.
  const [schedule, setSchedule] = React.useState(existing?.schedule ?? "0 * * * *")
  const connectorIdFromUrl = searchParams.get("connectorId") ?? ""
  const [connectorId, setConnectorId] = React.useState(def?.connectorId ?? connectorIdFromUrl)
  const hasConnector = connectorId.trim().length > 0
  // BLD-10 D1: FBIC does nothing in this version; a new pipeline sends it
  // off and an edit sends the stored value back unchanged. It is not on the
  // page (D17 lists what the second tab holds, and FBIC is not in it).
  const fbicEnabled = def?.fbicEnabled ?? false

  const connectors = useService((signal) => connectorService.listConnectors(signal), [])
  // BLD-7 decision 5: the connector's ingest targets, from the read the
  // connector page already uses (`GET /api/connectors/{id}/ingest-spec`).
  const connectorSpec = useService(
    (signal) => (hasConnector ? connectorService.getIngestSpec(connectorId.trim(), signal) : Promise.resolve(null)),
    [connectorId, hasConnector]
  )
  const connectorTargets = React.useMemo(() => {
    if (!hasConnector || connectorSpec.status !== "success" || connectorSpec.data === null) return null
    const targets = connectorSpec.data.sourceObjects.map((object) => object.target).filter(Boolean)
    return Array.from(new Set(targets)).sort()
  }, [hasConnector, connectorSpec.status, connectorSpec.data])
  const connector: ConnectorSource | null = hasConnector
    ? { connectorId: connectorId.trim(), targets: connectorTargets, failed: connectorSpec.status === "error" }
    : null
  // The inputs as they will be saved: with a connector, its own first input.
  const inputs = React.useMemo(
    () => effectiveInputs(editor.inputs, hasConnector ? (connectorTargets ?? []) : null),
    [editor.inputs, hasConnector, connectorTargets]
  )
  const shownState = React.useMemo(() => ({ ...editor, inputs }), [editor, inputs])
  const firstZone = inputs[0]?.zone.trim() ?? ""
  const firstTable = inputs[0]?.table.trim() ?? ""

  // DATA-19 D3 and BLD-7 decision 4: the two switches, each owning one
  // entry of the stored chain (`existing.dependsOn`); every other upstream
  // is carried through untouched.
  const [reuploadOn, setReuploadOn] = React.useState(() =>
    reuploadTriggerOn(existing?.dependsOn, def?.inputs[0]?.zone ?? "bronze", Boolean(existing))
  )
  const [connectorLoadOn, setConnectorLoadOn] = React.useState(() =>
    connectorLoadTriggerOn(existing?.dependsOn, hasConnector ? BRONZE_ZONE : (def?.inputs[0]?.zone ?? "bronze"), hasConnector, Boolean(existing))
  )
  function dependsOnFor(zone: string): string[] {
    return withConnectorLoadTrigger(
      withReuploadTrigger(existing?.dependsOn, zone, hasConnector ? false : reuploadOn),
      zone,
      hasConnector,
      connectorLoadOn
    )
  }

  // WEB-8 D8: zones are the lakehouse's namespaces; `null` while they are
  // read or when the read failed, because neither is "there are no zones".
  const namespaces = useService((signal) => lakehouseService.listNamespaces(undefined, signal), [])
  const zoneNames = namespaces.status === "success" ? namespaces.data.map((namespace) => namespace.name) : null
  const zoneNote =
    namespaces.status === "error" ? "Could not read the lakehouse's namespaces; type the zone name instead." : undefined
  const icebergZones = React.useMemo(() => new Set(zoneNames ?? []), [zoneNames])

  // Every input's columns with their types, from the read the table page
  // uses. `getTableDetail` answers for an Iceberg namespace only, so an
  // input in a zone it does not list keeps `null` (the fields stay free
  // text) instead of asking a question with no answer; a table that cannot
  // be read is `null` too.
  const columnsKey = JSON.stringify(inputs.map((input) => [input.id, input.zone.trim(), input.table.trim()]))
  const zonesKey = [...icebergZones].sort().join(",")
  const inputColumnsState = useService(
    async (signal) => {
      const wanted = JSON.parse(columnsKey) as [string, string, string][]
      const zones = new Set(zonesKey.split(","))
      const read = await Promise.all(
        wanted.map(async ([id, zone, table]): Promise<[string, Column[] | null]> => {
          if (!zones.has(zone) || table === "") return [id, null]
          try {
            const detail = await lakehouseService.getTableDetail(zone, table, signal)
            const columns = Array.isArray(detail.schema) ? detail.schema.map((field) => ({ name: field.name, type: field.type })) : []
            return [id, columns.length > 0 ? columns : null]
          } catch {
            return [id, null]
          }
        })
      )
      return Object.fromEntries(read) as Record<string, Column[] | null>
    },
    [columnsKey, zonesKey]
  )
  const inputColumns = React.useMemo<Record<string, Column[] | null>>(
    () => (inputColumnsState.status === "success" ? inputColumnsState.data : {}),
    [inputColumnsState.status, inputColumnsState.data]
  )

  // ── the preview (D16) ──────────────────────────────────────────────
  const preview = useServiceAction((signal, body: PipelinePreviewInput) => pipelineService.previewPipelineStep(body, signal))
  /** The item and the key of the preview that ran last. */
  const [ran, setRan] = React.useState<{ item: string; key: string } | null>(null)
  /** The last successful preview of each item, with the key it was made for. */
  const [results, setResults] = React.useState<Record<string, { key: string; data: PipelinePreview }>>({})
  React.useEffect(() => {
    if (preview.status !== "success" || ran === null) return
    const data = preview.data
    setResults((prev) => (prev[ran.item]?.data === data ? prev : { ...prev, [ran.item]: { key: ran.key, data } }))
  }, [preview.status, preview.data, ran])
  const currentBody = previewBody(inputs, editor.steps, selection)
  const currentKey = currentBody ? previewKey(currentBody) : null
  const itemKey = selectionKey(selection)
  async function runPreview() {
    if (!currentBody || currentKey === null) return
    setRan({ item: itemKey, key: currentKey })
    await preview.run(currentBody)
  }
  // A step's columns are known once it was previewed: they feed the pickers
  // of a form step that reads it.
  const stepColumns = React.useMemo(() => {
    const out: Record<string, Column[] | null> = {}
    for (const step of editor.steps) out[step.name] = results[step.id]?.data.columns ?? null
    return out
  }, [editor.steps, results])

  // ── checks and the save ────────────────────────────────────────────
  const problems = editorProblems(shownState, { targetZone, targetTable, plainCopy, hasConnector })
  const changed = def ? definitionChanged(inputs, editor.steps, def) : true
  const readsAsYou = Boolean(existing) && readsAsYouAfterSave(existing?.runAs, user?.id ?? null, changed)
  const save = useServiceAction(
    withNotify(
      existing
        ? { success: "Pipeline saved", error: "Failed to save pipeline" }
        : { success: "Pipeline created", error: "Failed to create pipeline" },
      (signal, body: ReturnType<typeof saveBody>): Promise<Pipeline> =>
        existing
          ? pipelineService.updatePipeline(
              existing.id,
              {
                // Decision 18: the stored kind goes back unchanged, and the
                // incremental column is not sent, which the API reads as
                // "leave it alone".
                kind: existing.kind,
                ...body.definition,
                fbicEnabled,
                targetZone: body.targetZone,
                targetTable: body.targetTable,
                schedule: body.schedule,
                description: body.description,
                // DATA-19 D3 + BLD-7 D4: sent on every save, so switching
                // one off CLEARS its stored upstream.
                dependsOn: body.dependsOn,
                // BLD-9 / decision 7: sent on every save, so "No connector"
                // clears the stored one (an omitted field means "unchanged").
                connectorId: connectorId.trim() || null,
              },
              signal
            )
          : pipelineService.createPipeline(
              {
                name: body.name,
                kind: "batch",
                ...body.definition,
                fbicEnabled,
                targetZone: body.targetZone,
                targetTable: body.targetTable,
                schedule: body.schedule,
                connectorId: connectorId.trim() || undefined,
                description: body.description || undefined,
                dependsOn: body.dependsOn,
              },
              signal
            )
    )
  )
  function saveBody() {
    return {
      name: name.trim(),
      definition: definitionBody(inputs, editor.steps),
      targetZone: targetZone.trim(),
      targetTable: targetTable.trim(),
      schedule: schedule.trim(),
      description: description.trim(),
      dependsOn: dependsOnFor(firstZone),
    }
  }
  // The ids of what was sent, so a refusal's `steps[1]` names the item it
  // meant even if the steps were moved since (D7).
  const [sent, setSent] = React.useState<Pick<EditorState, "inputs" | "steps"> | null>(null)
  async function handleSubmit() {
    setSent({ inputs, steps: editor.steps })
    const result = await save.run(saveBody())
    // Straight to the pipeline's own page, where it can be marked ready and
    // run; the list is one click away.
    if (result) router.push(`/pipelines/${encodeURIComponent(existing?.id ?? result.id)}`)
  }
  const saveError = save.status === "error" ? save.error.message : null
  const saveTarget = saveError ? errorTarget(saveError) : null
  const errorItem = sent ? errorItemId(sent, saveTarget) : null
  // D7: a refusal that names an item selects it, on the Build tab.
  // Only a new refusal moves the selection; editing afterwards does not.
  React.useEffect(() => {
    if (!errorItem) return
    const isInput = sent?.inputs.some((input) => input.id === errorItem) ?? false
    setSelection({ kind: isInput ? "input" : "step", id: errorItem })
    setTab("build")
  }, [errorItem, saveError, sent])

  const canSubmit = name.trim() !== "" && !hasProblems(problems) && schedule.trim() !== "" && isSchedulable(schedule)

  function select(next: EditorSelection) {
    setSelection(next)
  }
  function onAddInput() {
    const added = addInput(editor, hasConnector)
    if (added.id) {
      setEditor(added.state)
      select({ kind: "input", id: added.id })
    }
  }
  function onAddStep(kind: "transform" | "sql") {
    const added = addStep(editor, kind)
    if (!added.id) return
    setEditor(added.state)
    select({ kind: "step", id: added.id })
    // BLD-10 D2: a pipeline with a step is never also a plain copy.
    setPlainCopy(false)
  }
  function onRemove(id: string) {
    const next = removeItem(editor, id)
    setEditor(next)
    const first = next.inputs[0]
    select(first ? { kind: "input", id: first.id } : { kind: "output" })
  }

  // ── the pane of the selected item ──────────────────────────────────
  const errorMessageId = "pipeline-editor-item-error"
  function problemFor(id: string): React.ReactNode {
    if (errorItem === id && saveError) return <PaneProblem id={errorMessageId} message={saveError} tone="error" />
    const problem = problems.items[id]
    return problem ? <PaneProblem message={problem} tone="hint" /> : null
  }

  let pane: React.ReactNode = null
  let paneTitle = "Output"
  if (selection.kind === "input") {
    const index = editor.inputs.findIndex((input) => input.id === selection.id)
    const input = editor.inputs[index]
    if (input) {
      paneTitle = `Input ${input.name || index + 1}`
      pane = (
        <InputPane
          key={input.id}
          input={input}
          shown={inputs[index]}
          index={index}
          canRemove={editor.inputs.length > 1}
          connector={connector}
          zones={zoneNames}
          zoneNote={zoneNote}
          icebergZones={icebergZones}
          columns={inputColumns[input.id] ?? null}
          problem={problemFor(input.id)}
          onRename={(next) => setEditor((prev) => renameItem(prev, input.id, next))}
          onZone={(zone) => setEditor((prev) => updateInput(prev, input.id, { zone }))}
          onTable={(table) => setEditor((prev) => setInputTable(prev, input.id, table))}
          onRemove={() => onRemove(input.id)}
        />
      )
    }
  } else if (selection.kind === "step") {
    const index = editor.steps.findIndex((step) => step.id === selection.id)
    const step = editor.steps[index]
    if (step) {
      paneTitle = `Step ${step.name || index + 1}`
      const common = {
        index,
        count: editor.steps.length,
        problem: problemFor(step.id),
        onRename: (next: string) => setEditor((prev) => renameItem(prev, step.id, next)),
        onMove: (delta: -1 | 1) => setEditor((prev) => moveStep(prev, step.id, delta)),
        onRemove: () => onRemove(step.id),
      }
      pane =
        step.kind === "transform" ? (
          <TransformStepPane
            key={step.id}
            {...common}
            step={step}
            readable={readableNames(shownState, index)}
            columns={columnsRead(step, inputs, inputColumns, stepColumns)}
            readsStep={editor.steps.some((other) => other.name === step.reads)}
            failedTransform={errorItem === step.id && saveTarget?.kind === "step" ? saveTarget.transform : null}
            failedMessage={errorItem === step.id ? saveError : null}
            failedMessageId={errorMessageId}
            onChange={(patch) => setEditor((prev) => updateStep(prev, step.id, patch))}
          />
        ) : (
          <SqlStepPane
            key={step.id}
            {...common}
            step={step}
            state={shownState}
            inputColumns={inputColumns}
            onSql={(sql) => setEditor((prev) => updateStep(prev, step.id, { sql }))}
          />
        )
    }
  }
  if (pane === null) {
    const last = editor.steps[editor.steps.length - 1]
    pane = (
      <OutputPane
        targetZone={targetZone}
        targetTable={targetTable}
        zones={zoneNames}
        zoneNote={zoneNote}
        writtenFrom={last ? last.name || null : inputs.length === 1 ? inputs[0].name || null : null}
        plainCopy={plainCopy}
        plainCopyAllowed={editor.steps.length === 0}
        problem={
          problems.output ? <PaneProblem message={problems.output} tone="hint" /> : null
        }
        onZone={setTargetZone}
        onTable={setTargetTable}
        onPlainCopy={setPlainCopy}
      />
    )
  }

  // Decision 13: the preview reads as the person looking, never the author.
  const readsAsOther = Boolean(existing?.runAs) && existing?.runAs !== user?.id
  const result = results[itemKey]
  const thisRun = ran?.item === itemKey
  const previewPane = (
    <section aria-label="Preview" className="flex min-h-72 flex-col gap-3 border-t border-border px-5 py-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-medium">Preview of {currentBody?.upTo ?? "this item"}</p>
          <p className="text-xs text-muted-foreground">
            First 100 rows, as you may see them.
            {readsAsOther
              ? ` The pipeline reads as ${existing?.runAsName ?? "its author"}, so what it writes can differ.`
              : ""}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!currentBody || (thisRun && preview.status === "pending")}
          onClick={() => void runPreview()}
        >
          <PlayIcon aria-hidden />
          {thisRun && preview.status === "pending" ? "Running…" : result ? "Run again" : "Preview"}
        </Button>
      </div>
      {!currentBody ? (
        <p className="text-xs text-muted-foreground">Give it a name first; the preview runs it by its name.</p>
      ) : null}
      {thisRun && preview.status === "error" ? (
        <p role="alert" className="text-sm text-destructive">
          {preview.error.message}
        </p>
      ) : null}
      {result ? (
        <div className="flex min-h-0 flex-col gap-2">
          {result.key !== currentKey ? (
            <p role="status" className="text-xs font-medium text-amber-700 dark:text-amber-400">
              Stale: the definition changed since this preview ran.
            </p>
          ) : null}
          <SqlColumnChips result={previewAsRecords(result.data)} noRowsText="It returned no rows." />
          <SqlRowsTable
            result={previewAsRecords(result.data)}
            stale={result.key !== currentKey}
            staleText=" These are the rows from before your last edit."
            noRowsText="It returned no rows."
            className="max-h-80"
          />
          {result.data.truncated ? (
            <p className="text-xs text-muted-foreground">There are more rows; the preview shows the first 100.</p>
          ) : null}
        </div>
      ) : !(thisRun && preview.status === "error") ? (
        <EmptyState className="py-6" title="Not previewed yet" description="Run the preview to see its first rows." />
      ) : null}
    </section>
  )

  const backHref = existing ? `/pipelines/${encodeURIComponent(existing.id)}` : "/pipelines"
  return (
    <FormPage
      title={existing ? `Edit ${existing.name}` : "Create Pipeline"}
      description={
        existing
          ? "Change the inputs, steps, output or schedule. A ready pipeline keeps running with the new definition."
          : "A pipeline reads its inputs, runs its steps in order, and writes what the last step returns."
      }
      backHref={backHref}
      backLabel={existing ? "Back to the pipeline" : "Back to pipelines"}
      submitLabel={existing ? "Save changes" : "Create pipeline"}
      submittingLabel={existing ? "Saving…" : "Creating…"}
      canSubmit={canSubmit}
      submitting={save.status === "pending"}
      // A refusal that names an item is shown in that item's pane (D7).
      error={saveError && !errorItem ? saveError : null}
      onSubmit={() => void handleSubmit()}
      onCancel={() => router.push(backHref)}
      className="max-w-6xl"
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="pipeline-name">Pipeline name</Label>
          <Input
            id="pipeline-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="orders_hourly_rollup"
            disabled={Boolean(existing)}
            title={existing ? "The name is fixed: the pipeline's id and job derive from it" : undefined}
          />
        </div>
      </div>
      <Tabs value={tab} onValueChange={(next) => setTab(next === "settings" ? "settings" : "build")}>
        <TabsList variant="line" className="gap-4 border-b border-border pb-1">
          <TabsTrigger value="build">Build</TabsTrigger>
          <TabsTrigger value="settings">Schedule and settings</TabsTrigger>
        </TabsList>
        <TabsContent value="build" className="mt-3">
          <div className="grid min-h-[32rem] overflow-hidden rounded-xl border border-border md:grid-cols-[15rem_minmax(0,1fr)]">
            <nav aria-label="Pipeline parts" className="flex min-h-0 flex-col gap-4 border-b border-border bg-muted/30 p-3 md:border-r md:border-b-0">
              <RailGroup title={`Inputs · ${editor.inputs.length} of ${hasConnector ? 1 : MAX_INPUTS}`}>
                {inputs.map((input) => (
                  <RailItem
                    key={input.id}
                    icon={<DatabaseIcon aria-hidden />}
                    label={input.name || "Unnamed input"}
                    detail={input.table ? `${input.zone}.${input.table}` : "No table yet"}
                    selected={selection.kind === "input" && selection.id === input.id}
                    flagged={errorItem === input.id || Boolean(problems.items[input.id])}
                    describedBy={errorItem === input.id ? errorMessageId : undefined}
                    onSelect={() => select({ kind: "input", id: input.id })}
                  />
                ))}
                {/* D2 and D12: past the limit the action is not offered at all. */}
                {canAddInput(editor, hasConnector) ? (
                  <Button type="button" variant="ghost" size="sm" className="justify-start" onClick={onAddInput}>
                    <PlusIcon aria-hidden />
                    Add input
                  </Button>
                ) : null}
              </RailGroup>
              <RailGroup title={`Steps · ${editor.steps.length} of ${MAX_STEPS}`}>
                {editor.steps.length === 0 ? (
                  <p className="px-2 text-xs text-muted-foreground">No steps: the output is a copy of the input.</p>
                ) : null}
                {editor.steps.map((step, index) => (
                  <RailItem
                    key={step.id}
                    icon={step.kind === "sql" ? <CodeIcon aria-hidden /> : <ListChecksIcon aria-hidden />}
                    label={`${index + 1}. ${step.name || "Unnamed step"}`}
                    detail={step.kind === "sql" ? "SQL" : `Form · reads ${step.reads || "…"}`}
                    selected={selection.kind === "step" && selection.id === step.id}
                    flagged={errorItem === step.id || Boolean(problems.items[step.id])}
                    describedBy={errorItem === step.id ? errorMessageId : undefined}
                    onSelect={() => select({ kind: "step", id: step.id })}
                  />
                ))}
                {canAddStep(editor) ? (
                  <div className="flex flex-wrap gap-1">
                    <Button type="button" variant="ghost" size="sm" onClick={() => onAddStep("sql")}>
                      <PlusIcon aria-hidden />
                      SQL step
                    </Button>
                    <Button type="button" variant="ghost" size="sm" onClick={() => onAddStep("transform")}>
                      <PlusIcon aria-hidden />
                      Form step
                    </Button>
                  </div>
                ) : null}
              </RailGroup>
              <RailGroup title="Output">
                <RailItem
                  icon={<TableIcon aria-hidden />}
                  label={targetTable ? `${targetZone}.${targetTable}` : "No table yet"}
                  detail="Rebuilt every run"
                  selected={selection.kind === "output"}
                  flagged={problems.output !== null}
                  onSelect={() => select({ kind: "output" })}
                />
              </RailGroup>
            </nav>
            <div className="flex min-h-0 min-w-0 flex-col">
              <section aria-label={paneTitle} className="px-5 py-4">
                {pane}
              </section>
              {previewPane}
            </div>
          </div>
        </TabsContent>
        <TabsContent value="settings" className="mt-3">
          <div className="grid gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="pipeline-description">Description</Label>
              {/* Stored and shown on the pipeline's page. */}
              <Textarea
                id="pipeline-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                rows={2}
                placeholder="What this pipeline is for, and anything the next person should know."
              />
            </div>
            <div className="space-y-1.5">
              <Label>Schedule</Label>
              <PipelineScheduleField value={schedule} onChange={setSchedule} hasTargetLag={Boolean(existing?.targetLag)} />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="pipeline-connector">Connector</Label>
              {!existing && connectorIdFromUrl ? (
                // The connector page's own entry: the connector is the page
                // the user came from, shown with a way back rather than a
                // picker that could silently drop it.
                <div className="flex items-center justify-between rounded-lg border border-border px-3 py-1.5 text-sm">
                  <span className="font-mono">{connectorIdFromUrl}</span>
                  <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
                    Change
                  </Button>
                </div>
              ) : (
                // BLD-9 / decision 7: "No connector" is a real choice that
                // sends `null`. A pipeline with a connector reads one of its
                // Bronze tables as its one input (decision 5, D12).
                <SelectField
                  id="pipeline-connector"
                  className="w-full"
                  aria-label="Connector"
                  value={connectorId}
                  options={[
                    { value: "", label: "No connector (read a ClickHouse table directly)" },
                    ...(connectors.data ?? []).map((c) => ({ value: c.id, label: c.name })),
                  ]}
                  onValueChange={setConnectorId}
                />
              )}
            </div>
            {hasConnector ? (
              // BLD-7 decision 4: a pipeline made from the connector page runs
              // after that connector loads its table, and the user may turn it
              // off. A connector always loads Bronze.
              <TriggerRow
                title="Run after this connector loads the table"
                detail={
                  <>
                    When the connector&apos;s ingest loads <span className="font-mono">{BRONZE_ZONE}.{firstTable || "…"}</span>,
                    this pipeline runs. Each pipeline waits for its own table only.
                  </>
                }
                checked={connectorLoadOn}
                onChange={setConnectorLoadOn}
              />
            ) : showsReuploadSwitch(firstZone) ? (
              <TriggerRow
                title="Run again after this table is re-uploaded"
                detail={
                  <>
                    Uploads load into <span className="font-mono">{firstZone}.{firstTable || "…"}</span>, the first
                    input; with this on, uploading it again runs this pipeline.
                  </>
                }
                checked={reuploadOn}
                onChange={setReuploadOn}
              />
            ) : null}
          </div>
        </TabsContent>
      </Tabs>
      {readsAsYou ? (
        // D17a: said before the save, beside it, not after.
        <p role="status" className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400">
          {READS_AS_YOU_NOTICE}
        </p>
      ) : null}
    </FormPage>
  )
}

/** One part of the rail, a labelled group so a screen reader says which part an item is in. */
function RailGroup({ title, children }: { title: string; children: React.ReactNode }) {
  const id = React.useId()
  return (
    <div role="group" aria-labelledby={id} className="flex flex-col gap-1">
      <p id={id} className="px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </p>
      {children}
    </div>
  )
}

/**
 * One item of the rail: a button, so it is reached with Tab and chosen with
 * Enter or Space; `aria-current` says which one the pane shows. An item the
 * check or the API found a problem with is marked, and when the API named
 * it, the button is described by the message in its pane (D7).
 */
function RailItem({
  icon,
  label,
  detail,
  selected,
  flagged,
  describedBy,
  onSelect,
}: {
  icon: React.ReactNode
  label: string
  detail: string
  selected: boolean
  flagged: boolean
  describedBy?: string
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      aria-describedby={describedBy}
      onClick={onSelect}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4 [&_svg]:shrink-0",
        selected ? "bg-primary/10 font-medium text-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground"
      )}
    >
      <span className={selected ? "text-primary" : "opacity-70"}>{icon}</span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono">{label}</span>
        <span className="block truncate text-[11px] text-muted-foreground">{detail}</span>
      </span>
      {flagged ? (
        <>
          <AlertCircleIcon className="text-amber-600 dark:text-amber-400" aria-hidden />
          <span className="sr-only">needs attention</span>
        </>
      ) : null}
    </button>
  )
}

/** One trigger switch with its sentence. `aria-label`, because the text beside a toggle is a description, not a caption. */
function TriggerRow({
  title,
  detail,
  checked,
  onChange,
}: {
  title: string
  detail: React.ReactNode
  checked: boolean
  onChange: (on: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="text-xs text-muted-foreground">{detail}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} aria-label={title} />
    </div>
  )
}
