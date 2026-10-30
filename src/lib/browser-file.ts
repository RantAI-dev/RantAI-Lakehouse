import { readableErrorMessage } from "@/lib/notify-message"

/**
 * The three helpers every download shares: the name the server gave the
 * file, the hand-off to the browser, and the sentence of a refusal.
 *
 * They live here, and not in a feature, because a service client needs
 * them too: the Query Studio download fetches the file itself, and a
 * client may not import from `src/features`.
 */

/**
 * The name of the file the server sent: `Visits.csv` out of
 * `attachment; filename="Visits.csv"`.
 *
 * `fallback` is the name used when the header names no file; the caller
 * that knows what kind of file it asked for names it.
 */
export function fileNameOf(disposition: string | null, fallback = "table.csv"): string {
  return /filename="([^"]+)"/.exec(disposition ?? "")?.[1] ?? fallback
}

/** Save `blob` under `fileName` through a temporary link (the one place a download is handed to the browser). */
export function saveBlob(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = fileName
  document.body.appendChild(a)
  a.click()
  a.remove()
  URL.revokeObjectURL(url)
}

/**
 * The sentence for a refused download, or `fallback`. A body that starts with an
 * API error code (`permission_denied: ...`) reads as that code's hint, not as
 * the code (`readableErrorMessage`, BI-18 part C review fix (SHOULD-FIX) R20,
 * applied to BI-1's routes by the phase 3 merge).
 */
export async function refusalOf(res: Response, fallback: string): Promise<string> {
  const json = (await res.json().catch(() => null)) as { error?: unknown } | null
  return typeof json?.error === "string" ? readableErrorMessage(json.error) : fallback
}
