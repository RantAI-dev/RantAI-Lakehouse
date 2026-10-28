import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { PipelineScheduleField } from "./pipeline-schedule-field"

afterEach(cleanup)

describe("PipelineScheduleField with target lag", () => {
  it("renders the notice that schedule is not used while target lag is set", () => {
    render(<PipelineScheduleField value="0 * * * *" onChange={() => {}} hasTargetLag={true} />)
    expect(screen.getByText("This schedule is not used while a target lag is set.")).toBeTruthy()
  })

  it("does not render the target lag notice when hasTargetLag is false", () => {
    render(<PipelineScheduleField value="0 * * * *" onChange={() => {}} hasTargetLag={false} />)
    expect(screen.queryByText("This schedule is not used while a target lag is set.")).toBeNull()
  })
})
