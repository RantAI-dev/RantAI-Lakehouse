"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import type { PipelineKind } from "@/services/contracts/pipelines"

const STEPS: FormStep[] = [
  { id: "source", label: "Source", description: "Name and source table" },
  { id: "transform", label: "Transform", description: "Chips or FBIC" },
  { id: "target", label: "Target", description: "Destination table" },
  { id: "schedule", label: "Schedule", description: "Trigger and ownership" },
  { id: "review", label: "Review", description: "Confirm and create" },
]

const TRANSFORM_CHIPS = [
  "Select",
  "Filter",
  "Rename",
  "Cast",
  "Clean",
  "Join",
  "Aggregate",
  "Deduplicate",
]

const KIND_OPTIONS: PipelineKind[] = ["batch", "incremental", "document", "vector"]

export function PipelineCreatePage() {
  const router = useRouter()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [kind, setKind] = React.useState<PipelineKind>("incremental")
  const [sourceZone, setSourceZone] = React.useState("bronze")
  const [sourceTable, setSourceTable] = React.useState("")
  const [incrementalColumn, setIncrementalColumn] = React.useState("updated_at")
  const [transforms, setTransforms] = React.useState<string[]>([])
  const [fbicEnabled, setFbicEnabled] = React.useState(false)
  const [targetZone, setTargetZone] = React.useState("silver")
  const [targetTable, setTargetTable] = React.useState("")
  const [schedule, setSchedule] = React.useState("Every hour")
  const create = useServiceAction(
    withNotify(
      { success: "Pipeline created", error: "Failed to create pipeline" },
      (signal, input: Parameters<typeof pipelineService.createPipeline>[0]) =>
        pipelineService.createPipeline(input, signal)
    )
  )

  const canProceed = React.useMemo(() => {
    if (step === 0) {
      return Boolean(name.trim() && sourceZone.trim() && sourceTable.trim() && incrementalColumn.trim())
    }
    if (step === 1) return transforms.length > 0 || fbicEnabled
    if (step === 2) return Boolean(targetZone.trim() && targetTable.trim())
    if (step === 3) return Boolean(schedule.trim())
    return true
  }, [
    step,
    name,
    sourceZone,
    sourceTable,
    incrementalColumn,
    transforms,
    fbicEnabled,
    targetZone,
    targetTable,
    schedule,
  ])

  function toggleTransform(chip: string) {
    setTransforms((prev) =>
      prev.includes(chip) ? prev.filter((c) => c !== chip) : [...prev, chip]
    )
  }

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      kind,
      sourceZone: sourceZone.trim(),
      sourceTable: sourceTable.trim(),
      incrementalColumn: incrementalColumn.trim(),
      transforms,
      fbicEnabled,
      targetZone: targetZone.trim(),
      targetTable: targetTable.trim(),
      schedule: schedule.trim(),
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
            <Field label="Kind">
              <select
                className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                value={kind}
                onChange={(e) => setKind(e.target.value as PipelineKind)}
              >
                {KIND_OPTIONS.map((k) => (
                  <option key={k} value={k}>
                    {k}
                  </option>
                ))}
              </select>
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
              <div className="flex flex-wrap gap-2">
                {TRANSFORM_CHIPS.map((chip) => {
                  const active = transforms.includes(chip)
                  return (
                    <button
                      key={chip}
                      type="button"
                      onClick={() => toggleTransform(chip)}
                      className={cn(
                        "rounded-full border px-3 py-1 text-xs font-medium transition-colors",
                        active
                          ? "border-primary bg-primary/15 text-primary"
                          : "border-border text-muted-foreground hover:bg-muted"
                      )}
                    >
                      {chip}
                    </button>
                  )
                })}
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
                ],
              },
              {
                title: "Transform",
                items: [
                  {
                    label: "Transforms",
                    value: transforms.length ? transforms.join(", ") : "—",
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
