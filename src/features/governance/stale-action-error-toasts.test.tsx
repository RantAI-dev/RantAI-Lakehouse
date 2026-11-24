// WEB-6: the other two failure toasts that used to read a stale `action.error`
// right after `await action.run(...)` — removing a freshness target and
// deleting a quality rule. Both now read `getError()`, the error of the
// invocation just awaited, so the API's sentence reaches the toast.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"
import { AssetQuality } from "@/features/catalog/asset-quality"
import * as auth from "@/features/auth/auth-provider"
import { governanceService } from "@/services"
import { ServiceError } from "@/services/errors"
import type { AssetDetail } from "@/services/contracts/assets"
import { FreshnessTargetDialog } from "./freshness-target-dialog"

afterEach(() => {
  cleanup()
  mock.restore()
})

const descriptionOf = (call: unknown[]) => (call[1] as { description?: string })?.description
const refused = (sentence: string) => new ServiceError("invalid_request", sentence)

describe("FreshnessTargetDialog failure toast (WEB-6)", () => {
  it("shows the API's sentence on the first failed removal", async () => {
    const errors = spyOn(toast, "error").mockImplementation(() => 1)
    spyOn(toast, "success").mockImplementation(() => 1)
    spyOn(governanceService, "deleteDatasetSla").mockRejectedValue(refused("This SLA is used by a running job."))
    const onSaved = mock(() => {})

    render(
      <FreshnessTargetDialog
        table="silver.orders"
        label="silver.orders"
        currentMinutes={1440}
        onClose={() => {}}
        onSaved={onSaved}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Remove target" }))

    await waitFor(() => expect(errors).toHaveBeenCalledTimes(1))
    expect(errors.mock.calls[0]?.[0]).toBe("Failed to remove the freshness target")
    expect(descriptionOf(errors.mock.calls[0] ?? [])).toContain("This SLA is used by a running job.")
    expect(onSaved).not.toHaveBeenCalled()
  })
})

const asset = {
  id: "orders",
  name: "Orders",
  tableKey: "silver.orders",
  qualityChecks: [
    {
      id: "rule-1",
      name: "amount_complete",
      dimension: "completeness",
      status: null,
      lastRun: null,
      threshold: "amount not null",
      severity: "medium",
      origin: "rule",
    },
  ],
} as unknown as AssetDetail

describe("AssetQuality failure toast (WEB-6)", () => {
  it("shows the API's sentence on the first failed rule deletion", async () => {
    spyOn(auth, "useAuth").mockImplementation(
      () => ({ hasPermission: (p: string) => p === "governance:write" }) as unknown as ReturnType<typeof auth.useAuth>
    )
    const errors = spyOn(toast, "error").mockImplementation(() => 1)
    spyOn(toast, "success").mockImplementation(() => 1)
    spyOn(governanceService, "deleteQualityRule").mockRejectedValue(
      refused("Rule deletion is locked for this table.")
    )
    const onChanged = mock(() => {})

    render(<AssetQuality asset={asset} onChanged={onChanged} />)
    fireEvent.click(screen.getByRole("button", { name: "Delete rule amount_complete" }))
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Delete rule" }))

    await waitFor(() => expect(errors).toHaveBeenCalledTimes(1))
    expect(errors.mock.calls[0]?.[0]).toBe("The rule could not be deleted")
    expect(descriptionOf(errors.mock.calls[0] ?? [])).toContain("Rule deletion is locked for this table.")
    expect(onChanged).not.toHaveBeenCalled()
  })
})
