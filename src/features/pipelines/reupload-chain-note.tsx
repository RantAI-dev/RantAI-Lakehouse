"use client"

import Link from "next/link"

import { useService } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { pipelineService } from "@/services"
import type { ReuploadChainStep } from "@/services/contracts/pipelines"

type Parts = { before: string; link?: { label: string; href: string }; after?: string }

/**
 * One sentence from the newest re-upload chain step (BLD-14 D8). Every code
 * the API can send has a sentence; an unknown `outcome` is widened, never a
 * crash. The cause's name links to that pipeline only when the API sent a
 * name (it withholds one from another tenant), and the started run's id
 * opens the run the way the schedule history does.
 */
function parts(step: ReuploadChainStep, when: string, pipelineId: string): Parts {
  const source = step.sourceTable
  if (step.outcome === "started") {
    const before = `Ran after the re-upload of ${source} on ${when}.`
    return step.runId
      ? {
          before: `${before} `,
          link: {
            label: step.runId,
            href: `/pipelines/${pipelineId}?tab=runs&run=${encodeURIComponent(step.runId)}`,
          },
        }
      : { before }
  }
  if (step.outcome === "not_run") {
    const before = `Not run after the re-upload of ${source} on ${when}: `
    const tail: Record<string, string> = {
      upstream_failed: " failed first.",
      upstream_cancelled: " was cancelled.",
      wait_expired: " did not finish within 6 hours.",
    }
    const after = tail[step.reason ?? ""] ?? ` (${step.reason ?? "unknown reason"}).`
    const label = step.causePipelineName ?? "a pipeline before it"
    return step.causePipelineName && step.causePipelineId
      ? { before, link: { label, href: `/pipelines/${step.causePipelineId}` }, after }
      : { before: `${before}${label}`, after }
  }
  if (step.outcome === "superseded") {
    return {
      before: `Not run for the re-upload of ${source} on ${when}: a later re-upload of the same table replaced it.`,
    }
  }
  return { before: `After the re-upload of ${source} on ${when}: ${step.outcome}.` }
}

export function ReuploadChainNote({ pipelineId }: { pipelineId: string }) {
  const chain = useService((s) => pipelineService.getReuploadChain(pipelineId, s), [pipelineId])
  if (chain.status !== "success" || !chain.data?.last) return null

  const step = chain.data.last
  const sentence = parts(step, formatDateTime(step.recordedAt), pipelineId)
  return (
    <p className="mb-3 text-xs text-muted-foreground">
      {sentence.before}
      {sentence.link ? (
        <Link href={sentence.link.href} className="font-mono text-primary hover:underline">
          {sentence.link.label}
        </Link>
      ) : null}
      {sentence.after}
    </p>
  )
}
