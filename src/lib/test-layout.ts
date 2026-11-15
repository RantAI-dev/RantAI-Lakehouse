/**
 * Test-only layout stand-ins (happy-dom lays nothing out, so a test that
 * needs a width says what it is). Each stub returns its own undo, which puts
 * back exactly what was there: the property descriptor the prototype owned,
 * or no own property at all.
 *
 * Why not assign `Element.prototype.getBoundingClientRect` (CI full run,
 * 2026-11-15): bun's `spyOn(HTMLElement.prototype, "getBoundingClientRect")`
 * leaves an own copy of the original method on `HTMLElement.prototype` after
 * `mockRestore()` (seen with `copilot-dock.test.tsx`), and that copy shadows
 * any later stub on `Element.prototype` for every HTML element in the same
 * bun process. These stubs are set on both prototypes, so a leftover on
 * either one cannot hide them, and they leave nothing behind themselves.
 */

type Undo = () => void

function stubProperty(target: object, key: PropertyKey, descriptor: PropertyDescriptor): Undo {
  const own = Object.getOwnPropertyDescriptor(target, key)
  Object.defineProperty(target, key, { configurable: true, ...descriptor })
  return () => {
    if (own) Object.defineProperty(target, key, own)
    else Reflect.deleteProperty(target, key)
  }
}

const rect = (width: number): DOMRect =>
  ({ width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect

/** Every element's `getBoundingClientRect()` is `width(element)` wide. */
export function stubRectWidth(width: (el: Element) => number): Undo {
  const value = function getBoundingClientRect(this: Element) { return rect(width(this)) }
  const undo = [Element.prototype, HTMLElement.prototype].map((p) => stubProperty(p, "getBoundingClientRect", { value, writable: true }))
  return () => { for (const u of undo.reverse()) u() }
}

/** Every element's `clientWidth` is `width(element)`. */
export function stubClientWidth(width: (el: Element) => number): Undo {
  const get = function clientWidth(this: Element) { return width(this) }
  const undo = [Element.prototype, HTMLElement.prototype].map((p) => stubProperty(p, "clientWidth", { get }))
  return () => { for (const u of undo.reverse()) u() }
}

/** A `ResizeObserver` that never calls back, so the width a test set is the one the component keeps. */
export function stubResizeObserver(): Undo {
  return stubProperty(globalThis, "ResizeObserver", {
    value: class { observe() {} unobserve() {} disconnect() {} },
    writable: true,
  })
}
