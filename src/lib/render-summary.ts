import type { FilterDef } from "@/services/clients/bi-store"
import type { ParamControl } from "@/lib/params"

/**
 * The lines of the PDF's first page (`BI-1`, feature decision 4): which filters
 * and parameter values the dashboard was printed with, who asked, and when.
 * Plain text, so the page and its test read the same sentence.
 */

/** One active filter as a line: `region: north, south`, `visits: 10 to 20`. */
export function filterLine(f: FilterDef): string {
  if (f.segment) return `Segment ${f.segment}`
  if (f.op === "relative") {
    const unit = f.unit ?? "day"
    const phrase =
      f.anchor === "this" ? `this ${unit}`
      : f.anchor === "previous" ? `previous ${unit}`
      : f.anchor === "next" ? `next ${f.n ?? 1} ${unit}s`
      : `last ${f.n ?? 1} ${unit}s`
    return `${f.column}: ${phrase}`
  }
  if (f.op === "between" || f.min !== undefined || f.max !== undefined) {
    return `${f.column}: ${f.min ?? "…"} to ${f.max ?? "…"}`
  }
  if (f.text !== undefined && f.op && f.op !== "in" && f.op !== "not_in") {
    return `${f.column} ${f.op.replaceAll("_", " ")} “${f.text}”`
  }
  const verb = f.op === "not_in" ? " not in" : ""
  return `${f.column}${verb}: ${f.values.join(", ")}`
}

/**
 * The paper's printable width in CSS pixels (`BI-53`, D17; QA `LH-BI-1-01`
 * F1, `LH-BI-6-22`, `LH-BI-6-23`): the paper's width
 * less the two 0.5 in side margins the API sends, at 96 px per inch, rounded
 * down. A4 is 8.27 by 11.7 in and Letter 8.5 by 11 (anything else reads as
 * A4); `landscape` takes the longer side, anything else the shorter: 697, 1027,
 * 720 and 960.
 *
 * `lakehouse-api/src/pdf.rs` (`Paper::inches`, the margins of
 * `convert_url`) is the other owner of these numbers: whoever changes one
 * changes the other.
 */
export function paperContentWidth(paper = "a4", orientation = "portrait"): number {
  const [short, long] = paper === "letter" ? [8.5, 11] : [8.27, 11.7]
  const side = orientation === "landscape" ? long : short
  return Math.floor((side - 1) * 96)
}

/** `Region: North` for each parameter the dashboard shows, with the value used. */
export function paramLines(controls: readonly ParamControl[] | undefined): string[] {
  return (controls ?? []).map((c) => `${c.param.label || c.param.name}: ${String(c.value)}`)
}

/** `17 Oct 2026, 14:05` in the report time zone and the deployment's number locale. */
export function issuedLine(issuedAtSeconds: number | undefined, timeZone: string, locale = "en-US"): string {
  if (!issuedAtSeconds) return ""
  try {
    return new Intl.DateTimeFormat(locale, {
      timeZone,
      day: "numeric",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date(issuedAtSeconds * 1000))
  } catch {
    // An unknown zone name: say the time in UTC rather than nothing.
    return new Date(issuedAtSeconds * 1000).toISOString().replace("T", " ").slice(0, 16) + " UTC"
  }
}
