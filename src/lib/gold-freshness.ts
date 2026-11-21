/**
 * Shared freshness helpers for Gold per-mart publication (DATA-10 D4 /
 * PR slice D review D-S5). Reads `changedSinceExport`, the ClickHouse content
 * fingerprint comparison between the current mart and its newest successful
 * export run, avoiding false-positive "Out of date" signals caused by
 * background merges or full table rewrites.
 *
 * `DATA-49` (QA `LH-DATA-1-01`) adds a first state: a failed newest run is
 * "Last publish failed", whatever the last success says. The order is that
 * failure, then "Never published", "Not measured", "Out of date" and "Up to
 * date" — the card must not read "Up to date" through failed runs.
 */
import type { GoldPublication } from "@/services/contracts/gold"

export function freshnessLine(pub: GoldPublication): string {
  if (pub.lastFailure) return "Last publish failed"
  if (pub.lastExportedAt === null) return "Never published"
  if (pub.changedSinceExport === null || pub.changedSinceExport === undefined) return "Not measured"
  if (pub.changedSinceExport) return "Out of date"
  return "Up to date"
}

export function publicationLabel(pub: GoldPublication): string {
  if (!pub.enabled) return "Off"
  return `On · ${freshnessLine(pub)}`
}
