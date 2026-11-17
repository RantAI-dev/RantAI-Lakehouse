"use client"

import * as React from "react"

import {
  DEFAULT_FORMAT_SETTINGS,
  readFormatSettings,
  writeFormatSettings,
  type FormatSettings,
} from "@/lib/sql-format-settings"

/**
 * The Format settings: the keyword case and the indent the Format button
 * uses, kept in this browser.
 *
 * The stored settings are read after mount, never during render:
 * `localStorage` does not exist on the server, and reading it in render
 * would make the first paint differ from the markup Next sent.
 */
export function useFormatSettings() {
  const [settings, setSettings] = React.useState<FormatSettings>(DEFAULT_FORMAT_SETTINGS)
  const [restored, setRestored] = React.useState(false)

  React.useEffect(() => {
    setSettings(readFormatSettings())
    setRestored(true)
  }, [])

  React.useEffect(() => {
    // Before the stored settings arrive, what is in state is the default,
    // and writing it would throw away what was stored.
    if (!restored) return
    writeFormatSettings(settings)
  }, [settings, restored])

  return { settings, setSettings }
}
