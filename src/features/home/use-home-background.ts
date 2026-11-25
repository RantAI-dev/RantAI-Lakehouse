"use client"

import * as React from "react"

import {
  DEFAULT_HOME_BACKGROUND,
  HOME_BACKGROUND_ATTR,
  HOME_BACKGROUND_KEY,
  readHomeBackground,
  writeHomeBackground,
  type HomeBackground,
} from "@/lib/home-background"

/**
 * Home's Background choice (`WEB-17` D2), read from this browser before the
 * first paint of Home so an Off page never shows the band's colour.
 *
 * `useSyncExternalStore`: the server and the hydration pass see the default
 * (the console's markup is the same for everyone), and the stored choice
 * applies on the first client render after. Home itself renders only once
 * the session is known, after hydration, so in practice its first paint is
 * already the person's choice. A change in another tab follows through the
 * `storage` event. When storage refuses a write (private window, quota),
 * the choice is still held in memory for this page view.
 */
let memory: HomeBackground | null = null
const listeners = new Set<() => void>()

function storage(): Storage | undefined {
  try {
    return window.localStorage
  } catch {
    return undefined
  }
}

function subscribe(onChange: () => void): () => void {
  listeners.add(onChange)
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key === HOME_BACKGROUND_KEY) {
      memory = null
      mark(snapshot())
      onChange()
    }
  }
  window.addEventListener("storage", onStorage)
  return () => {
    listeners.delete(onChange)
    window.removeEventListener("storage", onStorage)
  }
}

/** Keep `<html data-home-bg>` (set before paint by the root layout's script) in step with the choice. */
function mark(value: HomeBackground): void {
  if (value === "off") document.documentElement.setAttribute(HOME_BACKGROUND_ATTR, "off")
  else document.documentElement.removeAttribute(HOME_BACKGROUND_ATTR)
}

function snapshot(): HomeBackground {
  return memory ?? readHomeBackground(storage())
}

export function setHomeBackground(value: HomeBackground): void {
  memory = writeHomeBackground(storage(), value) ? null : value
  mark(value)
  for (const l of listeners) l()
}

export function useHomeBackground(): [HomeBackground, (value: HomeBackground) => void] {
  const value = React.useSyncExternalStore(subscribe, snapshot, () => DEFAULT_HOME_BACKGROUND)
  return [value, setHomeBackground]
}

/** For tests: forget a choice held in memory. */
export function resetHomeBackgroundMemory(): void {
  memory = null
}
