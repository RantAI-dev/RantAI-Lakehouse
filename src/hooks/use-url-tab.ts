"use client"

import * as React from "react"
import { usePathname, useSearchParams } from "next/navigation"
import { parseUrlTab, urlWithTab } from "@/lib/url-tab"

/**
 * The open tab of a page, kept in the query parameter `param` (`WEB-3`
 * S3-D5 to S3-D7). The same posture as the asset and connector pages'
 * `?tab=`: switching writes the URL with the History API (no server round
 * trip; Next keeps `useSearchParams` in step), and a link to another tab of
 * the same page, which changes only the URL while the page stays mounted,
 * is taken over during render.
 */
export function useUrlTab<T extends string>(
  param: string,
  values: readonly T[]
): readonly [T, (next: T) => void] {
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const urlTab = parseUrlTab(searchParams.get(param), values)
  const [tab, setTab] = React.useState<T>(urlTab)
  const [seenUrlTab, setSeenUrlTab] = React.useState(urlTab)
  if (urlTab !== seenUrlTab) {
    setSeenUrlTab(urlTab)
    setTab(urlTab)
  }

  const select = React.useCallback(
    (next: T) => {
      setTab(next)
      window.history.replaceState(null, "", urlWithTab(pathname, searchParams.toString(), param, next, values))
    },
    [pathname, searchParams, param, values]
  )
  return [tab, select] as const
}
