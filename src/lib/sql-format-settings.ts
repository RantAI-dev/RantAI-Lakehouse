/**
 * The Query Studio format settings: the keyword case and the indent the
 * Format button uses.
 *
 * Kept in `localStorage` rather than on the server — the settings belong
 * to the browser the query was written in, like the draft — and read
 * through a parser that never trusts what it finds there, since a stale
 * or hand-edited entry must not break the page.
 */

import type { KeywordCase } from "./sql-format"

export const SQL_FORMAT_SETTINGS_KEY = "rantai-lakehouse:query-studio-format-settings"

export type FormatSettings = {
  /** How the formatter writes the keywords it recognises. */
  keywordCase: KeywordCase
  /** How far the body of a clause is indented, in spaces. */
  indent: 0 | 2 | 4
}

/** What the Format button does when nothing has been chosen yet. */
export const DEFAULT_FORMAT_SETTINGS: FormatSettings = { keywordCase: "upper", indent: 0 }

function isKeywordCase(value: unknown): value is KeywordCase {
  return value === "upper" || value === "lower" || value === "preserve"
}

function isIndent(value: unknown): value is 0 | 2 | 4 {
  return value === 0 || value === 2 || value === 4
}

/**
 * The settings in `raw`, or the default for every field that is missing
 * or holds something this formatter does not know.
 */
export function parseFormatSettings(raw: string | null): FormatSettings {
  if (!raw) return { ...DEFAULT_FORMAT_SETTINGS }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return { ...DEFAULT_FORMAT_SETTINGS }
  }
  if (typeof parsed !== "object" || parsed === null) return { ...DEFAULT_FORMAT_SETTINGS }
  const stored = parsed as Partial<FormatSettings>
  return {
    keywordCase: isKeywordCase(stored.keywordCase)
      ? stored.keywordCase
      : DEFAULT_FORMAT_SETTINGS.keywordCase,
    indent: isIndent(stored.indent) ? stored.indent : DEFAULT_FORMAT_SETTINGS.indent,
  }
}

/**
 * Read the settings. Every access is guarded: storage throws in a private
 * window and is simply absent during server rendering.
 */
export function readFormatSettings(): FormatSettings {
  try {
    return parseFormatSettings(window.localStorage.getItem(SQL_FORMAT_SETTINGS_KEY))
  } catch {
    return { ...DEFAULT_FORMAT_SETTINGS }
  }
}

/** Store the settings. A setting that cannot be saved is a lost convenience. */
export function writeFormatSettings(settings: FormatSettings): void {
  try {
    window.localStorage.setItem(SQL_FORMAT_SETTINGS_KEY, JSON.stringify(settings))
  } catch {
    // Storage throws in a private window; the format button still works
    // with the settings of this session.
  }
}
