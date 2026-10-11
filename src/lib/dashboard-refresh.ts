/**
 * The dashboard's auto-refresh interval (BI-18 part B). An editor can save
 * one as the board's default; whoever views it may change it for their own
 * session, and that change is never written back. The list mirrors the
 * server's (`lakehouse_bi::click::REFRESH_SECONDS`), which refuses any other
 * number.
 */

/** Metabase's intervals (decision 4 of the feature page), in seconds; 0 is off. */
export const REFRESH_OPTIONS = [
  { seconds: 0, label: "Manual" },
  { seconds: 60, label: "Every 1m" },
  { seconds: 300, label: "Every 5m" },
  { seconds: 600, label: "Every 10m" },
  { seconds: 900, label: "Every 15m" },
  { seconds: 1800, label: "Every 30m" },
  { seconds: 3600, label: "Every 60m" },
] as const

const LISTED: ReadonlySet<number> = new Set(REFRESH_OPTIONS.map((o) => o.seconds))

/** A number from the server or a stored value, or 0 (off) when it is not a listed interval. */
export function listedInterval(value: unknown): number {
  return typeof value === "number" && LISTED.has(value) ? value : 0
}

/**
 * The interval in force: the viewer's own choice for this session when
 * there is one, else the board's saved default, else off.
 */
export function effectiveRefresh(saved: unknown, session: number | null): number {
  return session !== null && LISTED.has(session) ? session : listedInterval(saved)
}

/** Whether "Save as dashboard default" has anything to save. */
export function canSaveRefresh(args: { mayWrite: boolean; saved: unknown; effective: number }): boolean {
  return args.mayWrite && args.effective !== listedInterval(args.saved)
}

export function refreshLabel(seconds: number): string {
  return REFRESH_OPTIONS.find((o) => o.seconds === seconds)?.label ?? "Manual"
}
