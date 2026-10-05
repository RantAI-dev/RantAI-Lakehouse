"use client"

import * as React from "react"

import { useService, useServiceAction } from "@/hooks/use-service"
import {
  appendId,
  MAX_SHORTCUTS,
  removeId,
  reorderAround,
  resolveLayout,
  sameLayout,
  WIDE_CARD,
  type CardId,
  type ResolvedLayout,
  type ShortcutId,
} from "@/lib/home-layout"
import { homeService } from "@/services"
import type { HomeLayoutResponse } from "@/services/contracts/home"

/**
 * Where the saved layout is:
 * - `loading`: not known yet. Home draws nothing that depends on it, so
 *   there is no flash of the default followed by a rearrangement.
 * - `ready`: loaded and the deployment stores layouts; Customize is offered.
 * - `unavailable`: the read failed or the deployment cannot store layouts
 *   (`supported: false`). Home shows the default and offers no Customize,
 *   since a save could not persist.
 */
export type LayoutPhase = "loading" | "ready" | "unavailable"

/**
 * Home's layout: the saved one, resolved against this build's catalogue,
 * and the draft being edited.
 *
 * The draft lives only in memory until "Done"; nothing is kept in the
 * browser, so a layout is the same on every device or it is the default.
 * After a save or a reset the response itself becomes the current layout,
 * not a re-read, so the page does not blank and reload under the person.
 */
export function useHomeLayout() {
  const read = useService((signal) => homeService.getLayout(signal), [])
  const [latest, setLatest] = React.useState<HomeLayoutResponse | null>(null)
  const [draft, setDraft] = React.useState<ResolvedLayout | null>(null)
  const [notice, setNotice] = React.useState<string | null>(null)

  const save = useServiceAction((signal, layout: ResolvedLayout) =>
    homeService.saveLayout(layout, signal),
  )
  const reset = useServiceAction((signal) => homeService.resetLayout(signal))

  const response = latest ?? read.data
  const phase: LayoutPhase =
    response === null
      ? read.status === "error"
        ? "unavailable"
        : "loading"
      : response.supported
        ? "ready"
        : "unavailable"

  const saved = response?.supported ? response.layout : null
  const current = React.useMemo(() => resolveLayout(saved), [saved])
  const editing = draft !== null
  const shown = draft ?? current
  const busy = save.status === "pending" || reset.status === "pending"
  const error =
    notice ??
    (save.status === "error" ? save.error.message : null) ??
    (reset.status === "error" ? reset.error.message : null)

  const clearErrors = () => {
    setNotice(null)
    save.reset()
    reset.reset()
  }

  const edit = (fn: (d: ResolvedLayout) => ResolvedLayout) =>
    setDraft((d) => (d ? fn(d) : d))

  return {
    phase,
    shown,
    editing,
    busy,
    error,
    /** Whether the draft differs from what is saved. */
    changed: draft !== null && !sameLayout(draft, current),

    startEditing() {
      clearErrors()
      setDraft(current)
    },
    cancel() {
      clearErrors()
      setDraft(null)
    },
    async done() {
      if (!draft) return
      clearErrors()
      // Nothing changed: leave without a request, so a person who only
      // looked does not turn "no saved layout" into a saved copy of the
      // default that would then miss future catalogue changes.
      if (sameLayout(draft, current)) {
        setDraft(null)
        return
      }
      const out = await save.run(draft)
      if (!out) return
      if (!out.supported) {
        setNotice("This deployment can't store a Home layout.")
        return
      }
      setLatest(out)
      setDraft(null)
    },
    async resetToDefault() {
      clearErrors()
      const out = await reset.run()
      if (!out) return
      if (!out.supported) {
        setNotice("This deployment can't store a Home layout.")
        return
      }
      setLatest(out)
      setDraft(null)
    },

    hideCard: (id: CardId) => edit((d) => ({ ...d, cards: removeId(d.cards, id) })),
    addCard: (id: CardId) => edit((d) => ({ ...d, cards: appendId(d.cards, id) })),
    /** `others` is the new order of every card but the wide one. */
    reorderCards: (others: CardId[]) =>
      edit((d) => ({ ...d, cards: reorderAround(d.cards, others, WIDE_CARD) })),
    hideShortcut: (id: ShortcutId) =>
      edit((d) => ({ ...d, shortcuts: removeId(d.shortcuts, id) })),
    addShortcut: (id: ShortcutId) =>
      edit((d) => ({ ...d, shortcuts: appendId(d.shortcuts, id, MAX_SHORTCUTS) })),
    reorderShortcuts: (next: ShortcutId[]) =>
      edit((d) =>
        next.length === d.shortcuts.length && next.every((s) => d.shortcuts.includes(s))
          ? { ...d, shortcuts: next }
          : d,
      ),
    setPreviewBoard: (id: string | null) => edit((d) => ({ ...d, previewBoardId: id })),
  }
}
