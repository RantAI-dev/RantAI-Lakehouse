import * as React from "react"

const STORAGE_KEY = "rantai:sidebar-groups"

/**
 * Which sidebar sections are expanded, remembered across visits.
 *
 * Two rules decide what a user sees:
 *
 * 1. The group holding the current page is ALWAYS open, whatever is
 *    stored. Otherwise navigating to a page could leave its own section
 *    collapsed, and the sidebar would show no trace of where you are.
 * 2. Everything else is the user's choice, and several groups may be open
 *    at once. Browsing one section should not close another.
 *
 * Persisted in `localStorage` (same approach as `use-table-layout`) rather
 * than a cookie: this is a per-browser preference the server never needs.
 *
 * The stored value is read in an effect, not during render, so the server
 * and first client render agree — reading storage inline would produce a
 * different tree on the client and trip a hydration mismatch.
 */
export function useSidebarGroups({
  activeLabel,
  defaultOpenLabels,
}: {
  /** Group containing the current page. Forced open, never collapsible. */
  activeLabel: string | undefined
  /** Groups expanded on a first visit, before anything is stored. */
  defaultOpenLabels: string[]
}) {
  const [stored, setStored] = React.useState<string[] | null>(null)

  React.useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      const parsed: unknown = raw ? JSON.parse(raw) : null
      setStored(
        Array.isArray(parsed) && parsed.every((v) => typeof v === "string")
          ? (parsed as string[])
          : // Corrupt or hand-edited value: fall back to the defaults
            // rather than throwing away navigation entirely.
            defaultOpenLabels
      )
    } catch {
      setStored(defaultOpenLabels)
    }
    // Defaults are only consulted on the very first read, so re-running
    // this when they change would clobber the user's stored choice.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const openLabels = React.useMemo(() => {
    // Before storage is read, show the defaults. They are also what a
    // first-time visitor gets, so nothing flashes open then shut.
    const base = new Set(stored ?? defaultOpenLabels)
    if (activeLabel) base.add(activeLabel)
    return base
  }, [stored, defaultOpenLabels, activeLabel])

  const toggle = React.useCallback(
    (label: string) => {
      setStored((current) => {
        const next = new Set(current ?? defaultOpenLabels)
        if (next.has(label)) next.delete(label)
        else next.add(label)

        const list = [...next]
        try {
          localStorage.setItem(STORAGE_KEY, JSON.stringify(list))
        } catch {
          // Private mode or a full quota — the sidebar still works for
          // this session, it just will not be remembered.
        }
        return list
      })
    },
    [defaultOpenLabels]
  )

  const isOpen = React.useCallback(
    (label: string) => openLabels.has(label),
    [openLabels]
  )

  return { isOpen, toggle }
}
