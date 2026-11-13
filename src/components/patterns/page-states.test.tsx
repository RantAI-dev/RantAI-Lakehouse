import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { ServiceError, deniedFor } from "@/services/errors"
import { ErrorState, PermissionState } from "./page-states"

afterEach(cleanup)

describe("ErrorState", () => {
  it("a refusal names the missing permission under \"You don't have access\", with no Retry", () => {
    render(<ErrorState error={deniedFor(403, { error: "permission_denied: connector:manage" })!} onRetry={() => {}} />)

    expect(screen.getByText("You don't have access")).toBeTruthy()
    expect(
      screen.getByText("This needs the connector:manage permission. Ask a workspace admin.")
    ).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull()
  })

  it("a refusal a client still calls an outage is drawn as a refusal", () => {
    render(<ErrorState error={new ServiceError("unavailable", "permission_denied: pipeline:read")} onRetry={() => {}} />)

    expect(screen.getByText("You don't have access")).toBeTruthy()
    expect(screen.getByText("This needs the pipeline:read permission. Ask a workspace admin.")).toBeTruthy()
    expect(screen.queryByText("Service unavailable")).toBeNull()
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull()
    expect(document.body.textContent ?? "").not.toMatch(/permission_denied/)
  })

  it("a refusal filed as a bad request is drawn as a refusal", () => {
    render(
      <ErrorState
        error={new ServiceError("invalid_request", "permission_denied: connector:manage", 403)}
        onRetry={() => {}}
      />
    )

    expect(screen.getByText("You don't have access")).toBeTruthy()
    expect(screen.getByText("This needs the connector:manage permission. Ask a workspace admin.")).toBeTruthy()
    expect(screen.queryByText("Request could not be processed")).toBeNull()
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull()
    expect(document.body.textContent ?? "").not.toMatch(/permission_denied/)
  })

  it("a refusal that names nothing keeps the general sentence", () => {
    render(<ErrorState error={new ServiceError("permission_denied", "Failed (403)")} />)

    expect(screen.getByText(/Your role does not permit viewing this resource/)).toBeTruthy()
  })

  it("an outage still says Service unavailable and offers Retry", () => {
    render(<ErrorState error={new ServiceError("unavailable", "The catalog did not answer.")} onRetry={() => {}} />)

    expect(screen.getByText("Service unavailable")).toBeTruthy()
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy()
  })
})

describe("PermissionState", () => {
  it("the page guard's state with no error is unchanged", () => {
    render(<PermissionState />)

    expect(screen.getByText("You don't have access")).toBeTruthy()
    expect(screen.getByText(/Your role does not permit viewing this resource/)).toBeTruthy()
  })
})
