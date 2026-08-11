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
import { Textarea } from "@/components/ui/textarea"
import { useServiceAction } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import { governanceService } from "@/services"

const STEPS: FormStep[] = [
  { id: "basics", label: "Basics", description: "Name and kind" },
  { id: "scope", label: "Scope", description: "Subjects and resources" },
  { id: "rules", label: "Rules", description: "Effect and conditions" },
  { id: "impact", label: "Impact", description: "Preview" },
  { id: "review", label: "Review", description: "Activate optional" },
]

const KINDS = ["Row filter", "Column mask", "Agent autonomy", "Retention", "Residency"]

export function PolicyCreatePage() {
  const router = useRouter()
  const [step, setStep] = React.useState(0)
  const [name, setName] = React.useState("")
  const [kind, setKind] = React.useState(KINDS[0])
  const [subjects, setSubjects] = React.useState("")
  const [resources, setResources] = React.useState("")
  const [effect, setEffect] = React.useState("Permit with obligation")
  const [conditions, setConditions] = React.useState("")
  const [activate, setActivate] = React.useState(false)
  const create = useServiceAction((signal, input: Parameters<typeof governanceService.createPolicy>[0]) =>
    governanceService.createPolicy(input, signal)
  )

  const canProceed =
    (step === 0 && Boolean(name.trim() && kind)) ||
    (step === 1 && Boolean(subjects.trim() && resources.trim())) ||
    (step === 2 && Boolean(effect.trim())) ||
    step === 3 ||
    step === 4

  async function handleSubmit() {
    const result = await create.run({
      name: name.trim(),
      kind,
      subjects: subjects.trim(),
      resources: resources.trim(),
      effect: effect.trim(),
      conditions: conditions.trim() || undefined,
      activate,
    })
    if (result) router.push("/governance/policies")
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Create Policy"
        description="Draft subjects, resources, and effects before optional activation."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/governance/policies" />}>
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
        submitLabel={activate ? "Create & activate" : "Create draft"}
        submitting={create.status === "pending"}
      >
        {step === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Policy name" className="sm:col-span-2">
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label="Kind" className="sm:col-span-2">
              <select
                className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
                value={kind}
                onChange={(e) => setKind(e.target.value)}
              >
                {KINDS.map((k) => (
                  <option key={k} value={k}>{k}</option>
                ))}
              </select>
            </Field>
          </div>
        ) : null}
        {step === 1 ? (
          <div className="grid gap-3">
            <Field label="Subjects">
              <Input
                value={subjects}
                onChange={(e) => setSubjects(e.target.value)}
                placeholder="All analysts"
              />
            </Field>
            <Field label="Resources">
              <Input
                value={resources}
                onChange={(e) => setResources(e.target.value)}
                placeholder="tenant-scoped tables"
              />
            </Field>
          </div>
        ) : null}
        {step === 2 ? (
          <div className="grid gap-3">
            <Field label="Effect">
              <Input value={effect} onChange={(e) => setEffect(e.target.value)} />
            </Field>
            <Field label="Conditions">
              <Textarea
                value={conditions}
                onChange={(e) => setConditions(e.target.value)}
                rows={3}
                placeholder="classification != restricted OR site = jakarta"
              />
            </Field>
          </div>
        ) : null}
        {step === 3 ? (
          <div className="space-y-3 rounded-lg border border-border bg-muted/30 p-4 text-sm">
            <p className="font-medium">Impact preview (mock)</p>
            <ul className="list-inside list-disc text-muted-foreground">
              <li>~24 users matching subjects</li>
              <li>~18 assets matching resources</li>
              <li>No conflicting active policies detected</li>
            </ul>
          </div>
        ) : null}
        {step === 4 ? (
          <>
            <FormReviewSummary
              sections={[
                {
                  title: "Policy",
                  items: [
                    { label: "Name", value: name },
                    { label: "Kind", value: kind },
                    { label: "Subjects", value: subjects },
                    { label: "Resources", value: resources },
                    { label: "Effect", value: effect },
                    { label: "Conditions", value: conditions || "—" },
                  ],
                },
              ]}
            />
            <div className="mt-4 flex items-center justify-between rounded-lg border border-border px-3 py-2">
              <div>
                <p className="text-sm font-medium">Activate on create</p>
                <p className="text-xs text-muted-foreground">
                  Leave off to keep the policy as a draft.
                </p>
              </div>
              <Switch checked={activate} onCheckedChange={setActivate} />
            </div>
          </>
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
