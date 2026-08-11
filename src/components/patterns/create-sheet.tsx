"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"

/**
 * Shared create sheet: title, body fields, validate + submit footer.
 * Layout follows shadcn sheet anatomy — header, scroll body, sticky footer.
 */
export function CreateSheet({
  open,
  onOpenChange,
  title,
  description,
  submitLabel = "Create",
  submitting = false,
  canSubmit,
  onSubmit,
  error,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  submitLabel?: string
  submitting?: boolean
  canSubmit: boolean
  onSubmit: () => void
  error?: string | null
  children: React.ReactNode
}) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 sm:max-w-md">
        <SheetHeader className="border-b border-border">
          <SheetTitle>{title}</SheetTitle>
          {description ? <SheetDescription>{description}</SheetDescription> : null}
        </SheetHeader>
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 py-5">
          {children}
          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>
        <SheetFooter>
          <Button
            type="button"
            variant="outline"
            disabled={submitting}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!canSubmit || submitting}
            onClick={onSubmit}
          >
            {submitting ? "Creating…" : submitLabel}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  )
}
