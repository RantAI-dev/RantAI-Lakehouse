"use client"

import { Check, Wand2, X } from "lucide-react"

import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import type { QueryEngine } from "@/services/contracts/queries"
import { MiniMarkdown } from "@/features/copilot/mini-markdown"
import { lineDiff, type DiffLine } from "@/lib/line-diff"
import type { SqlFixOutcome } from "@/services/contracts/queries"

/** The sign and the colour both say what a line did; colour alone would not reach a colour-blind reader. */
const DIFF_MARK: Record<Exclude<DiffLine["kind"], "same">, { sign: string; className: string; label: string }> = {
  removed: { sign: "−", className: "bg-red-500/10 text-red-700 dark:text-red-400", label: "removed" },
  added: { sign: "+", className: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400", label: "added" },
}

/**
 * "Fix with AI" beside a failed run. Disabled for Trino, where a proposal
 * cannot be checked yet; the reason is the button's title, so the person
 * sees why without a message to dismiss.
 */
export function FixWithAiButton({
  engine,
  busy,
  onFix,
}: {
  readonly engine: QueryEngine
  readonly busy: boolean
  readonly onFix: () => void
}) {
  const trino = engine === "trino"
  return (
    <Button
      size="sm"
      variant="outline"
      onClick={onFix}
      disabled={trino || busy}
      title={trino ? "fixing a Trino query is not supported yet" : undefined}
    >
      <Wand2 className="size-4" aria-hidden />
      {busy ? "Fixing…" : "Fix with AI"}
    </Button>
  )
}

/** The explanation of the statement, as Markdown, like the Copilot's answers. */
export function ExplainCard({ text }: { readonly text: string }) {
  return (
    <SectionCard title="Explanation">
      <div className="text-sm">
        <MiniMarkdown text={text} />
      </div>
    </SectionCard>
  )
}

/**
 * The assistant's answer to a failed statement. A proposal shows its reason
 * and a line diff against the editor, with Accept and Reject; with no
 * proposal it shows the assistant's text and says nothing was proposed.
 */
export function FixCard({
  outcome,
  editorSql,
  onAccept,
  onReject,
}: {
  readonly outcome: SqlFixOutcome
  /** The editor's statement now, which the diff is taken against. */
  readonly editorSql: string
  readonly onAccept: () => void
  readonly onReject: () => void
}) {
  if (!("sql" in outcome)) {
    return (
      <SectionCard title="Fix with AI">
        <div className="text-sm">
          <MiniMarkdown text={outcome.answer} />
        </div>
        <p className="mt-2 text-sm text-muted-foreground">No fix to apply was proposed.</p>
      </SectionCard>
    )
  }

  const lines = lineDiff(editorSql, outcome.sql)
  return (
    <SectionCard title="Proposed fix">
      <p className="text-sm">{outcome.reason}</p>
      <ol aria-label="Changes to the query" className="mt-3 overflow-auto rounded-md border border-border font-mono text-xs">
        {lines.map((line, i) =>
          line.kind === "same" ? (
            <li key={i} className="px-2 py-0.5 text-muted-foreground">
              <span className="mr-2 select-none" aria-hidden>
                {" "}
              </span>
              {line.text}
            </li>
          ) : (
            <li key={i} className={`px-2 py-0.5 ${DIFF_MARK[line.kind].className}`}>
              <span className="mr-2 select-none" aria-hidden>
                {DIFF_MARK[line.kind].sign}
              </span>
              <span className="sr-only">{DIFF_MARK[line.kind].label}: </span>
              {line.text}
            </li>
          ),
        )}
      </ol>
      <div className="mt-3 flex gap-2">
        <Button size="sm" onClick={onAccept}>
          <Check className="size-4" aria-hidden />
          Accept
        </Button>
        <Button size="sm" variant="outline" onClick={onReject}>
          <X className="size-4" aria-hidden />
          Reject
        </Button>
      </div>
    </SectionCard>
  )
}
