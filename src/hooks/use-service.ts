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
  deps: React.DependencyList = [],
  options: { keepDataOnReload?: boolean } = {}
): ServiceState<T> & { reload: () => void } {
  const [state, setState] = React.useState<ServiceState<T>>({
    status: "loading",
    data: null,
    error: null,
  })
  const [reloadKey, setReloadKey] = React.useState(0)
  const fetcherRef = React.useRef(fetcher)
  fetcherRef.current = fetcher

  const keepRef = React.useRef(options.keepDataOnReload ?? false)
  keepRef.current = options.keepDataOnReload ?? false
  const depsKeyRef = React.useRef<React.DependencyList | null>(null)

  React.useEffect(() => {
    const controller = new AbortController()
    let active = true
    // A polled page reloads on a timer: with `keepDataOnReload`, a reload
    // for the SAME deps keeps the last data on screen instead of flashing
    // a skeleton every tick. New deps still show loading, since the old
    // data belongs to something else.
    const sameDeps =
      depsKeyRef.current !== null &&
      depsKeyRef.current.length === deps.length &&
      depsKeyRef.current.every((d, i) => Object.is(d, deps[i]))
    depsKeyRef.current = deps
    if (!(keepRef.current && sameDeps)) {
      setState({ status: "loading", data: null, error: null })
    }
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
 *
 * `state.error` is for rendering: it is the value of the render that created
 * the caller's closure, so a caller that reads it on the line after
 * `await run(...)` gets `null` (or the previous attempt's error), not the
 * failure it just awaited. `getError()` is for that call site: it reads the
 * latest invocation's error from a ref that `run` writes before it resolves.
 */
export function useServiceAction<Args extends unknown[], T>(
  action: (signal: AbortSignal, ...args: Args) => Promise<T>
): ActionState<T> & {
  run: (...args: Args) => Promise<T | null>
  reset: () => void
  cancel: () => void
  getError: () => ServiceError | null
} {
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
  // WEB-6: `setState` commits on the next render, so `state.error` on the line
  // after `await run(...)` is still the old closure's value. This ref is
  // written inside `run` itself, before it resolves, for callers that must
  // show the error of the invocation they just awaited (a failure toast).
  const lastErrorRef = React.useRef<ServiceError | null>(null)

  React.useEffect(() => () => controllerRef.current?.abort(), [])

  const run = React.useCallback(async (...args: Args): Promise<T | null> => {
    controllerRef.current?.abort()
    const controller = new AbortController()
    controllerRef.current = controller
    lastErrorRef.current = null
    setState((prev) => ({ status: "pending", data: prev.data, error: null }))
    try {
      const data = await actionRef.current(controller.signal, ...args)
      if (!controller.signal.aborted) {
        setState({ status: "success", data, error: null })
      }
      return data
    } catch (err) {
      const serviceError = toServiceError(err)
      // An invocation that `run` just superseded can still reject late; it
      // must not overwrite the new invocation's error with an older one.
      if (controllerRef.current === controller) lastErrorRef.current = serviceError
      if (!controller.signal.aborted && serviceError.code !== "aborted") {
        setState({ status: "error", data: null, error: serviceError })
      }
      return null
    }
  }, [])

  const reset = React.useCallback(() => {
    controllerRef.current?.abort()
    lastErrorRef.current = null
    setState({ status: "idle", data: null, error: null })
  }, [])

  // Stop what is in flight but keep what is on screen — the user who
  // cancels a slow query still wants the last result they were reading.
  const cancel = React.useCallback(() => {
    controllerRef.current?.abort()
    setState((prev) =>
      prev.data === null
        ? { status: "idle", data: null, error: null }
        : { status: "success", data: prev.data, error: null }
    )
  }, [])

  // WEB-6: read the fresh error at the awaited call site, not the stale
  // `state.error` captured by that call site's render.
  const getError = React.useCallback((): ServiceError | null => lastErrorRef.current, [])

  return { ...state, run, reset, cancel, getError }
}
