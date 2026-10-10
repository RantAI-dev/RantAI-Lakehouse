import type { ColumnSetting } from "./table-types"

/**
 * How a table cell reads (`BI-16` part A). The server returns raw values;
 * the column's setting decides how they look. Currency is the browser's
 * Indonesian formatting (`Rp`), without conversion (feature-page decision 7).
 */
export type CellContent =
  | { kind: "text"; text: string }
  | { kind: "link"; href: string; text: string }
  | { kind: "image"; src: string; alt: string }

/** The value as a number, or `null` for anything that is not one (a quoted `UInt64` counts). */
export function asNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null
  if (typeof value === "string" && value.trim() !== "") {
    const n = Number(value)
    return Number.isFinite(n) ? n : null
  }
  return null
}

/**
 * A link cell opens only `http:` and `https:` (decision 6); anything else,
 * `javascript:` and `data:` included, is plain text.
 */
export function safeHttpUrl(value: unknown): string | null {
  if (typeof value !== "string") return null
  try {
    const url = new URL(value.trim())
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null
  } catch {
    return null
  }
}

/** An image cell loads only `https:`: an `http:` image would be mixed content and a tracker over the clear. */
export function safeImageUrl(value: unknown): string | null {
  const href = safeHttpUrl(value)
  return href !== null && href.startsWith("https:") ? href : null
}

function fractionOptions(decimals: number | undefined, fallbackMax: number): Intl.NumberFormatOptions {
  return decimals === undefined
    ? { maximumFractionDigits: fallbackMax }
    : { minimumFractionDigits: decimals, maximumFractionDigits: decimals }
}

const DATE = new Intl.DateTimeFormat("en-GB", { timeZone: "UTC", day: "2-digit", month: "short", year: "numeric" })

/**
 * A date or timestamp as `05 Mar 2026`; text that is not a date is returned as
 * it came. The engine prints `YYYY-MM-DD` and `YYYY-MM-DD HH:MM:SS` in the
 * report zone, so the calendar day is read off the text and the browser's own
 * zone never moves it; a timestamp shows its day only.
 */
export function formatDate(value: unknown): string {
  const raw = String(value ?? "")
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T]|$)/.exec(raw.trim())
  const d = m ? new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]))) : typeof value === "number" ? new Date(value) : null
  return d && !Number.isNaN(d.getTime()) ? DATE.format(d) : raw
}

/** What one cell shows under its column's setting. `null` is an empty cell. */
export function cellContent(value: unknown, setting?: ColumnSetting): CellContent {
  if (value === null || value === undefined) return { kind: "text", text: "" }
  const format = setting?.format ?? "auto"
  const decimals = setting?.decimals
  const n = asNumber(value)
  switch (format) {
    case "link": {
      const href = safeHttpUrl(value)
      return href ? { kind: "link", href, text: String(value) } : { kind: "text", text: String(value) }
    }
    case "image": {
      const src = safeImageUrl(value)
      return src ? { kind: "image", src, alt: "" } : { kind: "text", text: String(value) }
    }
    case "date":
      return { kind: "text", text: formatDate(value) }
    case "percent":
      // A ratio, as `Intl` reads it: 0.25 is 25%.
      return { kind: "text", text: n === null ? String(value) : new Intl.NumberFormat("id-ID", { style: "percent", ...fractionOptions(decimals, 2) }).format(n) }
    case "currency":
      return { kind: "text", text: n === null ? String(value) : new Intl.NumberFormat("id-ID", { style: "currency", currency: "IDR", ...fractionOptions(decimals ?? 0, 0) }).format(n) }
    case "number":
      return { kind: "text", text: n === null ? String(value) : n.toLocaleString("id-ID", fractionOptions(decimals, 3)) }
    default:
      // `auto`: a JSON number reads as a number; a quoted one is left as text.
      if (typeof value === "number") return { kind: "text", text: value.toLocaleString("id-ID", fractionOptions(decimals, 3)) }
      return { kind: "text", text: String(value) }
  }
}

/** The plain text of a cell, for a CSV or a one-record view. */
export function cellText(value: unknown, setting?: ColumnSetting): string {
  const c = cellContent(value, setting)
  return c.kind === "image" ? c.src : c.text
}
