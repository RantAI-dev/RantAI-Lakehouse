"use client"

import * as React from "react"
import Link from "next/link"
import { ArrowLeftIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/**
 * `WEB-7` D1: the shell every create/edit form page uses.
 *
 * A form that used to open in a side panel (`CreateSheet`) is now its own
 * page: a max-width column under the normal app frame (so the `ADM-13`
 * route guard applies to it), a back link and title at the top, the fields
 * in the middle, and Cancel and Save at the bottom.
 *
 * The submitting label is a required prop on purpose. The sheet said
 * "Creating…" even while editing; here each page states "Creating…" or
 * "Saving…", and there is no default that could be wrong for one of them.
 */
export function FormPage({
  title,
  description,
  backHref,
  backLabel = "Back",
  submitLabel,
  submittingLabel,
  canSubmit,
  submitting = false,
  error,
  onSubmit,
  onCancel,
  children,
  className,
}: {
  title: string
  description?: string
  /** Where the back link and a successful save go. */
  backHref: string
  backLabel?: string
  submitLabel: string
  /** "Creating…" on a new page, "Saving…" on an edit page. */
  submittingLabel: string
  canSubmit: boolean
  submitting?: boolean
  /** The API's own sentence, shown above the footer instead of a toast. */
  error?: string | null
  onSubmit: () => void
  onCancel: () => void
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("mx-auto flex w-full max-w-2xl flex-col gap-5", className)}>
      <div className="flex flex-col gap-2 border-b border-border pb-4">
        <Link
          href={backHref}
          className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeftIcon className="size-4" aria-hidden />
          {backLabel}
        </Link>
        <div className="min-w-0">
          <h1 className="text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
            {title}
          </h1>
          {description ? (
            <p className="mt-1 text-sm leading-5 text-muted-foreground">{description}</p>
          ) : null}
        </div>
      </div>

      <div className="flex flex-col gap-4">{children}</div>

      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex items-center justify-end gap-2 border-t border-border pt-4">
        <Button type="button" variant="outline" disabled={submitting} onClick={onCancel}>
          Cancel
        </Button>
        <Button type="button" disabled={!canSubmit || submitting} onClick={onSubmit}>
          {submitting ? submittingLabel : submitLabel}
        </Button>
      </div>
    </div>
  )
}
