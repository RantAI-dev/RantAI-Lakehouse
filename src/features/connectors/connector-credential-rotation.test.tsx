import { fireEvent, render, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorCredentialRotation } from "./connector-credential-rotation"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
})

function mockRotate(status: number, body: unknown) {
  global.fetch = mock(async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    })
  ) as unknown as typeof fetch
}

describe("ConnectorCredentialRotation", () => {
  /**
   * ADR 0002 Addendum 3 / `rotate_secret`'s probe-first contract: 422 means
   * the candidate could not be verified (unsupported type, or a failed
   * probe) and nothing was written — the old credential is still the one
   * in use. Told apart from other failures by `ServiceError.status`, not
   * by parsing the message.
   */
  it("shows the server's message and says the old credential is still in use on a 422", async () => {
    mockRotate(422, { error: "connector's type cannot be probed by this build" })
    const onRotated = mock(() => {})
    const { container } = render(
      <ConnectorCredentialRotation connectorId="conn-orders-abc" onRotated={onRotated} />
    )
    fireEvent.click(within(container).getByRole("button", { name: "Rotate credential" }))
    await waitFor(() =>
      expect(
        within(container).getByText(/connector's type cannot be probed by this build/)
      ).toBeDefined()
    )
    expect(within(container).getByText(/old credential is still in use/)).toBeDefined()
    expect(onRotated).not.toHaveBeenCalled()
  })

  /**
   * `swap_secret_ref`'s optimistic-concurrency check: 409 means the slot's
   * ref changed since this handler read it — someone else rotated first.
   */
  it("tells the user to reload and retry on a 409", async () => {
    mockRotate(409, { error: "the secret ref changed since it was read; reload and retry" })
    const onRotated = mock(() => {})
    const { container } = render(
      <ConnectorCredentialRotation connectorId="conn-orders-abc" onRotated={onRotated} />
    )
    fireEvent.click(within(container).getByRole("button", { name: "Rotate credential" }))
    await waitFor(() =>
      expect(within(container).getByText(/reload this connector and try again/i)).toBeDefined()
    )
    expect(onRotated).not.toHaveBeenCalled()
  })

  /**
   * 200: `{ rotated: true, slot }` only — no derived name to echo (see this
   * component's module doc comment for why). Success also triggers the
   * drawer's reload via `onRotated`.
   */
  it("shows success and calls the reload prop on a 200", async () => {
    mockRotate(200, { rotated: true, slot: "primary" })
    const onRotated = mock(() => {})
    const { container } = render(
      <ConnectorCredentialRotation connectorId="conn-orders-abc" onRotated={onRotated} />
    )
    fireEvent.click(within(container).getByRole("button", { name: "Rotate credential" }))
    await waitFor(() => expect(within(container).getByText(/^Rotated/)).toBeDefined())
    expect(onRotated).toHaveBeenCalledTimes(1)
  })
})
