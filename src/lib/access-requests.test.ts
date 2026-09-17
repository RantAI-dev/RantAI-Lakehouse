import { describe, expect, it } from "bun:test"
import { accessGrantState, isOwnAccessRequest, requestableAccessPermissions } from "./access-requests"

describe("accessGrantState", () => {
  const NOW = Date.parse("2026-09-18T00:00:00Z")

  it("reads a pending access request as pending, regardless of any expiresAt", () => {
    expect(accessGrantState("pending", undefined, NOW)).toBe("pending")
    expect(accessGrantState("pending", "2020-01-01T00:00:00Z", NOW)).toBe("pending")
  })

  it("reads a rejected access request as rejected", () => {
    expect(accessGrantState("rejected", undefined, NOW)).toBe("rejected")
  })

  it("reads an approved grant with no known expiry as active — honest absence, not a fabricated date", () => {
    // GET /api/agents/approvals never joins access_grant (WS7 item E1's
    // migration keeps the grant's own expiry off the approval_item row),
    // so a reloaded list has no expiry to show for an already-decided
    // request. That must render as "approved", never as "expired" —
    // there is no date to compare against.
    expect(accessGrantState("approved", undefined, NOW)).toBe("approved-active")
  })

  it("reads an approved grant whose expiry is in the future as active", () => {
    expect(accessGrantState("approved", "2026-10-01T00:00:00Z", NOW)).toBe("approved-active")
  })

  it("reads an approved grant whose expiry has passed as expired, never as still active", () => {
    expect(accessGrantState("approved", "2026-09-01T00:00:00Z", NOW)).toBe("approved-expired")
  })

  it("treats an expiry exactly at now as already expired (the grant's own `expires_at > now()` boundary)", () => {
    expect(accessGrantState("approved", "2026-09-18T00:00:00Z", NOW)).toBe("approved-expired")
  })
})

describe("isOwnAccessRequest", () => {
  it("is true only for a pending kind=access row requested by the given user", () => {
    expect(
      isOwnAccessRequest(
        { kind: "access", status: "pending", requestedByUserId: "u1" },
        "u1"
      )
    ).toBe(true)
  })

  it("is false for a different requester", () => {
    expect(
      isOwnAccessRequest(
        { kind: "access", status: "pending", requestedByUserId: "u1" },
        "u2"
      )
    ).toBe(false)
  })

  it("is false for a tool_call approval even if requestedByUserId somehow matched", () => {
    expect(
      isOwnAccessRequest(
        { kind: "tool_call", status: "pending", requestedByUserId: "u1" },
        "u1"
      )
    ).toBe(false)
  })

  it("is false once the request has already been decided — the self-approval rule only blocks a pending decision", () => {
    expect(
      isOwnAccessRequest(
        { kind: "access", status: "approved", requestedByUserId: "u1" },
        "u1"
      )
    ).toBe(false)
  })
})

describe("requestableAccessPermissions", () => {
  it("offers only permissions the principal does not already hold — never one already granted", () => {
    expect(requestableAccessPermissions(["catalog:write", "lineage:read"], (p) => p === "lineage:read")).toEqual([
      "catalog:write",
    ])
  })

  it("offers nothing once every candidate permission is already held", () => {
    expect(requestableAccessPermissions(["catalog:write"], () => true)).toEqual([])
  })
})
