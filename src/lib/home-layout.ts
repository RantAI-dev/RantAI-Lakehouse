/**
 * What Home can show, and how a saved layout becomes what is shown.
 *
 * This module owns the catalogue. The server stores a layout as lists of
 * ids and checks only their shape (`routes::home`), so adding a card is a
 * change here and nowhere else, and a layout saved before the card existed
 * keeps working: ids this build does not know are dropped when a saved
 * layout is resolved, never shown and never an error.
 *
 * Everything here is pure so the rules can be tested without a page.
 */
export const CARD_IDS = [
  "dashboard-preview",
  "recent",
  "pipeline-runs",
  "sources",
  "open-alerts",
  "saved-queries",
] as const
export type CardId = (typeof CARD_IDS)[number]

export const SHORTCUT_IDS = [
  "connect-source",
  "create-pipeline",
  "build-dashboard",
  "new-query",
  "browse-catalog",
] as const
export type ShortcutId = (typeof SHORTCUT_IDS)[number]

/** The row under the composer is three wide. */
export const MAX_SHORTCUTS = 3

export const DEFAULT_CARDS: readonly CardId[] = CARD_IDS.slice(0, 3)
export const DEFAULT_SHORTCUTS: readonly ShortcutId[] = SHORTCUT_IDS.slice(0, MAX_SHORTCUTS)

/** The card that takes the wide column; every other card is a list. */
export const WIDE_CARD: CardId = "dashboard-preview"

/** What is saved for a user: the shape of `HomeLayout`, ids still untrusted. */
export type SavedLayout = {
  cards?: readonly string[]
  shortcuts?: readonly string[]
  previewBoardId?: string | null
}

/** A layout in terms this build knows: every id is in the catalogue, once. */
export type ResolvedLayout = {
  cards: CardId[]
  shortcuts: ShortcutId[]
  previewBoardId: string | null
}

function known<T extends string>(
  ids: readonly string[] | undefined,
  catalogue: readonly T[],
  fallback: readonly T[],
  max = Infinity,
): T[] {
  // A missing list (not an empty one) is a layout from a client that did
  // not set it: the default is the honest reading. An empty list is a
  // person choosing to show none and stays empty.
  if (!Array.isArray(ids)) return [...fallback]
  const out: T[] = []
  for (const id of ids) {
    const match = catalogue.find((c) => c === id)
    if (match !== undefined && !out.includes(match)) out.push(match)
    if (out.length >= max) break
  }
  return out
}

/**
 * A saved layout as this build can show it: unknown ids and repeats
 * dropped, shortcuts capped at three, and the defaults when nothing was
 * saved (`null`). An explicitly empty list stays empty.
 */
export function resolveLayout(saved: SavedLayout | null | undefined): ResolvedLayout {
  if (!saved) {
    return { cards: [...DEFAULT_CARDS], shortcuts: [...DEFAULT_SHORTCUTS], previewBoardId: null }
  }
  return {
    cards: known(saved.cards, CARD_IDS, DEFAULT_CARDS),
    shortcuts: known(saved.shortcuts, SHORTCUT_IDS, DEFAULT_SHORTCUTS, MAX_SHORTCUTS),
    previewBoardId:
      typeof saved.previewBoardId === "string" && saved.previewBoardId !== ""
        ? saved.previewBoardId
        : null,
  }
}

/** Catalogue entries not in `shown`, in catalogue order: what "Add" offers. */
export function hiddenIds<T extends string>(catalogue: readonly T[], shown: readonly T[]): T[] {
  return catalogue.filter((id) => !shown.includes(id))
}

/** `ids` without `id`. */
export function removeId<T extends string>(ids: readonly T[], id: T): T[] {
  return ids.filter((x) => x !== id)
}

/** `ids` with `id` last, unless it is already there or `max` is reached. */
export function appendId<T extends string>(ids: readonly T[], id: T, max = Infinity): T[] {
  if (ids.includes(id) || ids.length >= max) return [...ids]
  return [...ids, id]
}

/**
 * Put the cards other than `pinned` into `ordered`, leaving `pinned` (the
 * wide card, which the layout rule fixes to its own column and so is not
 * dragged) at the index it had. Returns `cards` unchanged when `ordered` is
 * not a permutation of the other cards, so a stale drag cannot drop or
 * invent a card.
 */
export function reorderAround<T extends string>(
  cards: readonly T[],
  ordered: readonly T[],
  pinned: T,
): T[] {
  const others = cards.filter((c) => c !== pinned)
  const samePool =
    ordered.length === others.length &&
    others.every((c) => ordered.includes(c)) &&
    new Set(ordered).size === ordered.length
  if (!samePool) return [...cards]
  let next = 0
  return cards.map((c) => (c === pinned ? c : (ordered[next++] as T)))
}

/** Whether two layouts show the same things in the same order. */
export function sameLayout(a: ResolvedLayout, b: ResolvedLayout): boolean {
  const eq = (x: readonly string[], y: readonly string[]) =>
    x.length === y.length && x.every((v, i) => v === y[i])
  return (
    eq(a.cards, b.cards) && eq(a.shortcuts, b.shortcuts) && a.previewBoardId === b.previewBoardId
  )
}
