"use client"

import * as React from "react"
import Link from "next/link"
import { Sparkles } from "lucide-react"

export type StartProps = {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  href: string
  onAi?: () => void
}

/**
 * A thing to create, as a bordered button under the composer: the label
 * opens the manual flow, and the sparkle at its end hands the same task
 * to Copilot. The description is the tooltip. A shortcut with no `onAi`
 * has no sparkle: it is not offered a hand-off it has no prompt for.
 */
export function StartButton({ icon: Icon, title, description, href, onAi }: StartProps) {
  return (
    <div className="flex items-stretch overflow-hidden rounded-xl border border-border bg-background transition-colors hover:border-foreground/25">
      <Link
        href={href}
        title={description}
        className="flex min-w-0 flex-1 items-center gap-2.5 px-3.5 py-2.5 text-sm font-medium outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <Icon className="size-4 shrink-0 text-muted-foreground" />
        <span className="truncate">{title}</span>
      </Link>
      {onAi ? (
        <button
          type="button"
          onClick={onAi}
          aria-label={`${title}: let AI do it`}
          title="Let AI do it"
          className="inline-flex items-center border-l border-border px-3 text-[var(--brand-1)] transition-colors hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <Sparkles className="size-4" />
        </button>
      ) : null}
    </div>
  )
}
