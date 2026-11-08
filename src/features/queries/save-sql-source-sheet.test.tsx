// A dashboard's SQL source cannot hold a `{{name}}` placeholder yet (`QS-5`,
// decision 15): the server refuses it, and the sheet shows the server's
// sentence as it does for every other refusal. The console does not check
// the SQL itself.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { dashboardService } from "@/services"
import { ServiceError } from "@/services/errors"
import { SaveSqlSourceSheet } from "./save-sql-source-sheet"

afterEach(() => {
  cleanup()
  mock.restore()
})

const REFUSAL = "Parameters in a dashboard's SQL source come later: give the value in the SQL for now."

describe("SaveSqlSourceSheet", () => {
  it("sends the SQL with its placeholder as it is, and shows the sentence the server refuses it with", async () => {
    spyOn(globalThis, "fetch").mockResolvedValue(new Response("{}", { status: 404 }))
    const create = spyOn(dashboardService, "createSqlSource").mockRejectedValue(
      new ServiceError("invalid_request", REFUSAL)
    )
    render(<SaveSqlSourceSheet open onOpenChange={() => {}} sql="SELECT * FROM serving.t WHERE r = {{region}}" />)

    fireEvent.click(screen.getByRole("button", { name: "Save source" }))

    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    expect(create.mock.calls[0]?.[0].sql).toBe("SELECT * FROM serving.t WHERE r = {{region}}")
    expect((await screen.findByText(REFUSAL)).textContent).toBe(REFUSAL)
  })
})
