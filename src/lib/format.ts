/**
 * Shared display formatters. All modules must use these instead of local
 * helpers so dates, durations, bytes, rates, and costs read identically.
 */

/** 1234567 → "1.2M"; 950 → "950". */
export function formatCompactNumber(value: number): string {
  return Intl.NumberFormat("en", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value)
}

/** 1234567 → "1,234,567". */
export function formatNumber(value: number): string {
  return Intl.NumberFormat("en").format(value)
}

/** Bytes → short human string, e.g. 1536 → "1.5 KB", 2.4e12 → "2.4 TB". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—"
  const units = ["B", "KB", "MB", "GB", "TB", "PB"]
  let v = bytes
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i += 1
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`
}

/** Milliseconds → "840 ms", "2.4 s", "3m 12s", "1h 04m". */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—"
  if (ms < 1000) return `${Math.round(ms)} ms`
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)} s`
  const m = Math.floor(s / 60)
  const rs = Math.round(s % 60)
  if (m < 60) return `${m}m ${String(rs).padStart(2, "0")}s`
  const h = Math.floor(m / 60)
  return `${h}h ${String(m % 60).padStart(2, "0")}m`
}

/** Internal cost units, e.g. 0.0421 → "0.0421 cu". */
export function formatCost(units: number): string {
  if (!Number.isFinite(units)) return "—"
  const digits = units >= 10 ? 1 : units >= 1 ? 2 : 4
  return `${units.toFixed(digits)} cu`
}

/** Percentage with one decimal, e.g. 0.634 → "63.4%". */
export function formatPercent(fraction: number): string {
  if (!Number.isFinite(fraction)) return "—"
  return `${(fraction * 100).toFixed(1)}%`
}

/** ISO timestamp → "12 Jun 2026, 09:41". */
export function formatDateTime(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return "—"
  return Intl.DateTimeFormat("en", {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(d)
}

/** ISO timestamp → relative age, e.g. "4m ago", "3h ago", "2d ago". */
export function formatRelativeTime(iso: string, now = Date.now()): string {
  const t = new Date(iso).getTime()
  if (Number.isNaN(t)) return "—"
  const diffMs = now - t
  if (diffMs < 0) return "just now"
  const mins = Math.floor(diffMs / 60_000)
  if (mins < 1) return "just now"
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  const months = Math.floor(days / 30)
  return `${months}mo ago`
}

/** Freshness lag in seconds → "8 s", "4m", "3h 20m". */
export function formatLagSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—"
  if (seconds < 60) return `${Math.round(seconds)} s`
  const m = Math.floor(seconds / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  return `${h}h ${String(m % 60).padStart(2, "0")}m`
}

/** Events/records per second, e.g. 15400 → "15.4K rec/s". */
export function formatRate(perSecond: number): string {
  if (!Number.isFinite(perSecond)) return "—"
  return `${formatCompactNumber(perSecond)} rec/s`
}
