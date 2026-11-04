"use client"

import * as React from "react"

/**
 * "The caller's mentions were read somewhere — the bell should reload."
 * Opening a dashboard's comments marks its mentions read (`BI-38` D6), which
 * happens inside a feature; the navbar that shows the bell is in
 * `components/app-shell` and must not import a feature. Both ends meet here,
 * the same channel pattern as `features/dashboards/dashboard-events.ts`.
 */
const EVENT = "mentions:read"

export function notifyMentionsRead(): void {
  try {
    window.dispatchEvent(new Event(EVENT))
  } catch {
    /* not in a browser */
  }
}

export function useMentionsRead(onRead: () => void): void {
  React.useEffect(() => {
    window.addEventListener(EVENT, onRead)
    return () => window.removeEventListener(EVENT, onRead)
  }, [onRead])
}
