/**
 * Real full screen for a dashboard (BI-18 part B): the Fullscreen API where
 * the browser allows it, the in-page view (the page's own `fixed inset-0`
 * overlay) where it does not. The overlay is always applied while the
 * dashboard is "full screen"; the API only adds the browser's own full
 * screen on top, so a refusal costs nothing but the browser chrome staying.
 *
 * It is requested on the document element rather than on the dashboard's
 * container: under an element's full screen the browser draws only that
 * element's subtree, and the menus, dialogs and tooltips of this console
 * are portalled to the body, so they would vanish (the "..." menu among
 * them, with the way out of the full screen).
 *
 * Kept free of React so the decision and the fallback are tested with a
 * fake document.
 */

/** The part of `Document` this module uses. */
export type FullscreenDoc = {
  fullscreenEnabled?: boolean
  fullscreenElement?: unknown
  documentElement: { requestFullscreen?: () => Promise<void> }
  exitFullscreen?: () => Promise<void>
}

/** What entering did: the browser's full screen, or the in-page view alone. */
export type FullscreenOutcome = "native" | "in-page"

/**
 * Ask the browser for full screen. Resolves `"in-page"` when the API is
 * missing, disabled (an iframe without `allowfullscreen`) or refused (not a
 * user gesture); never rejects.
 */
export async function enterFullscreen(doc: FullscreenDoc): Promise<FullscreenOutcome> {
  const request = doc.documentElement.requestFullscreen
  if (doc.fullscreenEnabled !== true || typeof request !== "function") return "in-page"
  try {
    await request.call(doc.documentElement)
    return "native"
  } catch {
    return "in-page"
  }
}

/** Leave the browser's full screen if this document is in it; never rejects. */
export async function leaveFullscreen(doc: FullscreenDoc): Promise<void> {
  if (!doc.fullscreenElement || typeof doc.exitFullscreen !== "function") return
  try {
    await doc.exitFullscreen()
  } catch {
    // Already out (the user pressed Esc first); nothing to undo.
  }
}

/** Where the dark choice of the full screen is remembered: this browser only. */
export const FULLSCREEN_DARK_KEY = "lh_dashboard_fullscreen_dark"

/** The remembered dark choice; false when none, or when storage is unavailable. */
export function readFullscreenDark(storage: Pick<Storage, "getItem"> | undefined = safeStorage()): boolean {
  try {
    return storage?.getItem(FULLSCREEN_DARK_KEY) === "true"
  } catch {
    return false
  }
}

/** Remember the dark choice. Best effort: a lost write costs only the memory. */
export function writeFullscreenDark(on: boolean, storage: Pick<Storage, "setItem"> | undefined = safeStorage()): void {
  try {
    storage?.setItem(FULLSCREEN_DARK_KEY, on ? "true" : "false")
  } catch {
    // Private mode or blocked site data.
  }
}

/** `localStorage`, or undefined where even touching it throws. */
function safeStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage
  } catch {
    return undefined
  }
}
