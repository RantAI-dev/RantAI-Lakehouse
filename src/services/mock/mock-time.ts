/** Time helpers so mock timestamps always read as recent, live data. */

/** ISO timestamp `minutes` ago from now. */
export function agoIso(minutes: number): string {
  return new Date(Date.now() - minutes * 60_000).toISOString()
}

/** ISO timestamp `days` ago from now. */
export function daysAgoIso(days: number): string {
  return agoIso(days * 24 * 60)
}
