/**
 * Shared freshness helpers for Gold per-mart publication (DATA-10 D4 /
 * PR slice D review D-S5). Reads `changedSinceExport`, the ClickHouse content
 * fingerprint comparison between the current mart and its newest successful
 * export run, avoiding false-positive "Out of date" signals caused by
 * background merges or full table rewrites.
 */
import type { GoldPublication } from "@/services/contracts/gold"

export function freshnessLine(pub: GoldPublication): string {
  if (pub.lastExportedAt === null) return "Never published"
  if (pub.changedSinceExport === null || pub.changedSinceExport === undefined) return "Not measured"
  if (pub.changedSinceExport) return "Out of date"
  return "Up to date"
}

export function publicationLabel(pub: GoldPublication): string {
  if (!pub.enabled) return "Off"
  return `On · ${freshnessLine(pub)}`
}
