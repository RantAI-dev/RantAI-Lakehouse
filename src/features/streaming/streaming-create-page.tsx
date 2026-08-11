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
import { Textarea } from "@/components/ui/textarea"
import { useServiceAction } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import { streamingService } from "@/services"

const STEPS: FormStep[] = [
  { id: "basics", label: "Basics", description: "Job name" },
  { id: "sources", label: "Sources", description: "Inputs and sinks" },
  { id: "definition", label: "Definition", description: "SQL" },
  { id: "triggers", label: "Triggers", description: "Watermark & conditions" },
  { id: "review", label: "Review", description: "Confirm" },
]

export function StreamingCreatePage() {
  const router = useRouter()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [sources, setSources] = React.useState("kafka.events")
  const [sinks, setSinks] = React.useState("hot.mv_target")
  const [definitionSql, setDefinitionSql] = React.useState(
    "CREATE MATERIALIZED VIEW example AS\nSELECT window_start, count(*) FROM kafka.events\nGROUP BY tumble(event_time, INTERVAL '1' MINUTE);"
  )
  const [watermarkIntervalSec, setWatermarkIntervalSec] = React.useState("5")
  const [triggerCondition, setTriggerCondition] = React.useState("lag_seconds > 30")
  const create = useServiceAction((signal, input: Parameters<typeof streamingService.createStreamingJob>[0]) =>
    streamingService.createStreamingJob(input, signal)
  )

  const canProceed =
    (step === 0 && Boolean(name.trim())) ||
    (step === 1 && Boolean(sources.trim() && sinks.trim())) ||
    (step === 2 && Boolean(definitionSql.trim())) ||
    (step === 3 && Boolean(watermarkIntervalSec && triggerCondition.trim())) ||
    step === 4

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      sources: sources.split(",").map((s) => s.trim()).filter(Boolean),
      sinks: sinks.split(",").map((s) => s.trim()).filter(Boolean),
      definitionSql: definitionSql.trim(),
      watermarkIntervalSec: Number(watermarkIntervalSec) || 5,
      triggerCondition: triggerCondition.trim(),
    })
    if (result) router.push("/streaming")
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Create Streaming Job"
        description="Define sources, SQL, watermarks, and trigger conditions."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/streaming" />}>
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
        submitLabel="Create job"
        submitting={create.status === "pending"}
      >
        {step === 0 ? (
          <Field label="Job name">
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="rt.example_mv" />
          </Field>
        ) : null}
        {step === 1 ? (
          <div className="grid gap-3">
            <Field label="Sources (comma-separated)">
              <Input value={sources} onChange={(e) => setSources(e.target.value)} />
            </Field>
            <Field label="Sinks (comma-separated)">
              <Input value={sinks} onChange={(e) => setSinks(e.target.value)} />
            </Field>
          </div>
        ) : null}
        {step === 2 ? (
          <Field label="Definition SQL">
            <Textarea
              value={definitionSql}
              onChange={(e) => setDefinitionSql(e.target.value)}
              rows={8}
              className="font-mono text-xs"
            />
          </Field>
        ) : null}
        {step === 3 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Watermark interval (sec)">
              <Input
                type="number"
                value={watermarkIntervalSec}
                onChange={(e) => setWatermarkIntervalSec(e.target.value)}
              />
            </Field>
            <Field label="Trigger condition">
              <Input
                value={triggerCondition}
                onChange={(e) => setTriggerCondition(e.target.value)}
              />
            </Field>
          </div>
        ) : null}
        {step === 4 ? (
          <FormReviewSummary
            sections={[
              {
                title: "Job",
                items: [
                  { label: "Name", value: name },
                  { label: "Sources", value: sources },
                  { label: "Sinks", value: sinks },
                  { label: "Watermark", value: `${watermarkIntervalSec}s` },
                  { label: "Trigger", value: triggerCondition },
                ],
              },
              {
                title: "Definition",
                items: [{ label: "SQL", value: <pre className="whitespace-pre-wrap font-mono text-xs">{definitionSql}</pre> }],
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
