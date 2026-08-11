"use client"

import * as React from "react"
import { toServiceError, type ServiceError } from "@/services/errors"

export type ServiceState<T> =
  | { status: "loading"; data: null; error: null }
  | { status: "success"; data: T; error: null }
  | { status: "error"; data: null; error: ServiceError }

/**
 * Client data-fetch hook for service adapters.
 *
 * Handles loading/success/error states, abort-on-unmount, and reloads.
 * `fetcher` must be stable across renders unless `deps` change (pass filter
 * values through `deps`). Aborted requests never surface as errors.
 */
export function useService<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  deps: React.DependencyList = []
): ServiceState<T> & { reload: () => void } {
  const [state, setState] = React.useState<ServiceState<T>>({
    status: "loading",
    data: null,
    error: null,
  })
  const [reloadKey, setReloadKey] = React.useState(0)
  const fetcherRef = React.useRef(fetcher)
  fetcherRef.current = fetcher

  React.useEffect(() => {
    const controller = new AbortController()
    let active = true
    setState({ status: "loading", data: null, error: null })
    fetcherRef
      .current(controller.signal)
      .then((data) => {
        if (active) setState({ status: "success", data, error: null })
      })
      .catch((err) => {
        const serviceError = toServiceError(err)
        if (active && serviceError.code !== "aborted") {
          setState({ status: "error", data: null, error: serviceError })
        }
      })
    return () => {
      active = false
      controller.abort()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, reloadKey])

  const reload = React.useCallback(() => setReloadKey((k) => k + 1), [])

  return { ...state, reload }
}

export type ActionState<T> =
  | { status: "idle"; data: null; error: null }
  | { status: "pending"; data: T | null; error: null }
  | { status: "success"; data: T; error: null }
  | { status: "error"; data: null; error: ServiceError }

/**
 * Imperative counterpart of `useService` for user-triggered operations
 * (run a query, acknowledge an alert, cancel a workload).
 *
 * Invoking `run` aborts any in-flight invocation, keeps the previous data
 * visible while pending, and resolves to the result (or `null` on failure so
 * callers can branch without try/catch). Aborted calls never surface as errors.
 */
export function useServiceAction<Args extends unknown[], T>(
  action: (signal: AbortSignal, ...args: Args) => Promise<T>
): ActionState<T> & { run: (...args: Args) => Promise<T | null>; reset: () => void } {
  const [state, setState] = React.useState<ActionState<T>>({
    status: "idle",
    data: null,
    error: null,
  })
  const actionRef = React.useRef(action)
  React.useEffect(() => {
    actionRef.current = action
  })
  const controllerRef = React.useRef<AbortController | null>(null)

  React.useEffect(() => () => controllerRef.current?.abort(), [])

  const run = React.useCallback(async (...args: Args): Promise<T | null> => {
    controllerRef.current?.abort()
    const controller = new AbortController()
    controllerRef.current = controller
    setState((prev) => ({ status: "pending", data: prev.data, error: null }))
    try {
      const data = await actionRef.current(controller.signal, ...args)
      if (!controller.signal.aborted) {
        setState({ status: "success", data, error: null })
      }
      return data
    } catch (err) {
      const serviceError = toServiceError(err)
      if (!controller.signal.aborted && serviceError.code !== "aborted") {
        setState({ status: "error", data: null, error: serviceError })
      }
      return null
    }
  }, [])

  const reset = React.useCallback(() => {
    controllerRef.current?.abort()
    setState({ status: "idle", data: null, error: null })
  }, [])

  return { ...state, run, reset }
}
