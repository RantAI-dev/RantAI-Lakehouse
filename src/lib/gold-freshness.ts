/**
 * Shared freshness helpers for Gold per-mart publication (PR slice D
 * review D-S5). Parse and compare RFC 3339 timestamps as times, not as
 * strings, so a local timezone offset in one string and a `Z` suffix in
 * another do not cause a spurious ordering flip. Used by both the
 * per-mart detail card and the Gold Exports overview page.
 */
import type { GoldPublication } from "@/services/contracts/gold"

export function freshnessLine(pub: GoldPublication): string {
  if (pub.lastExportedAt === null) return "Never published"
  if (pub.lastChangedAt === null) return "Not measured"
  const changed = Date.parse(pub.lastChangedAt)
  const exported = Date.parse(pub.lastExportedAt)
  if (Number.isNaN(changed) || Number.isNaN(exported)) return "Not measured"
  if (changed < exported) return "Up to date"
  return "Out of date"
}

export function publicationLabel(pub: GoldPublication): string {
  if (!pub.enabled) return "Off"
  return `On · ${freshnessLine(pub)}`
}