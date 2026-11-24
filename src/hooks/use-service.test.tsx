// WEB-6: the error a caller reads on the line after `await run(...)` used to
// come from the render's stale `state.error` (null on the first failure, the
// previous attempt's error on the second). `getError()` reads a ref that
// `run` writes before it resolves, so a failure toast can use the sentence
// from the invocation it just awaited.
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { buildErrorDescription } from "@/lib/notify-message"
import { ServiceError } from "@/services/errors"
import { useServiceAction } from "./use-service"

afterEach(cleanup)

const refused = (message: string) => new ServiceError("permission_denied", message)
const invalid = (message: string) => new ServiceError("invalid_request", message)

describe("useServiceAction getError", () => {
  it("is null before the first run and after a success", async () => {
    const action = mock(async (): Promise<string> => "done")
    const hook = renderHook(() => useServiceAction(() => action()))

    expect(hook.result.current.getError()).toBeNull()
    await act(async () => {
      expect(await hook.result.current.run()).toBe("done")
    })
    expect(hook.result.current.getError()).toBeNull()
    expect(hook.result.current.status).toBe("success")
  })

  it("gives the first failure's API sentence at the awaited call site, where state.error is still stale", async () => {
    const action = mock(async (): Promise<string> => {
      throw invalid("Policy name already exists.")
    })
    const hook = renderHook(() => useServiceAction(() => action()))
    // The object captured by the handler's closure, as in a component that
    // does `const done = await action.run(...); toast.error(..., action.error)`.
    const closure = hook.result.current
    const seen: { outcome?: string | null; fresh?: ServiceError | null; stale?: ServiceError | null } = {}

    await act(async () => {
      seen.outcome = await closure.run()
      seen.fresh = closure.getError()
      seen.stale = closure.error
    })

    expect(seen.outcome).toBeNull()
    expect(seen.stale).toBeNull() // without the fix, the toast had no sentence
    expect(seen.fresh?.message).toBe("Policy name already exists.")
    // The same string `notifyError` puts in the toast description.
    expect(buildErrorDescription(seen.fresh ?? null)).toContain("Policy name already exists.")
  })

  it("gives the second failure's error B, not stale error A from the previous attempt", async () => {
    const action = mock(async (): Promise<string> => {
      throw invalid("First attempt was refused.")
    })
    const hook = renderHook(() => useServiceAction(() => action()))

    await act(async () => {
      await hook.result.current.run()
    })
    expect(hook.result.current.error?.message).toBe("First attempt was refused.")
    // The closure a second click would use already holds error A.
    const secondClosure = hook.result.current
    expect(secondClosure.error?.message).toBe("First attempt was refused.")

    action.mockImplementation(async () => {
      throw refused("Second attempt was denied for analyst:write.")
    })
    const seen: { fresh?: ServiceError | null; stale?: ServiceError | null } = {}
    await act(async () => {
      await secondClosure.run()
      seen.fresh = secondClosure.getError()
      seen.stale = secondClosure.error
    })

    expect(seen.stale?.message).toBe("First attempt was refused.") // the old bug
    expect(seen.fresh?.message).toBe("Second attempt was denied for analyst:write.")
    expect(buildErrorDescription(seen.fresh ?? null)).toContain(
      "Second attempt was denied for analyst:write."
    )
  })

  it("keeps the newest invocation's error when a superseded call rejects later", async () => {
    let rejectOld: ((err: unknown) => void) | null = null
    const action = mock((): Promise<string> => {
      if (action.mock.calls.length === 1) {
        return new Promise<string>((_resolve, reject) => {
          rejectOld = reject
        })
      }
      return Promise.reject(invalid("Newest attempt failed."))
    })
    const hook = renderHook(() => useServiceAction(() => action()))

    let first: Promise<string | null> = Promise.resolve(null)
    act(() => {
      first = hook.result.current.run()
    })
    await act(async () => {
      await hook.result.current.run()
    })
    expect(hook.result.current.getError()?.message).toBe("Newest attempt failed.")

    await act(async () => {
      rejectOld?.(invalid("Older attempt failed."))
      await first
    })
    // The late rejection of the aborted call must not replace error B.
    expect(hook.result.current.getError()?.message).toBe("Newest attempt failed.")
  })

  it("clears the error on a successful retry and on reset", async () => {
    const action = mock(async (): Promise<string> => {
      if (action.mock.calls.length === 1) throw invalid("Try again.")
      return "done"
    })
    const hook = renderHook(() => useServiceAction(() => action()))
    const closure = hook.result.current

    await act(async () => {
      await closure.run()
    })
    expect(closure.getError()?.message).toBe("Try again.")
    await act(async () => {
      await closure.run()
    })
    expect(closure.getError()).toBeNull()

    action.mockImplementation(async () => {
      throw invalid("Again.")
    })
    await act(async () => {
      await closure.run()
    })
    expect(closure.getError()?.message).toBe("Again.")
    act(() => hook.result.current.reset())
    expect(hook.result.current.getError()).toBeNull()
  })

  it("reports an aborted call's error so the toast layer can swallow it", async () => {
    const action = mock(async (): Promise<string> => {
      throw new ServiceError("aborted", "The request was cancelled.")
    })
    const hook = renderHook(() => useServiceAction(() => action()))
    const closure = hook.result.current

    await act(async () => {
      await closure.run()
    })
    expect(closure.getError()?.code).toBe("aborted")
    expect(buildErrorDescription(closure.getError())).toBeNull()
  })
})
