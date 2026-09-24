"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter, useSearchParams } from "next/navigation"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  CAST_TYPES,
  FILTER_OPERATORS,
  renderTransformDraft,
  transformErrorRowIndex,
  type TransformDraft,
} from "@/lib/transform-draft"
import { cn } from "@/lib/utils"
import { connectorService, pipelineService } from "@/services"
import type { PipelineKind } from "@/services/contracts/pipelines"

const STEPS: FormStep[] = [
  { id: "source", label: "Source", description: "Name and source table" },
  { id: "transform", label: "Transform", description: "Grammar-checked transform steps" },
  { id: "target", label: "Target", description: "Destination table" },
  { id: "schedule", label: "Schedule", description: "Trigger and ownership" },
  { id: "review", label: "Review", description: "Confirm and create" },
]

const KIND_OPTIONS: PipelineKind[] = ["batch", "incremental"]

/** The five verbs `transform_grammar.rs::parse_transform` accepts — nothing else is offered. */
const TRANSFORM_VERBS: TransformDraft["verb"][] = ["dedupe", "filter", "rename", "cast", "select"]

function emptyDraftFor(verb: TransformDraft["verb"]): TransformDraft {
  switch (verb) {
    case "dedupe":
      return { verb: "dedupe", key: "" }
    case "filter":
      return { verb: "filter", column: "", operator: FILTER_OPERATORS[0], value: "" }
    case "rename":
      return { verb: "rename", from: "", to: "" }
    case "cast":
      return { verb: "cast", column: "", type: CAST_TYPES[0] }
    case "select":
      return { verb: "select", columns: "" }
  }
}

/** A draft is only addable once its own required fields are non-blank — never sent to the server half-filled. */
function draftIsComplete(draft: TransformDraft): boolean {
  switch (draft.verb) {
    case "dedupe":
      return draft.key.trim().length > 0
    case "filter":
      return draft.column.trim().length > 0 && draft.value.trim().length > 0
    case "rename":
      return draft.from.trim().length > 0 && draft.to.trim().length > 0
    case "cast":
      return draft.column.trim().length > 0
    case "select":
      return draft.columns.trim().length > 0
  }
}

export function PipelineCreatePage() {
  const router = useRouter()
  const searchParams = useSearchParams()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [kind, setKind] = React.useState<PipelineKind>("incremental")
  const [sourceZone, setSourceZone] = React.useState("bronze")
  const [sourceTable, setSourceTable] = React.useState("")
  const [incrementalColumn, setIncrementalColumn] = React.useState("updated_at")
  const [transformDrafts, setTransformDrafts] = React.useState<TransformDraft[]>([])
  const [pendingVerb, setPendingVerb] = React.useState<TransformDraft["verb"]>("dedupe")
  const [pendingDraft, setPendingDraft] = React.useState<TransformDraft>(emptyDraftFor("dedupe"))
  const [fbicEnabled, setFbicEnabled] = React.useState(false)
  const [targetZone, setTargetZone] = React.useState("silver")
  const [targetTable, setTargetTable] = React.useState("")
  const [schedule, setSchedule] = React.useState("Every hour")
  const connectorIdFromUrl = searchParams.get("connectorId") ?? ""
  const [connectorId, setConnectorId] = React.useState(connectorIdFromUrl)
  const connectors = useService(
    (signal) => connectorService.listConnectors(signal),
    []
  )
  const [description, setDescription] = React.useState("")
  const create = useServiceAction(
    withNotify(
      { success: "Pipeline created", error: "Failed to create pipeline" },
      (signal, input: Parameters<typeof pipelineService.createPipeline>[0]) =>
        pipelineService.createPipeline(input, signal)
    )
  )

  // The 400 body names the failing row as `transforms[<index>]` (see
  // routes/pipelines.rs::create) — surface it against that row instead of
  // only the page-level generic message.
  const failedTransformIndex =
    create.status === "error" ? transformErrorRowIndex(create.error.message) : null

  const canProceed = React.useMemo(() => {
    if (step === 0) {
      return Boolean(name.trim() && sourceZone.trim() && sourceTable.trim() && incrementalColumn.trim())
    }
    if (step === 1) return transformDrafts.length > 0 || fbicEnabled
    if (step === 2) return Boolean(targetZone.trim() && targetTable.trim())
    if (step === 3) return Boolean(schedule.trim())
    return true
  }, [
    step,
    name,
    sourceZone,
    sourceTable,
    incrementalColumn,
    transformDrafts,
    fbicEnabled,
    targetZone,
    targetTable,
    schedule,
  ])

  function addTransformDraft() {
    if (!draftIsComplete(pendingDraft)) return
    setTransformDrafts((prev) => [...prev, pendingDraft])
    setPendingDraft(emptyDraftFor(pendingVerb))
  }

  function removeTransformDraft(index: number) {
    setTransformDrafts((prev) => prev.filter((_, i) => i !== index))
  }

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      kind,
      sourceZone: sourceZone.trim(),
      sourceTable: sourceTable.trim(),
      incrementalColumn: incrementalColumn.trim(),
      transforms: transformDrafts.map(renderTransformDraft),
      fbicEnabled,
      targetZone: targetZone.trim(),
      targetTable: targetTable.trim(),
      schedule: schedule.trim(),
      connectorId: connectorId || undefined,
      description: description.trim() || undefined,
    })
    if (result) router.push("/pipelines")
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Create Pipeline"
        description="Step through source, transform, target, and schedule to draft a pipeline."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/pipelines" />}>
            Cancel
          </Button>
        }
      />
      <FormStepLayout
        steps={STEPS}
        currentIndex={step}
        onStepChange={setStep}
        canProceed={canProceed}
        onSubmit={handleSubmit}
        submitLabel="Create pipeline"
        submitting={create.status === "pending"}
      >
        {step === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Pipeline name" className="sm:col-span-2">
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="orders_hourly_rollup" />
            </Field>
            <Field label="Description" className="sm:col-span-2">
              {/* Stored and shown on the pipeline's page, which used to
                  display one fixed sentence for every pipeline. */}
              <Textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                rows={2}
                placeholder="What this pipeline is for, and anything the next person should know."
              />
            </Field>
            <Field label="Kind">
              {/* The design system's Select, not a hand-styled native one:
                  this was the only picker in the console that did not
                  match the others. */}
              <Select
                value={kind}
                onValueChange={(v) => setKind((v ?? "batch") as PipelineKind)}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Pick a kind" />
                </SelectTrigger>
                <SelectContent>
                  {KIND_OPTIONS.map((k) => (
                    <SelectItem key={k} value={k}>
                      {k}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field label="Incremental column">
              <Input
                value={incrementalColumn}
                onChange={(e) => setIncrementalColumn(e.target.value)}
              />
            </Field>
            <Field label="Source zone">
              <Input value={sourceZone} onChange={(e) => setSourceZone(e.target.value)} />
            </Field>
            <Field label="Connector" className="sm:col-span-2">
              {connectorIdFromUrl ? (
                <div className="flex items-center justify-between rounded-lg border border-border px-3 py-1.5 text-sm">
                  <span className="font-mono">{connectorIdFromUrl}</span>
                  <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
                    Change
                  </Button>
                </div>
              ) : (
                <select
                  className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                  value={connectorId}
                  onChange={(e) => setConnectorId(e.target.value)}
                >
                  <option value="">No connector (read a ClickHouse table directly)</option>
                  {(connectors.data ?? []).map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              )}
            </Field>
            <Field label="Source table">
              <Input
                value={sourceTable}
                onChange={(e) => setSourceTable(e.target.value)}
                placeholder="orders_events"
              />
            </Field>
          </div>
        ) : null}

        {step === 1 ? (
          <div className="space-y-4">
            <div>
              <p className="mb-2 text-sm font-medium">Transforms</p>
              <p className="mb-2 text-xs text-muted-foreground">
                Only the verbs and operators the server&apos;s transform grammar accepts are offered
                here — the request is still re-validated server-side on submit.
              </p>
              {transformDrafts.length > 0 ? (
                <ul className="mb-3 space-y-1">
                  {transformDrafts.map((draft, index) => (
                    <li
                      key={index}
                      className={cn(
                        "flex items-center justify-between gap-2 rounded-lg border px-3 py-1.5 text-xs",
                        failedTransformIndex === index
                          ? "border-destructive bg-destructive/10"
                          : "border-border"
                      )}
                    >
                      <span className="font-mono">{renderTransformDraft(draft)}</span>
                      <div className="flex items-center gap-2">
                        {failedTransformIndex === index ? (
                          <span className="text-destructive">{create.status === "error" ? create.error.message : ""}</span>
                        ) : null}
                        <button
                          type="button"
                          onClick={() => removeTransformDraft(index)}
                          className="text-muted-foreground hover:text-foreground"
                        >
                          Remove
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="mb-3 text-xs text-muted-foreground">No transforms added yet.</p>
              )}
              <div className="flex flex-wrap items-end gap-2 rounded-lg border border-dashed border-border p-3">
                <Field label="Verb">
                  <select
                    className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                    value={pendingVerb}
                    onChange={(e) => {
                      const verb = e.target.value as TransformDraft["verb"]
                      setPendingVerb(verb)
                      setPendingDraft(emptyDraftFor(verb))
                    }}
                  >
                    {TRANSFORM_VERBS.map((verb) => (
                      <option key={verb} value={verb}>
                        {verb}
                      </option>
                    ))}
                  </select>
                </Field>
                {pendingDraft.verb === "dedupe" ? (
                  <Field label="Key column">
                    <Input
                      value={pendingDraft.key}
                      onChange={(e) => setPendingDraft({ verb: "dedupe", key: e.target.value })}
                      placeholder="order_id"
                    />
                  </Field>
                ) : null}
                {pendingDraft.verb === "filter" ? (
                  <>
                    <Field label="Column">
                      <Input
                        value={pendingDraft.column}
                        onChange={(e) =>
                          setPendingDraft({ ...pendingDraft, column: e.target.value })
                        }
                        placeholder="status"
                      />
                    </Field>
                    <Field label="Operator">
                      <select
                        className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                        value={pendingDraft.operator}
                        onChange={(e) =>
                          setPendingDraft({
                            ...pendingDraft,
                            operator: e.target.value as (typeof FILTER_OPERATORS)[number],
                          })
                        }
                      >
                        {FILTER_OPERATORS.map((op) => (
                          <option key={op} value={op}>
                            {op}
                          </option>
                        ))}
                      </select>
                    </Field>
                    <Field label="Value">
                      <Input
                        value={pendingDraft.value}
                        onChange={(e) => setPendingDraft({ ...pendingDraft, value: e.target.value })}
                        placeholder="active"
                      />
                    </Field>
                  </>
                ) : null}
                {pendingDraft.verb === "rename" ? (
                  <>
                    <Field label="From column">
                      <Input
                        value={pendingDraft.from}
                        onChange={(e) => setPendingDraft({ ...pendingDraft, from: e.target.value })}
                        placeholder="old_col"
                      />
                    </Field>
                    <Field label="To column">
                      <Input
                        value={pendingDraft.to}
                        onChange={(e) => setPendingDraft({ ...pendingDraft, to: e.target.value })}
                        placeholder="new_col"
                      />
                    </Field>
                  </>
                ) : null}
                {pendingDraft.verb === "cast" ? (
                  <>
                    <Field label="Column">
                      <Input
                        value={pendingDraft.column}
                        onChange={(e) =>
                          setPendingDraft({ ...pendingDraft, column: e.target.value })
                        }
                        placeholder="amount"
                      />
                    </Field>
                    <Field label="Type">
                      <select
                        className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                        value={pendingDraft.type}
                        onChange={(e) =>
                          setPendingDraft({
                            ...pendingDraft,
                            type: e.target.value as (typeof CAST_TYPES)[number],
                          })
                        }
                      >
                        {CAST_TYPES.map((t) => (
                          <option key={t} value={t}>
                            {t}
                          </option>
                        ))}
                      </select>
                    </Field>
                  </>
                ) : null}
                {pendingDraft.verb === "select" ? (
                  <Field label="Columns (comma-separated)">
                    <Input
                      value={pendingDraft.columns}
                      onChange={(e) => setPendingDraft({ verb: "select", columns: e.target.value })}
                      placeholder="id,name,amount"
                    />
                  </Field>
                ) : null}
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!draftIsComplete(pendingDraft)}
                  onClick={addTransformDraft}
                >
                  Add transform
                </Button>
              </div>
            </div>
            <div className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
              <div>
                <p className="text-sm font-medium">FBIC enrichment</p>
                <p className="text-xs text-muted-foreground">
                  Enable feature-based incremental compute.
                </p>
              </div>
              <Switch checked={fbicEnabled} onCheckedChange={setFbicEnabled} />
            </div>
          </div>
        ) : null}

        {step === 2 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Target zone">
              <Input value={targetZone} onChange={(e) => setTargetZone(e.target.value)} />
            </Field>
            <Field label="Target table">
              <Input
                value={targetTable}
                onChange={(e) => setTargetTable(e.target.value)}
                placeholder="orders_hourly"
              />
            </Field>
          </div>
        ) : null}

        {step === 3 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Schedule" className="sm:col-span-2">
              <Input
                value={schedule}
                onChange={(e) => setSchedule(e.target.value)}
                placeholder="Every hour"
              />
            </Field>
          </div>
        ) : null}

        {step === 4 ? (
          <FormReviewSummary
            sections={[
              {
                title: "Source",
                items: [
                  { label: "Name", value: name },
                  { label: "Kind", value: kind },
                  { label: "Source", value: `${sourceZone}.${sourceTable}` },
                  { label: "Incremental column", value: incrementalColumn },
                  { label: "Connector", value: connectorId || "None" },
                ],
              },
              {
                title: "Transform",
                items: [
                  {
                    label: "Transforms",
                    value: transformDrafts.length
                      ? transformDrafts.map(renderTransformDraft).join(", ")
                      : "—",
                  },
                  { label: "FBIC", value: fbicEnabled ? "Enabled" : "Off" },
                ],
              },
              {
                title: "Target & schedule",
                items: [
                  { label: "Target", value: `${targetZone}.${targetTable}` },
                  { label: "Schedule", value: schedule },
                ],
              },
            ]}
          />
        ) : null}

        {create.status === "error" ? (
          <p className="text-sm text-destructive">{create.error.message}</p>
        ) : null}
      </FormStepLayout>
    </div>
  )
}

function Field({
  label,
  children,
  className,
}: {
  label: string
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label>{label}</Label>
      {children}
    </div>
  )
}
