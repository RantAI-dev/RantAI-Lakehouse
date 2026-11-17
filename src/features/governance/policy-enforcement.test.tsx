// SEC-61: the engine enforces only a structured `conditions` clause, so a
// policy without one masks nothing and filters no rows. The list must not
// call it enforced, the drawer must say what it does not do, and Enforce
// must not invite a change that changes nothing.
//
// `PolicyActions` renders nothing without `policy:write`, so the auth hook
// is stubbed before the imports resolve (`mock.module` is hoisted).
import { cleanup, render, screen } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"

import type { EntityStatus } from "@/lib/status"
import * as realAuth from "@/features/auth/auth-provider"

const real = { ...realAuth }
mock.module("@/features/auth/auth-provider", () => ({
  ...real,
  useAuth: () => ({ hasPermission: (p: string) => p === "policy:write" }),
}))
afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

import {
  POLICY_NOT_ENFORCEABLE,
  PolicyActions,
  PolicyNotEnforceableNotice,
} from "./policy-actions"
import { PolicyStatusPill } from "./policy-columns"

afterEach(cleanup)

const policy = (status: EntityStatus, enforceable: boolean) => ({
  id: "p1",
  name: "mask_email",
  status,
  enforceable,
})

describe("SEC-61: the status pill", () => {
  it("calls a ready policy the engine cannot enforce Not enforced", () => {
    render(<PolicyStatusPill policy={policy("ready", false)} />)
    const pill = screen.getByText("Not enforced")
    expect(pill.getAttribute("title")).toBe(POLICY_NOT_ENFORCEABLE)
    // The lifecycle label ("Ready" in ENTITY_STATUS_LABEL) must not appear.
    expect(screen.queryByText("Ready")).toBeNull()
  })

  it("still shows a draft as a draft, which is already accurate", () => {
    render(<PolicyStatusPill policy={policy("draft", false)} />)
    expect(screen.getByText("Draft")).toBeTruthy()
    expect(screen.queryByText("Not enforced")).toBeNull()
  })

  it("keeps the lifecycle label for a ready policy the engine can enforce", () => {
    render(<PolicyStatusPill policy={policy("ready", true)} />)
    expect(screen.getByText("Ready")).toBeTruthy()
    expect(screen.queryByText("Not enforced")).toBeNull()
  })
})

describe("SEC-61: the drawer", () => {
  it("says the policy masks nothing and links to where a rule is enforced", () => {
    render(<PolicyNotEnforceableNotice />)
    expect(
      screen.getByText(
        /This policy is written as text, so it masks nothing and filters no rows/
      )
    ).toBeTruthy()
    expect(
      screen.getByRole("link", { name: "Open the catalog" }).getAttribute("href")
    ).toBe("/catalog")
  })

  it("disables Enforce with the reason and keeps Delete", () => {
    render(<PolicyActions policy={policy("draft", false)} onChanged={() => {}} />)
    const enforce = screen.getByRole("button", { name: "Enforce" }) as HTMLButtonElement
    expect(enforce.disabled).toBe(true)
    expect(enforce.title).toBe(POLICY_NOT_ENFORCEABLE)
    const remove = screen.getByRole("button", {
      name: "Delete policy mask_email",
    }) as HTMLButtonElement
    expect(remove.disabled).toBe(false)
  })

  it("disables Stop enforcing for a ready policy that enforces nothing", () => {
    render(<PolicyActions policy={policy("ready", false)} onChanged={() => {}} />)
    const stop = screen.getByRole("button", {
      name: "Stop enforcing",
    }) as HTMLButtonElement
    expect(stop.disabled).toBe(true)
    expect(stop.title).toBe(POLICY_NOT_ENFORCEABLE)
  })

  it("leaves Enforce available for a draft the engine can enforce", () => {
    render(<PolicyActions policy={policy("draft", true)} onChanged={() => {}} />)
    expect(
      (screen.getByRole("button", { name: "Enforce" }) as HTMLButtonElement).disabled
    ).toBe(false)
  })
})
