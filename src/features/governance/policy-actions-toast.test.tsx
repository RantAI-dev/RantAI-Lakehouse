// WEB-6: the Enforce / Stop / Delete buttons on the asset's Access tab are
// `PolicyActions`. Its failure toast used to read `run.error` from the render's
// closure right after `await run.run(...)`, so the first failure showed no
// sentence at all and a later one showed the earlier attempt's error.
// `run.getError()` is the error of the invocation just awaited.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"
import * as auth from "@/features/auth/auth-provider"
import { governanceService } from "@/services"
import { ServiceError } from "@/services/errors"
import { PolicyActions } from "./policy-actions"

afterEach(() => {
  cleanup()
  mock.restore()
})

const policy = { id: "p1", name: "mask_email", status: "draft", enforceable: true }
const withWrite = () => {
  spyOn(auth, "useAuth").mockImplementation(
    () => ({ hasPermission: (p: string) => p === "policy:write" }) as unknown as ReturnType<typeof auth.useAuth>
  )
}

async function confirmEnforce() {
  fireEvent.click(screen.getByRole("button", { name: "Enforce" }))
  const dialog = await screen.findByRole("dialog")
  fireEvent.click(within(dialog).getByRole("button", { name: "Enforce" }))
}

const descriptionOf = (call: unknown[]) => (call[1] as { description?: string })?.description

describe("Access tab policy button failure toasts (WEB-6)", () => {
  it("shows the API's sentence on the very first failed attempt", async () => {
    withWrite()
    const errors = spyOn(toast, "error").mockImplementation(() => 1)
    spyOn(toast, "success").mockImplementation(() => 1)
    spyOn(governanceService, "setPolicyStatus").mockRejectedValue(
      new ServiceError("permission_denied", "Only a workspace admin may enforce a policy.", 403)
    )
    render(<PolicyActions policy={policy} onChanged={() => {}} />)

    await confirmEnforce()
    await waitFor(() => expect(errors).toHaveBeenCalledTimes(1))

    expect(errors.mock.calls[0]?.[0]).toBe("Enforce policy failed")
    expect(descriptionOf(errors.mock.calls[0] ?? [])).toContain("Only a workspace admin may enforce a policy.")
  })

  it("shows error B on a second failure, not stale error A", async () => {
    withWrite()
    const errors = spyOn(toast, "error").mockImplementation(() => 1)
    spyOn(toast, "success").mockImplementation(() => 1)
    const setStatus = spyOn(governanceService, "setPolicyStatus")
    setStatus.mockRejectedValueOnce(new ServiceError("invalid_request", "Error A: the policy name is taken."))
    setStatus.mockRejectedValueOnce(new ServiceError("invalid_request", "Error B: the policy is locked."))
    render(<PolicyActions policy={policy} onChanged={() => {}} />)

    await confirmEnforce()
    await waitFor(() => expect(errors).toHaveBeenCalledTimes(1))
    expect(descriptionOf(errors.mock.calls[0] ?? [])).toContain("Error A: the policy name is taken.")

    await confirmEnforce()
    await waitFor(() => expect(errors).toHaveBeenCalledTimes(2))
    expect(errors.mock.calls[1]?.[0]).toBe("Enforce policy failed")
    expect(descriptionOf(errors.mock.calls[1] ?? [])).toContain("Error B: the policy is locked.")
    expect(descriptionOf(errors.mock.calls[1] ?? [])).not.toContain("Error A")
  })
})
