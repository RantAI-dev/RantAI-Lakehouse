import { strict as assert } from "node:assert"
import { test } from "node:test"
import { isPublicPath } from "./auth-provider"

// WEB-5 D11: the landing page moved to its own site (`site/`); the console
// has no `/landing` route, so the path is no longer public.
test("isPublicPath no longer treats the moved landing page as public", () => {
  assert.equal(isPublicPath("/landing"), false)
  assert.equal(isPublicPath("/landing/"), false)
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
