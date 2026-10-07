"use client"

import * as React from "react"
import { toServiceError, type ServiceError } from "@/services/errors"

/**
 * The latest value of `fetcher`, refetched on `refresh()` without clearing
 * what is on screen: `useService` resets to a loading state on every
 * reload, which would flash a polled list on every poll.
 */
export function useRefreshable<T>(fetcher: (signal: AbortSignal) => Promise<T>, key: string) {
  const [data, setData] = React.useState<T | null>(null)
  const [error, setError] = React.useState<ServiceError | null>(null)
  const fetcherRef = React.useRef(fetcher)
  // Declared before the fetching effect below, so it runs first.
  React.useEffect(() => {
    fetcherRef.current = fetcher
  })
  const controllerRef = React.useRef<AbortController | null>(null)

  const refresh = React.useCallback(() => {
    controllerRef.current?.abort()
    const controller = new AbortController()
    controllerRef.current = controller
    fetcherRef.current(controller.signal).then(
      (next) => {
        if (controller.signal.aborted) return
        setData(next)
        setError(null)
      },
      (err) => {
        const serviceError = toServiceError(err)
        if (!controller.signal.aborted && serviceError.code !== "aborted") setError(serviceError)
      }
    )
  }, [])

  React.useEffect(() => {
    refresh()
    return () => controllerRef.current?.abort()
  }, [key, refresh])

  return { data, error, refresh }
}
