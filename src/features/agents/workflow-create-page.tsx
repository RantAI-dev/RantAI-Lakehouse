"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { FlowCanvas } from "@/components/patterns/flow-canvas"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useServiceAction } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import { agentService } from "@/services"

const STEPS: FormStep[] = [
  { id: "trigger", label: "Trigger", description: "Name and event" },
  { id: "steps", label: "Steps", description: "Ordered actions" },
  { id: "approval", label: "Approval", description: "Human gate" },
  { id: "review", label: "Review", description: "Confirm" },
]

const STEP_KINDS = [
  "condition",
  "model",
  "data query",
  "retrieval",
  "tool action",
  "human approval",
  "notification",
  "output sink",
]

export function WorkflowCreatePage() {
  const router = useRouter()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [trigger, setTrigger] = React.useState("")
  const [stepKinds, setStepKinds] = React.useState<string[]>(["retrieval", "data query"])
  const [approvalRequired, setApprovalRequired] = React.useState(true)
  const create = useServiceAction((signal, input: Parameters<typeof agentService.createWorkflow>[0]) =>
    agentService.createWorkflow(input, signal)
  )

  const canProceed =
    (step === 0 && Boolean(name.trim() && trigger.trim())) ||
    (step === 1 && stepKinds.length > 0) ||
    step === 2 ||
    step === 3

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      trigger: trigger.trim(),
      stepKinds,
      approvalRequired,
    })
    if (result) router.push("/agents/workflows")
  }

  const previewNodes = [
    { id: "t", label: "Trigger", kind: "trigger", status: "ready" as const },
    ...stepKinds.map((kind, i) => ({
      id: `s${i}`,
      label: kind,
      kind: "step",
      status: "draft" as const,
    })),
    {
      id: "out",
      label: approvalRequired ? "Approval" : "Output",
      kind: "sink",
      status: "draft" as const,
    },
  ]

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Create Workflow"
        description="Define trigger, ordered steps, and optional approval gate."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/agents/workflows" />}>
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
        submitLabel="Create workflow"
        submitting={create.status === "pending"}
      >
        {step === 0 ? (
          <div className="grid gap-3">
            <Field label="Workflow name">
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label="Trigger">
              <Input
                value={trigger}
                onChange={(e) => setTrigger(e.target.value)}
                placeholder="Pipeline event: quality_check_failed"
              />
            </Field>
          </div>
        ) : null}
        {step === 1 ? (
          <div className="space-y-4">
            <div className="flex flex-wrap gap-2">
              {STEP_KINDS.map((kind) => {
                const active = stepKinds.includes(kind)
                return (
                  <button
                    key={kind}
                    type="button"
                    onClick={() =>
                      setStepKinds((prev) =>
                        active ? prev.filter((k) => k !== kind) : [...prev, kind]
                      )
                    }
                    className={cn(
                      "rounded-full border px-3 py-1 text-xs font-medium",
                      active
                        ? "border-primary bg-primary/15 text-primary"
                        : "border-border text-muted-foreground"
                    )}
                  >
                    {kind}
                  </button>
                )
              })}
            </div>
            <FlowCanvas nodes={previewNodes} />
          </div>
        ) : null}
        {step === 2 ? (
          <div className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
            <div>
              <p className="text-sm font-medium">Require human approval</p>
              <p className="text-xs text-muted-foreground">
                High-risk tool actions pause until an approver acts.
              </p>
            </div>
            <Switch checked={approvalRequired} onCheckedChange={setApprovalRequired} />
          </div>
        ) : null}
        {step === 3 ? (
          <FormReviewSummary
            sections={[
              {
                title: "Workflow",
                items: [
                  { label: "Name", value: name },
                  { label: "Trigger", value: trigger },
                  { label: "Steps", value: stepKinds.join(" → ") },
                  {
                    label: "Approval",
                    value: approvalRequired ? "Required" : "Autonomous",
                  },
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
