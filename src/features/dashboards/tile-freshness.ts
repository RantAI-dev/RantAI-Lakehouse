import { dataFromLabel } from "@/lib/result-cache"

/**
 * What a dashboard-page tile says about its data time: the hover chip (`dataFrom`) and the
 * ⋯ menu's one short line (`menuNote`), both built from `dataFromLabel`, the one wording the
 * embed and report views use too.
 *
 * BI-4-AC1, owner acceptance 2026-11-16: BI-61 had stripped "· remembered" from the chip,
 * the ⋯ line and its tooltip; a result served from memory says so again in all three.
 */
export function tileFreshness(
  computedAt: string | undefined,
  timeZone: string | undefined,
  remembered: boolean | undefined,
  reads: string | null,
  readsDetail: string | null,
): { dataFrom: string | null; menuNote: { label: string; detail: string } | undefined } {
  const dataFrom = dataFromLabel(computedAt, timeZone, remembered)
  const clock = dataFrom?.replace(/^Data from /, "")
  const menuNote = clock || reads
    ? {
        label: [clock, reads].filter(Boolean).join(" · "),
        detail: [dataFrom ? `${dataFrom.replace(" · remembered", ", remembered")}.` : null, readsDetail].filter(Boolean).join(" "),
      }
    : undefined
  return { dataFrom, menuNote }
}
