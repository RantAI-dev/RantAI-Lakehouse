/**
 * Copy `text` to the clipboard; resolves to whether it worked.
 *
 * `navigator.clipboard` exists only in a secure context (https or localhost).
 * A console opened over plain http on a LAN address has none, so a bare
 * `navigator.clipboard.writeText` throws there. This tries the async API when
 * it is available and the context is secure, and otherwise falls back to a
 * temporary off-screen textarea and `document.execCommand("copy")`. A caller
 * must show "Copied" only when this resolves `true`.
 */
export async function copyText(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText && globalThis.isSecureContext !== false) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // Permission denied or similar: try the fallback below.
    }
  }
  return copyWithTextarea(text)
}

function copyWithTextarea(text: string): boolean {
  if (typeof document === "undefined" || !document.body) return false
  const area = document.createElement("textarea")
  area.value = text
  area.setAttribute("readonly", "")
  area.style.position = "fixed"
  area.style.top = "-1000px"
  area.style.opacity = "0"
  // Inside an open dialog the page outside it is inert, so the element is
  // added to the active element's container when there is one.
  const host = document.activeElement?.closest("[role=dialog]") ?? document.body
  host.appendChild(area)
  const previous = document.activeElement as HTMLElement | null
  try {
    area.focus()
    area.select()
    return document.execCommand("copy")
  } catch {
    return false
  } finally {
    area.remove()
    previous?.focus?.()
  }
}
