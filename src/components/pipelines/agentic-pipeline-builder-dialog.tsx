"use client"

import { useCallback, useRef, useState } from "react"
import { Loader2, Upload } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { buildAgentPipelineSuccessOutput } from "@/lib/pipeline-agent-output"
import type { PipelineDetail, PipelineItem } from "@/types/pipeline"

const AI_MODELS = [
  { value: "gpt-4.1", label: "GPT-4.1" },
  { value: "gpt-5", label: "GPT-5" },
  { value: "claude-sonnet", label: "Claude (Sonnet)" },
  { value: "gemini-2", label: "Gemini 2" },
  { value: "rantai-internal", label: "Rantai Internal Model" },
] as const

const DB_SOURCES = [
  { value: "postgresql", label: "PostgreSQL" },
  { value: "mysql", label: "MySQL" },
  { value: "bigquery", label: "BigQuery" },
  { value: "snowflake", label: "Snowflake" },
  { value: "internal-dw", label: "Internal data warehouse" },
] as const

const GENERATION_PHASES = [
  "Analyzing data source…",
  "Generating pipeline steps…",
  "Validating transformation logic…",
  "Creating pipeline draft…",
] as const

const NL_PLACEHOLDER =
  "Describe the pipeline you want to create, including source data, transformation rules, validation, and destination."

/**
 * Derives a short safe pipeline name from the user's natural-language instruction.
 *
 * Takes the first 40 chars, replaces whitespace with `_`, strips non-alphanumeric
 * characters, and falls back to a timestamp-based name when nothing usable remains.
 */
function slugFromInstruction(text: string): string {
  const base = text
    .slice(0, 40)
    .trim()
    .replace(/\s+/g, "_")
    .replace(/[^a-zA-Z0-9_]/g, "")
  return base.length > 0 ? base : `Pipeline_${Date.now().toString(36)}`
}

/**
 * Builds a mock `PipelineDetail` (Extract / Transform / Validate / Load steps and
 * source/destination/transform descriptions) from the user's dialog inputs.
 *
 * In a production integration this would be replaced by a real call to the
 * orchestration / LLM backend; here it just stitches the inputs into descriptive
 * strings so the generated draft pipeline looks plausible in the UI.
 */
function buildDetailFromInputs(args: {
  instruction: string
  modelLabel: string
  sourceLabel: string
  fileNames: string[]
}): PipelineDetail {
  const fileHint =
    args.fileNames.length > 0
      ? `Reference files: ${args.fileNames.join(", ")}.`
      : "No uploaded reference files."
  return {
    sourceConfig: `${args.sourceLabel}. ${fileHint} Model context: ${args.modelLabel}.`,
    transformLogic:
      args.instruction.trim().length > 0
        ? args.instruction.trim()
        : "Transformations derived from your selections and uploaded context.",
    destination: "Draft target: promoted table in the Silver or Gold zone (configurable in the pipeline editor).",
    steps: [
      {
        title: "Extract",
        description: `Connect to ${args.sourceLabel} and ingest the described datasets.`,
      },
      {
        title: "Transform",
        description:
          "Apply cleansing, deduplication, and business rules implied by your instructions.",
      },
      {
        title: "Validate",
        description: "Schema checks, null constraints, and quality gates before load.",
      },
      {
        title: "Load",
        description: "Write curated rows to the destination warehouse table.",
      },
    ],
  }
}

export type AgenticPipelineBuilderDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPipelineGenerated: (pipeline: PipelineItem) => void
}

/**
 * Modal dialog used on the Pipelines list page to "AI-generate" a draft pipeline.
 *
 * Captures: AI model, natural-language instructions, optional reference files,
 * and a database source. On submit it simulates a multi-phase generation flow
 * (`GENERATION_PHASES`) using `setTimeout`, then calls `onPipelineGenerated`
 * with a synthetic `PipelineItem` and closes itself.
 *
 * No real network calls happen here — this is a UI preview.
 */
export function AgenticPipelineBuilderDialog({
  open,
  onOpenChange,
  onPipelineGenerated,
}: AgenticPipelineBuilderDialogProps) {
  const [model, setModel] = useState<string>(AI_MODELS[0]!.value)
  const [instruction, setInstruction] = useState("")
  const [dbSource, setDbSource] = useState<string>(DB_SOURCES[0]!.value)
  const [fileNames, setFileNames] = useState<string[]>([])
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [generating, setGenerating] = useState(false)
  const [phaseIndex, setPhaseIndex] = useState(0)
  const [error, setError] = useState<string | null>(null)

  /** Returns every form field to its initial value (used when the dialog closes). */
  const resetForm = useCallback(() => {
    setModel(AI_MODELS[0]!.value)
    setInstruction("")
    setDbSource(DB_SOURCES[0]!.value)
    setFileNames([])
    setGenerating(false)
    setPhaseIndex(0)
    setError(null)
    if (fileInputRef.current) fileInputRef.current.value = ""
  }, [])

  /** Wraps `onOpenChange` so the form is cleared whenever the dialog closes. */
  const handleDialogOpenChange = useCallback(
    (next: boolean) => {
      onOpenChange(next)
      if (!next) resetForm()
    },
    [onOpenChange, resetForm]
  )

  const modelLabel =
    AI_MODELS.find((m) => m.value === model)?.label ?? model
  const sourceLabel =
    DB_SOURCES.find((s) => s.value === dbSource)?.label ?? dbSource

  /** Captures only file names from the file picker; we don't read file bytes here. */
  const handleFiles = (e: React.ChangeEvent<HTMLInputElement>) => {
    const list = e.target.files
    if (!list?.length) return
    setFileNames(Array.from(list).map((f) => f.name))
  }

  /**
   * Validates the instruction and runs the mock multi-phase generation flow.
   *
   * Cycles `phaseIndex` through `GENERATION_PHASES` to drive the loading copy,
   * then synthesizes a `PipelineItem` (with a generated detail and a fake
   * "last successful run" payload) and emits it via `onPipelineGenerated`.
   */
  const handleGenerate = () => {
    if (!instruction.trim()) {
      setError("Please describe your pipeline in natural language.")
      return
    }
    setError(null)
    setGenerating(true)
    setPhaseIndex(0)

    let step = 0
    const tick = () => {
      setPhaseIndex(step)
      step += 1
      if (step < GENERATION_PHASES.length) {
        window.setTimeout(tick, 900)
      } else {
        window.setTimeout(() => {
          const id = `agent-${Date.now()}`
          const name = slugFromInstruction(instruction)
          const detail = buildDetailFromInputs({
            instruction,
            modelLabel,
            sourceLabel,
            fileNames,
          })
          const pipeline: PipelineItem = {
            id,
            name,
            description: `AI-generated draft using ${modelLabel}.`,
            flow: { from: "Bronze", to: "Silver" },
            status: "Success",
            lastRun: "Just now",
            records: "12.4k record",
            schedule: "On demand",
            successRate: "—",
            detail,
            lastSuccessResult: buildAgentPipelineSuccessOutput(name),
          }
          onPipelineGenerated(pipeline)
          setGenerating(false)
          handleDialogOpenChange(false)
        }, 600)
      }
    }
    window.setTimeout(tick, 400)
  }

  return (
    <Dialog open={open} onOpenChange={handleDialogOpenChange}>
      <DialogContent className="max-h-[min(90vh,760px)] max-w-xl sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Agentic Pipeline Builder</DialogTitle>
          <DialogDescription>
            Describe your pipeline in natural language, pick a model and data
            source, optionally attach files, then generate a draft you can open
            and refine.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {generating ? (
            <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
              <Loader2
                className="size-10 animate-spin text-primary"
                aria-hidden
              />
              <p className="text-sm font-medium text-primary">
                {GENERATION_PHASES[phaseIndex] ?? GENERATION_PHASES[0]}
              </p>
              <p className="max-w-sm text-xs text-muted-foreground">
                This is a preview flow. A production integration would call your
                orchestration and model APIs.
              </p>
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="agentic-model">AI model</Label>
                <Select value={model} onValueChange={(v) => v && setModel(v)}>
                  <SelectTrigger id="agentic-model" className="h-10">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {AI_MODELS.map((m) => (
                      <SelectItem key={m.value} value={m.value}>
                        {m.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="flex flex-col gap-1.5">
                <Label htmlFor="agentic-instruction">Pipeline instructions</Label>
                <Textarea
                  id="agentic-instruction"
                  value={instruction}
                  onChange={(e) => setInstruction(e.target.value)}
                  placeholder={NL_PLACEHOLDER}
                  rows={5}
                  className="min-h-[120px] resize-y text-sm"
                />
              </div>

              <div className="flex flex-col gap-1.5">
                <Label htmlFor="agentic-files">Reference files (optional)</Label>
                <p className="text-xs text-muted-foreground">
                  CSV, Excel, JSON, or SQL — used as context for the agent.
                </p>
                <input
                  ref={fileInputRef}
                  id="agentic-files"
                  type="file"
                  multiple
                  accept=".csv,.tsv,.xlsx,.xls,.json,.sql,text/csv,application/json"
                  className="sr-only"
                  onChange={handleFiles}
                />
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="gap-2"
                    onClick={() => fileInputRef.current?.click()}
                  >
                    <Upload className="size-4" />
                    Choose files
                  </Button>
                  {fileNames.length > 0 ? (
                    <span className="text-xs text-muted-foreground">
                      {fileNames.join(", ")}
                    </span>
                  ) : (
                    <span className="text-xs text-muted-foreground">
                      No files selected
                    </span>
                  )}
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <Label htmlFor="agentic-source">Database source</Label>
                <Select value={dbSource} onValueChange={(v) => v && setDbSource(v)}>
                  <SelectTrigger id="agentic-source" className="h-10">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DB_SOURCES.map((s) => (
                      <SelectItem key={s.value} value={s.value}>
                        {s.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {error ? (
                <p className="text-sm text-destructive" role="alert">
                  {error}
                </p>
              ) : null}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => handleDialogOpenChange(false)}
            disabled={generating}
          >
            Cancel
          </Button>
          <Button
            type="button"
            className="gap-2 bg-primary text-primary-foreground"
            onClick={handleGenerate}
            disabled={generating}
          >
            {generating ? (
              <>
                <Loader2 className="size-4 animate-spin" />
                Generating…
              </>
            ) : (
              "Generate Pipeline"
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
