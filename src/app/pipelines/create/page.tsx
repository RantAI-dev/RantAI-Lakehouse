"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { ArrowLeft, ArrowRight, Check, PlusCircle, Square } from "lucide-react"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Card, CardContent } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

const STEPS = [
  { id: 1, key: "source", label: "Source" },
  { id: 2, key: "transform", label: "Transform" },
  { id: 3, key: "target", label: "Target" },
  { id: 4, key: "review", label: "Review" },
] as const

const ZONE_OPTIONS = ["Default", "Raw", "Bronze", "Silver", "Gold"] as const
const TABLE_OPTIONS = ["Default", "Customer_revenue", "Sales_analytics", "Orders_fact"] as const
const COLUMN_OPTIONS = ["Default", "updated_at", "created_at", "id"] as const

/** Figma Transform tab — labels match design (incl. "Agregate" spelling). */
const TRANSFORM_CHIPS = [
  "Cast Types",
  "Rename Columns",
  "Filter Rows",
  "Deduplicate",
  "Join Tables",
  "Agregate",
  "Parse JSON",
  "Add Timestamp",
] as const

type StepIndex = 0 | 1 | 2 | 3

/**
 * Multi-step "Create pipeline" wizard at `/pipelines/create`.
 *
 * Steps are tracked by the numeric `step` index against `STEPS`:
 * 0 Source → 1 Transform → 2 Target → 3 Review.
 * Each step has its own `*StepValid` memo that gates the Next button via
 * `canGoNext`. The Stepper rail on the left lets the user jump back to any
 * already-visited step (forward navigation is blocked until validation passes).
 *
 * The final "Create pipeline" button just routes back to `/pipelines`; no
 * persistence is performed in this preview build.
 */
export default function CreatePipelinePage() {
  const router = useRouter()
  const [step, setStep] = useState<StepIndex>(0)

  const [pipelineName, setPipelineName] = useState("")
  const [description, setDescription] = useState("")
  const [sourceZone, setSourceZone] = useState<string>("")
  const [sourceTable, setSourceTable] = useState<string>("")
  const [incrementalColumn, setIncrementalColumn] = useState<string>("")

  const [selectedTransforms, setSelectedTransforms] = useState<string[]>([])
  const [fbicEnabled, setFbicEnabled] = useState(false)

  const [targetZone, setTargetZone] = useState<string>("")
  const [targetTable, setTargetTable] = useState<string>("")

  const sourceStepValid = useMemo(() => {
    return (
      pipelineName.trim().length > 0 &&
      Boolean(sourceZone) &&
      Boolean(sourceTable) &&
      Boolean(incrementalColumn)
    )
  }, [pipelineName, sourceZone, sourceTable, incrementalColumn])

  const transformStepValid = useMemo(() => {
    return selectedTransforms.length > 0 || fbicEnabled
  }, [selectedTransforms, fbicEnabled])

  const targetStepValid = useMemo(() => {
    return Boolean(targetZone) && Boolean(targetTable)
  }, [targetZone, targetTable])

  const canGoNext =
    step === 0
      ? sourceStepValid
      : step === 1
        ? transformStepValid
        : step === 2
          ? targetStepValid
          : true

  /** Advances to the next wizard step when the current one passes validation. */
  const handleNext = () => {
    if (step < 3 && canGoNext) setStep((s) => (s + 1) as StepIndex)
  }

  /** Returns to the previous wizard step (no-op on the first step). */
  const handleBack = () => {
    if (step > 0) setStep((s) => (s - 1) as StepIndex)
  }

  /** Closes the wizard by navigating back to the pipelines list. */
  const handleFinish = () => {
    router.push("/pipelines")
  }

  return (
    <div className="flex flex-col gap-4 p-4">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <div className="flex flex-col gap-1">
          <Breadcrumb>
            <BreadcrumbList className="text-sm text-muted-foreground">
              <BreadcrumbItem>
                <BreadcrumbLink href="/pipelines" render={<Link href="/pipelines" />}>
                  Pipelines
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbPage className="font-normal text-primary">
                  Create Pipeline
                </BreadcrumbPage>
              </BreadcrumbItem>
            </BreadcrumbList>
          </Breadcrumb>
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Create Pipeline
          </h1>
        </div>
      </header>

      <div className="flex flex-col gap-4 lg:flex-row lg:items-start">
        {/* Stepper — Figma: 176px rail, border bg, rounded-md */}
        <nav
          className="flex w-full shrink-0 flex-col gap-2 rounded-md border border-border bg-border p-4 lg:w-[176px]"
          aria-label="Pipeline steps"
        >
          {STEPS.map((s, i) => {
            const active = step === i
            const done = step > i
            return (
              <button
                key={s.key}
                type="button"
                onClick={() => {
                  if (i <= step) setStep(i as StepIndex)
                }}
                className={cn(
                  "flex h-7 w-full items-center gap-1 rounded px-3 py-1 text-left transition-colors",
                  active &&
                    "bg-background shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
                  !active && "bg-transparent"
                )}
              >
                <span
                  className={cn(
                    "flex min-w-[22px] items-center justify-center rounded border px-1.5 py-0.5 text-xs leading-none tracking-[-0.072px]",
                    active
                      ? "border-primary text-primary"
                      : "border-[#64748b] text-[#64748b]"
                  )}
                >
                  {done ? <Check className="size-3" strokeWidth={2.5} /> : s.id}
                </span>
                <span
                  className={cn(
                    "text-xs font-medium leading-4 tracking-[-0.072px]",
                    active ? "text-primary" : "text-[#64748b]"
                  )}
                >
                  {s.label}
                </span>
              </button>
            )
          })}
        </nav>

        <Card
          className={cn(
            "min-h-[396px] flex-1 gap-0 rounded-lg border border-border bg-card py-0 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] ring-0"
          )}
        >
          <CardContent className="flex flex-col gap-6 p-6">
            {step === 0 && (
              <SourceStep
                pipelineName={pipelineName}
                setPipelineName={setPipelineName}
                description={description}
                setDescription={setDescription}
                sourceZone={sourceZone}
                setSourceZone={setSourceZone}
                sourceTable={sourceTable}
                setSourceTable={setSourceTable}
                incrementalColumn={incrementalColumn}
                setIncrementalColumn={setIncrementalColumn}
              />
            )}
            {step === 1 && (
              <TransformStep
                selectedTransforms={selectedTransforms}
                onToggleTransform={(label) => {
                  setSelectedTransforms((prev) =>
                    prev.includes(label)
                      ? prev.filter((x) => x !== label)
                      : [...prev, label]
                  )
                }}
                fbicEnabled={fbicEnabled}
                onFbicChange={setFbicEnabled}
              />
            )}
            {step === 2 && (
              <TargetStep
                targetZone={targetZone}
                setTargetZone={setTargetZone}
                targetTable={targetTable}
                setTargetTable={setTargetTable}
              />
            )}
            {step === 3 && (
              <ReviewStep
                pipelineName={pipelineName}
                description={description}
                sourceZone={sourceZone}
                sourceTable={sourceTable}
                incrementalColumn={incrementalColumn}
                selectedTransforms={selectedTransforms}
                fbicEnabled={fbicEnabled}
                targetZone={targetZone}
                targetTable={targetTable}
              />
            )}

            <div className="flex flex-wrap items-center justify-end gap-6">
              {step > 0 && (
                <Button
                  type="button"
                  variant="outline"
                  size="lg"
                  className="h-10 gap-2 rounded-md border-border bg-background px-4 text-sm font-medium text-primary hover:bg-muted"
                  onClick={handleBack}
                >
                  <ArrowLeft className="size-4" />
                  Previous
                </Button>
              )}
              {step < 3 ? (
                <Button
                  type="button"
                  size="lg"
                  disabled={!canGoNext}
                  className="h-10 gap-2 rounded-md bg-primary px-4 text-sm font-medium tracking-[-0.084px] text-primary-foreground hover:bg-primary/90"
                  onClick={handleNext}
                >
                  Next
                  <ArrowRight className="size-4" />
                </Button>
              ) : (
                <Button
                  type="button"
                  size="lg"
                  className="h-10 gap-2 rounded-md bg-primary px-4 text-sm font-medium tracking-[-0.084px] text-primary-foreground hover:bg-primary/90"
                  onClick={handleFinish}
                >
                  Create pipeline
                  <Check className="size-4" />
                </Button>
              )}
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

/** `<Label>` wrapper that appends a red `*` when `required` is true. */
function FieldLabel({
  children,
  required,
  ...props
}: React.ComponentProps<typeof Label> & { required?: boolean }) {
  return (
    <Label
      className="gap-0 text-sm font-medium leading-5 tracking-[-0.084px] text-primary"
      {...props}
    >
      {children}
      {required && (
        <span className="ml-0.5 font-medium text-[#e60000]" aria-hidden>
          *
        </span>
      )}
    </Label>
  )
}

/**
 * Step 1 — Source. Captures the pipeline name, description, source zone/table,
 * and the incremental-load column used by FBIC. All fields are controlled by
 * the parent so wizard-level validation can run.
 */
function SourceStep({
  pipelineName,
  setPipelineName,
  description,
  setDescription,
  sourceZone,
  setSourceZone,
  sourceTable,
  setSourceTable,
  incrementalColumn,
  setIncrementalColumn,
}: {
  pipelineName: string
  setPipelineName: (v: string) => void
  description: string
  setDescription: (v: string) => void
  sourceZone: string
  setSourceZone: (v: string) => void
  sourceTable: string
  setSourceTable: (v: string) => void
  incrementalColumn: string
  setIncrementalColumn: (v: string) => void
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <FieldLabel htmlFor="pipeline-name">Pipeline Name</FieldLabel>
        <Input
          id="pipeline-name"
          placeholder="e.g. customer_revenue_daily"
          value={pipelineName}
          onChange={(e) => setPipelineName(e.target.value)}
          className="h-10 rounded-md border-border px-3 text-sm placeholder:text-muted-foreground"
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <FieldLabel htmlFor="pipeline-desc">Description</FieldLabel>
        <Textarea
          id="pipeline-desc"
          placeholder="e.g. Incremental load from raw landing to bronze with validation"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={4}
          className="min-h-[108px] resize-y rounded-md border-border px-3 py-2 text-sm placeholder:text-muted-foreground"
        />
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          <FieldLabel required>Source Zone</FieldLabel>
          <Select
            value={sourceZone || undefined}
            onValueChange={(v) => setSourceZone(v ?? "")}
          >
            <SelectTrigger
              className="h-10 w-full rounded-md border-border pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary"
            >
              <SelectValue placeholder="Default" />
            </SelectTrigger>
            <SelectContent>
              {ZONE_OPTIONS.map((z) => (
                <SelectItem key={z} value={z}>
                  {z}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <FieldLabel required>Source Table</FieldLabel>
          <Select
            value={sourceTable || undefined}
            onValueChange={(v) => setSourceTable(v ?? "")}
          >
            <SelectTrigger
              className="h-10 w-full rounded-md border-border pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary"
            >
              <SelectValue placeholder="Default" />
            </SelectTrigger>
            <SelectContent>
              {TABLE_OPTIONS.map((t) => (
                <SelectItem key={t} value={t}>
                  {t}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex min-w-0 flex-col gap-1.5 sm:col-span-2 xl:col-span-1">
          <FieldLabel required>Incremental Column (for FBIC)</FieldLabel>
          <Select
            value={incrementalColumn || undefined}
            onValueChange={(v) => setIncrementalColumn(v ?? "")}
          >
            <SelectTrigger
              className="h-10 w-full rounded-md border-border pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary"
            >
              <SelectValue placeholder="Default" />
            </SelectTrigger>
            <SelectContent>
              {COLUMN_OPTIONS.map((c) => (
                <SelectItem key={c} value={c}>
                  {c}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
    </div>
  )
}

/**
 * Step 2 — Transform. Lets the user toggle one or more transform "chips"
 * (cast, dedupe, join, …) and switch on FBIC (Fingerprint-Based Incremental
 * Computation). Validation requires at least one transform selected OR FBIC on.
 */
function TransformStep({
  selectedTransforms,
  onToggleTransform,
  fbicEnabled,
  onFbicChange,
}: {
  selectedTransforms: string[]
  onToggleTransform: (label: string) => void
  fbicEnabled: boolean
  onFbicChange: (v: boolean) => void
}) {
  return (
    <div className="flex w-full flex-col gap-6">
      <div className="flex flex-col gap-1.5">
        {/* Figma node 12:1194 uses label "Pipeline Name" on this frame; chips are transform actions. */}
        <FieldLabel>Pipeline Name</FieldLabel>
        <div className="flex flex-wrap gap-1.5">
          {TRANSFORM_CHIPS.map((label) => {
            const on = selectedTransforms.includes(label)
            return (
              <Button
                key={label}
                type="button"
                variant="outline"
                onClick={() => onToggleTransform(label)}
                aria-pressed={on}
                className={cn(
                  "h-9 gap-2 rounded-md border-border px-4 text-sm font-medium tracking-[-0.084px] text-primary shadow-none",
                  "hover:bg-muted hover:text-primary",
                  on && "border-primary bg-background ring-1 ring-primary/30"
                )}
              >
                <PlusCircle className="size-4 shrink-0" strokeWidth={2} />
                {label}
              </Button>
            )
          })}
        </div>
      </div>

      <div className="flex flex-col gap-2 rounded-md bg-accent p-4">
        <div className="flex items-start gap-2 sm:items-center sm:justify-between">
          <div className="flex min-w-0 flex-1 items-start gap-2">
            <Square className="mt-0.5 size-5 shrink-0 text-primary sm:mt-0" />
            <p className="min-w-0 text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
              Enable FBIC Fingerprint-Based Incremental Computation
            </p>
          </div>
          <Switch
            checked={fbicEnabled}
            onCheckedChange={onFbicChange}
            className="mt-1 shrink-0 sm:mt-0"
            aria-label="Enable FBIC fingerprint-based incremental computation"
          />
        </div>
        <p className="text-sm leading-5 text-[#5d5d5d]">
          Only process new and modified records since the last run
        </p>
      </div>
    </div>
  )
}

/** Step 3 — Target. Two required selects: target zone and target table. */
function TargetStep({
  targetZone,
  setTargetZone,
  targetTable,
  setTargetTable,
}: {
  targetZone: string
  setTargetZone: (v: string) => void
  targetTable: string
  setTargetTable: (v: string) => void
}) {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="flex min-w-0 flex-col gap-1.5">
        <FieldLabel required>Target zone</FieldLabel>
        <Select
          value={targetZone || undefined}
          onValueChange={(v) => setTargetZone(v ?? "")}
        >
          <SelectTrigger className="h-10 w-full rounded-md border-border pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary">
            <SelectValue placeholder="Select zone" />
          </SelectTrigger>
          <SelectContent>
            {ZONE_OPTIONS.filter((z) => z !== "Default").map((z) => (
              <SelectItem key={z} value={z}>
                {z}
              </SelectItem>
            ))}
            <SelectItem value="Semantic">Semantic</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <FieldLabel required>Target table</FieldLabel>
        <Select
          value={targetTable || undefined}
          onValueChange={(v) => setTargetTable(v ?? "")}
        >
          <SelectTrigger className="h-10 w-full rounded-md border-border pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary">
            <SelectValue placeholder="Select table" />
          </SelectTrigger>
          <SelectContent>
            {TABLE_OPTIONS.filter((t) => t !== "Default").map((t) => (
              <SelectItem key={t} value={t}>
                {t}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </div>
  )
}

/**
 * Step 4 — Review. Read-only summary of every field collected in the previous
 * steps. The user confirms here, then `handleFinish` closes the wizard.
 */
function ReviewStep({
  pipelineName,
  description,
  sourceZone,
  sourceTable,
  incrementalColumn,
  selectedTransforms,
  fbicEnabled,
  targetZone,
  targetTable,
}: {
  pipelineName: string
  description: string
  sourceZone: string
  sourceTable: string
  incrementalColumn: string
  selectedTransforms: string[]
  fbicEnabled: boolean
  targetZone: string
  targetTable: string
}) {
  const transformSummary =
    selectedTransforms.length > 0 ? selectedTransforms.join(", ") : "—"
  const rows = [
    ["Pipeline name", pipelineName || "—"],
    ["Description", description || "—"],
    ["Source zone", sourceZone || "—"],
    ["Source table", sourceTable || "—"],
    ["Incremental column", incrementalColumn || "—"],
    ["Transformations", transformSummary],
    ["FBIC enabled", fbicEnabled ? "Yes" : "No"],
    ["Target zone", targetZone || "—"],
    ["Target table", targetTable || "—"],
  ] as const
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
        Review and confirm
      </p>
      <dl className="grid gap-2 rounded-lg border border-border bg-muted/30 p-4 text-sm">
        {rows.map(([k, v]) => (
          <div key={k} className="grid gap-0.5 sm:grid-cols-[160px_1fr] sm:gap-4">
            <dt className="font-medium text-muted-foreground">{k}</dt>
            <dd className="text-foreground">{v}</dd>
          </div>
        ))}
      </dl>
    </div>
  )
}
