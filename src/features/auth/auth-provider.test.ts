import { strict as assert } from "node:assert"
import { test } from "node:test"
import { isPublicPath } from "./auth-provider"

test("isPublicPath lets the landing page render without a session", () => {
  assert.equal(isPublicPath("/landing"), true)
  assert.equal(isPublicPath("/landing/"), true)
})

test("isPublicPath does not treat a lookalike prefix as public", () => {
  assert.equal(isPublicPath("/landingpage"), false)
  assert.equal(isPublicPath("/"), false)
  assert.equal(isPublicPath("/tables"), false)
})

test("isPublicPath keeps the three existing public surfaces public", () => {
  for (const path of ["/login", "/public/dashboard/abc", "/embed/chart"]) {
    assert.equal(isPublicPath(path), true, path)
  }
})

test("isPublicPath lets the PDF print view render without a session (BI-1)", () => {
  assert.equal(isPublicPath("/render/dashboard/some.token"), true)
  assert.equal(isPublicPath("/renderer"), false)
})
