"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { useServiceAction } from "@/hooks/use-service"
import { pipelineService } from "@/services"
import type { Pipeline } from "@/services/contracts/pipelines"

const PHASES = [
  "Understanding instruction…",
  "Discovering source schema…",
  "Designing transforms…",
  "Validating pipeline draft…",
]

export function AgenticBuilderDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (pipeline: Pipeline) => void
}) {
  const [model, setModel] = React.useState("rantai-agent-pro")
  const [instruction, setInstruction] = React.useState("")
  const [database, setDatabase] = React.useState("core.sales")
  const [fileName, setFileName] = React.useState("")
  const [error, setError] = React.useState<string | null>(null)
  const [phase, setPhase] = React.useState(0)
  const action = useServiceAction((signal, input: Parameters<typeof pipelineService.generatePipelineFromPrompt>[0]) =>
    pipelineService.generatePipelineFromPrompt(input, signal)
  )

  React.useEffect(() => {
    if (!open) {
      setModel("rantai-agent-pro")
      setInstruction("")
      setDatabase("core.sales")
      setFileName("")
      setError(null)
      setPhase(0)
      action.reset()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  React.useEffect(() => {
    if (action.status !== "pending") return
    setPhase(0)
    const timer = setInterval(() => {
      setPhase((p) => (p + 1) % PHASES.length)
    }, 400)
    return () => clearInterval(timer)
  }, [action.status])

  async function handleGenerate() {
    if (!instruction.trim()) {
      setError("Instruction is required.")
      return
    }
    setError(null)
    const result = await action.run({
      model,
      instruction: instruction.trim(),
      database,
      fileName: fileName || undefined,
    })
    if (result) {
      onCreated(result)
      onOpenChange(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Agentic Builder</DialogTitle>
          <DialogDescription>
            Describe the pipeline in natural language. A draft is generated from mock agent phases.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-2">
            <Label htmlFor="ab-model">Model</Label>
            <Input id="ab-model" value={model} onChange={(e) => setModel(e.target.value)} />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ab-instruction">Instruction</Label>
            <Textarea
              id="ab-instruction"
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder="Ingest orders events hourly into a rollup table…"
              rows={4}
            />
            {error ? <p className="text-xs text-destructive">{error}</p> : null}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ab-db">Source database</Label>
            <Input id="ab-db" value={database} onChange={(e) => setDatabase(e.target.value)} />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ab-file">Optional file</Label>
            <Input
              id="ab-file"
              type="file"
              onChange={(e) => setFileName(e.target.files?.[0]?.name ?? "")}
            />
            {fileName ? (
              <p className="text-xs text-muted-foreground">Attached: {fileName}</p>
            ) : null}
          </div>
          {action.status === "pending" ? (
            <p className="text-sm text-muted-foreground">{PHASES[phase]}</p>
          ) : null}
          {action.status === "error" ? (
            <p className="text-xs text-destructive">{action.error.message}</p>
          ) : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={action.status === "pending"}
            onClick={handleGenerate}
          >
            {action.status === "pending" ? "Generating…" : "Generate"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
