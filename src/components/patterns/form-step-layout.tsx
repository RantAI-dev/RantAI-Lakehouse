"use client"

import * as React from "react"
import { CheckIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

export type FormStep = {
  id: string
  label: string
  description?: string
}

/**
 * Shared multi-step wizard shell: left stepper, content slot, and footer
 * with Previous / Next / submit. Next stays disabled until `canProceed`.
 */
export function FormStepLayout({
  steps,
  currentIndex,
  onStepChange,
  canProceed,
  onSubmit,
  submitLabel = "Create",
  submitting = false,
  children,
  className,
}: {
  steps: FormStep[]
  currentIndex: number
  onStepChange: (index: number) => void
  canProceed: boolean
  onSubmit: () => void
  submitLabel?: string
  submitting?: boolean
  children: React.ReactNode
  className?: string
}) {
  const isLast = currentIndex >= steps.length - 1
  const [maxReached, setMaxReached] = React.useState(currentIndex)
  React.useEffect(() => {
    setMaxReached((prev) => Math.max(prev, currentIndex))
  }, [currentIndex])

  return (
    <div
      className={cn(
        "grid gap-6 lg:grid-cols-[220px_minmax(0,1fr)]",
        className
      )}
    >
      <ol className="flex flex-row gap-2 overflow-x-auto lg:flex-col lg:gap-1">
        {steps.map((step, index) => {
          const active = index === currentIndex
          const completed = index < currentIndex
          const reachable = index <= maxReached
          return (
            <li key={step.id}>
              <button
                type="button"
                disabled={!reachable}
                onClick={() => {
                  if (reachable) onStepChange(index)
                }}
                className={cn(
                  "flex w-full items-start gap-3 rounded-lg px-3 py-2 text-left transition-colors",
                  active && "bg-muted",
                  !active && reachable && "hover:bg-muted/60",
                  !reachable && "opacity-50"
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-medium",
                    active && "border-primary bg-primary text-primary-foreground",
                    completed && "border-primary bg-primary/15 text-primary",
                    !active && !completed && "border-border text-muted-foreground"
                  )}
                >
                  {completed ? <CheckIcon className="size-3.5" /> : index + 1}
                </span>
                <span className="min-w-0">
                  <span
                    className={cn(
                      "block text-sm font-medium",
                      active ? "text-foreground" : "text-muted-foreground"
                    )}
                  >
                    {step.label}
                  </span>
                  {step.description ? (
                    <span className="mt-0.5 hidden text-xs text-muted-foreground lg:block">
                      {step.description}
                    </span>
                  ) : null}
                </span>
              </button>
            </li>
          )
        })}
      </ol>

      <div className="flex min-h-[320px] flex-col rounded-xl border border-border">
        <div className="flex-1 space-y-4 p-5">{children}</div>
        <div className="flex items-center justify-between gap-2 border-t border-border px-5 py-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={currentIndex === 0 || submitting}
            onClick={() => onStepChange(currentIndex - 1)}
          >
            Previous
          </Button>
          {isLast ? (
            <Button
              type="button"
              size="sm"
              disabled={!canProceed || submitting}
              onClick={onSubmit}
            >
              {submitting ? "Creating…" : submitLabel}
            </Button>
          ) : (
            <Button
              type="button"
              size="sm"
              disabled={!canProceed || submitting}
              onClick={() => onStepChange(currentIndex + 1)}
            >
              Next
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
