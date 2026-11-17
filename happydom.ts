// Registers happy-dom's DOM implementation as globals (document, window,
// HTMLElement, ...) for every test process bun's preload list runs this
// in -- @testing-library/react needs a real DOM to render into, and bun
// itself ships none. Preloaded via bunfig.toml, not imported per-test-file,
// so every future test file gets a DOM automatically with no per-file
// boilerplate (judge amendment A2 -- WS3 is the first workstream to need
// this; WS7/WS9 reuse it rather than each registering their own).
import { GlobalRegistrator } from "@happy-dom/global-registrator"

GlobalRegistrator.register()

// PR #210 (motion 14.3 + happy-dom 21). motion-dom 14.3 stops its Web
// Animations fallback by calling animation.cancel(); the Web Animations spec's
// cancel procedure says to "reject the current finished promise with a
// DOMException named AbortError", and "reset an animation's pending tasks"
// then says to "set the current finished promise's [[PromiseIsHandled]] to
// true" -- so a browser never reports that rejection as unhandled. happy-dom
// 21.0.3 does the rejection without the handled flag
// (node_modules/happy-dom/lib/animation/Animation.js, Animation#cancel), so
// bun's runner prints "# Unhandled error between tests" per canceled animation
// and `bun run test` exits 1 with zero failing tests. This only marks the
// promise handled before delegating -- the same rejection still reaches any
// real consumer of `finished`; no other error is touched, and there is no
// global unhandledRejection hook. Removable once happy-dom sets the flag.
const animationPrototype = globalThis.Animation?.prototype
if (animationPrototype) {
  const cancel = animationPrototype.cancel
  animationPrototype.cancel = function cancelAnimation(this: Animation) {
    this.finished.catch(() => {
      // [[PromiseIsHandled]] = true, per the spec: the AbortError rejection is
      // expected output of a cancel, not a test failure.
    })
    return cancel.call(this)
  }
}

