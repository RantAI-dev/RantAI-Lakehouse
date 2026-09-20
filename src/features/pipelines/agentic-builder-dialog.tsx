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
import { Spinner } from "@/components/ui/spinner"
import { useServiceAction } from "@/hooks/use-service"
import { pipelineService } from "@/services"
import type { Pipeline } from "@/services/contracts/pipelines"

/**
 * Draft a pipeline from a sentence.
 *
 * What this used to show: a model field the server ignored, a file picker
 * that uploaded nothing and only remembered a filename, and four "agent
 * phases" rotating every 400ms while a single request was in flight. The
 * dialog's own description said the phases were mock. What it now shows is
 * the one request that actually happens, and what came back from it.
 */
export function AgenticBuilderDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (pipeline: Pipeline) => void
}) {
  const [instruction, setInstruction] = React.useState("")
  const [database, setDatabase] = React.useState("serving")
  const [error, setError] = React.useState<string | null>(null)
  const action = useServiceAction(
    (signal, input: Parameters<typeof pipelineService.generatePipelineFromPrompt>[0]) =>
      pipelineService.generatePipelineFromPrompt(input, signal)
  )

  React.useEffect(() => {
    if (!open) {
      setInstruction("")
      setDatabase("serving")
      setError(null)
      action.reset()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  async function handleGenerate() {
    if (!instruction.trim()) {
      setError("Instruction is required.")
      return
    }
    setError(null)
    const result = await action.run({
      instruction: instruction.trim(),
      database,
    })
    if (result) {
      onCreated(result)
      onOpenChange(false)
    }
  }

  const generating = action.status === "pending"

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Agentic Builder</DialogTitle>
          <DialogDescription>
            Describe what the pipeline should do. It is saved as a draft —
            review and edit it before anything runs.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-2">
            <Label htmlFor="ab-instruction">Instruction</Label>
            <Textarea
              id="ab-instruction"
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder="Roll up hourly order events into a daily table, keyed by region…"
              rows={4}
            />
            {error ? <p className="text-xs text-destructive">{error}</p> : null}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ab-db">Database</Label>
            <Input
              id="ab-db"
              value={database}
              onChange={(e) => setDatabase(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              Source and target tables are proposed inside this database.
            </p>
          </div>
          {generating ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner className="size-4" />
              Drafting the pipeline…
            </p>
          ) : null}
          {action.status === "error" ? (
            <p className="text-xs text-destructive">{action.error.message}</p>
          ) : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" disabled={generating} onClick={handleGenerate}>
            {generating ? "Drafting…" : "Draft pipeline"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
