/**
 * Where a chart's click leads when an editor chose a destination instead of
 * the drill menu (BI-18 part B). Pure, so the rules are tested here; the
 * dashboard page only navigates to what these return.
 *
 * The clicked value is always inserted with `encodeURIComponent`, so a value
 * with a space, `&`, `#`, `/` or non-ASCII text stays one value in the
 * address and cannot add a parameter or end the path.
 */

import { FILTER_PARAM, encodeFilters } from "@/lib/dashboard-filter-state"
import type { ChartClick } from "@/services/clients/bi-store"

/** Where a `{value}` goes in a URL template. */
export const URL_PLACEHOLDER = "{value}"

const URL_MAX_CHARS = 2048
/** Whitespace, control characters (C0 and C1) and backslashes: how a scheme or host gets hidden. */
const HIDDEN = /[\s\u0000-\u001f\u007f-\u009f\\]/u

/**
 * The URL rule of the server (`lakehouse_bi::click::validate_click_url`),
 * checked again here because the saved text can be changed outside this
 * console: `https://…`, `http://…`, or a path starting with a single `/`,
 * with `{value}` anywhere but the scheme and the host. This copy only ever
 * refuses more than the server does.
 */
export function isAllowedClickUrl(url: string): boolean {
  if (!url || [...url].length > URL_MAX_CHARS || HIDDEN.test(url)) return false
  if (url.startsWith("/")) return !url.startsWith("//")
  const scheme = /^https?:\/\//i.exec(url)
  if (!scheme) return false
  const rest = url.slice(scheme[0].length)
  const end = rest.search(/[/?#]/)
  const authority = end < 0 ? rest : rest.slice(0, end)
  return authority !== "" && !authority.includes("{")
}

/** Another dashboard, opened with one `in` filter on `column` (the `?f=` the page reads). */
export function dashboardDestination(board: string, column: string, value: string): string {
  const q = new URLSearchParams({ [FILTER_PARAM]: encodeFilters([{ column, values: [value] }]) })
  return `/dashboards/${encodeURIComponent(board)}?${q.toString()}`
}

/** A saved query, opened in Query Studio by its id. */
export function queryDestination(id: string): string {
  return `/query-studio?saved=${encodeURIComponent(id)}`
}

/**
 * The address a URL template leads to for `value`, and whether it leaves the
 * console (an external address opens in a new tab, a path navigates in
 * place). `null` for a template the rule refuses: it is never followed.
 */
export function urlDestination(template: string, value: string): { href: string; external: boolean } | null {
  if (!isAllowedClickUrl(template)) return null
  // split/join, not replace: a value containing `$&` must stay literal.
  const href = template.split(URL_PLACEHOLDER).join(encodeURIComponent(value))
  return { href, external: !href.startsWith("/") }
}

/**
 * What is still missing from, or wrong with, a click setting the editor is
 * filling in; null when it can be saved. The server checks the same things
 * again (`ClickAction::validate`); this only lets the form say it first.
 */
export function clickProblem(click: ChartClick | undefined): string | null {
  if (!click) return null
  switch (click.kind) {
    case "dashboard":
      if (!click.board) return "Choose the dashboard a click opens."
      return click.column ? null : "Choose the column the clicked value filters."
    case "query":
      return click.id ? null : "Choose the saved query a click opens."
    case "url":
      if (!click.url.trim()) return "Enter the URL a click opens."
      return isAllowedClickUrl(click.url.trim()) ? null : "Use an https:// or http:// address, or a path starting with /. The value cannot be part of the host."
  }
}
