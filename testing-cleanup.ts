// Unmount whatever each test rendered. @testing-library/react only does
// this automatically when the runner exposes a global `afterEach` at import
// time, which bun's does not, so every rendered tree stayed in the one
// shared `document`. A later file's `getByText`/`getByLabelText` then
// matched an earlier file's leftovers ("Found multiple elements"), and
// which tests failed depended on file order: CI and local runs disagreed.
//
// A separate preload, listed after `happydom.ts` in bunfig.toml, because
// ES imports are hoisted: importing @testing-library/react inside
// happydom.ts would load it before the DOM globals are registered.
import { afterEach } from "bun:test"
import { cleanup } from "@testing-library/react"

afterEach(() => {
  cleanup()
})
