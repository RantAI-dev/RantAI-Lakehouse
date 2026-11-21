// WEB-7 S1 review round 1, S1: the shown-once password belongs to the user
// who created it and expires. These tests drive the store directly with an
// injected clock, so nothing sleeps and the two-minute window is exact.
import { afterEach, describe, expect, it } from "bun:test"
import {
  INVITE_PASSWORD_TTL_MS,
  rememberInvitePassword,
  takeInvitePassword,
} from "./invite-password"

// A deliberately low-entropy stand-in for a server-generated password, so
// the secret scanner does not read it as a credential (same reason as the
// page tests).
const VALUE = { password: "test-only-temporary-password", mustChangePassword: true }
const T0 = 1_700_000_000_000

afterEach(() => {
  // The store is module-level; taking with any id clears whatever is left.
  takeInvitePassword("no-such-user")
})

describe("invite password store (WEB-7 S1 review round 1, S1)", () => {
  it("hands the value to the same user once inside the window, then nothing", () => {
    rememberInvitePassword(VALUE, "u1", T0)
    expect(takeInvitePassword("u1", T0 + 1_000)).toEqual(VALUE)
    expect(takeInvitePassword("u1", T0 + 1_001)).toBeNull()
  })

  it("gives another user nothing, and the value is gone afterwards", () => {
    rememberInvitePassword(VALUE, "u1", T0)
    expect(takeInvitePassword("u2", T0 + 1_000)).toBeNull()
    // The mismatched read cleared it, so the user it belonged to cannot get
    // it back either.
    expect(takeInvitePassword("u1", T0 + 1_001)).toBeNull()
  })

  it("gives nothing for a value older than the window", () => {
    rememberInvitePassword(VALUE, "u1", T0)
    expect(takeInvitePassword("u1", T0 + INVITE_PASSWORD_TTL_MS + 1)).toBeNull()
  })

  it("keeps the value for exactly the two minutes the constant names", () => {
    rememberInvitePassword(VALUE, "u1", T0)
    expect(takeInvitePassword("u1", T0 + INVITE_PASSWORD_TTL_MS)).toEqual(VALUE)
  })
})
