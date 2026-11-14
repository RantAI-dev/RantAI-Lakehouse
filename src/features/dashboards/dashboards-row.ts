import * as React from "react"

/**
 * Ingatan baris menu "Dashboards": dashboard atau laporan yang terakhir dibuka,
 * supaya baris itu bernama sesuai isinya dan langsung menuju ke sana.
 *
 * Sengaja terpisah dari `last-board.ts`, yang memberi makan resolver `/dashboards`
 * dan Home: membuka Browse mengosongkan baris ini, tetapi tidak boleh membuat
 * Home kehilangan board terakhirnya.
 *
 * localStorage, per-browser. Perubahan di tab ini diumumkan lewat event, di tab
 * lain lewat `storage`.
 */
export type RowKind = "dashboard" | "report"
export type RowMemory = { kind: RowKind; id: string; name: string }

const ROW_KEY = "dashboards:menu-row"
const ROW_EVENT = "dashboards:menu-row-change"

function announce() {
  try {
    window.dispatchEvent(new Event(ROW_EVENT))
  } catch {
    /* ignore */
  }
}

function readRaw(): string | null {
  if (typeof window === "undefined") return null
  try {
    return window.localStorage.getItem(ROW_KEY)
  } catch {
    return null
  }
}

function parse(raw: string | null): RowMemory | null {
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as Partial<RowMemory>
    if ((v.kind !== "dashboard" && v.kind !== "report") || typeof v.id !== "string" || !v.id) return null
    if (typeof v.name !== "string" || !v.name.trim()) return null
    return { kind: v.kind, id: v.id, name: v.name }
  } catch {
    return null
  }
}

export function readRow(): RowMemory | null {
  return parse(readRaw())
}

/** Dipanggil saat halaman dashboard atau laporan terbuka, dan lagi saat namanya berubah. */
export function rememberRow(kind: RowKind, id: string, name: string) {
  if (typeof window === "undefined" || !id || !name.trim()) return
  try {
    const next = JSON.stringify({ kind, id, name })
    if (window.localStorage.getItem(ROW_KEY) === next) return
    window.localStorage.setItem(ROW_KEY, next)
  } catch {
    return
  }
  announce()
}

/** Tanpa argumen: kosongkan (Browse dibuka). Dengan item: hanya kalau itu yang diingat (item dihapus). */
export function forgetRow(item?: { kind: RowKind; id: string }) {
  if (typeof window === "undefined") return
  try {
    if (item) {
      const cur = readRow()
      if (!cur || cur.kind !== item.kind || cur.id !== item.id) return
    }
    if (window.localStorage.getItem(ROW_KEY) === null) return
    window.localStorage.removeItem(ROW_KEY)
  } catch {
    return
  }
  announce()
}

function subscribe(onChange: () => void) {
  window.addEventListener(ROW_EVENT, onChange)
  window.addEventListener("storage", onChange)
  return () => {
    window.removeEventListener(ROW_EVENT, onChange)
    window.removeEventListener("storage", onChange)
  }
}

/** Server dan render pertama memberi null, jadi baris menulis "Dashboards" tanpa hydration mismatch. */
export function useDashboardsRow(): RowMemory | null {
  const raw = React.useSyncExternalStore(subscribe, readRaw, () => null)
  return React.useMemo(() => parse(raw), [raw])
}
