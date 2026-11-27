/**
 * Runs `run` once the page is usable: after the `load` event, in the next
 * idle period, or after `timeoutMs` if the browser never goes idle (WEB-5
 * D5). The 3D scene waits on it, so its probe and its `three` chunk never
 * compete with the first paint, the fonts or hydration. Browsers without
 * `requestIdleCallback` (Safari) get a short timer instead.
 *
 * Returns a cancel function.
 */
export function afterPaint(run: () => void, timeoutMs = 2500): () => void {
  let cancelled = false;
  let idleId: number | null = null;
  let timerId: number | null = null;
  const fire = () => {
    if (!cancelled) run();
  };
  const schedule = () => {
    if (cancelled) return;
    // TypeScript's DOM types always declare requestIdleCallback; Safari lacks it.
    if (typeof window.requestIdleCallback === "function") idleId = window.requestIdleCallback(fire, { timeout: timeoutMs });
    else timerId = globalThis.setTimeout(fire, 200) as unknown as number;
  };
  if (document.readyState === "complete") schedule();
  else window.addEventListener("load", schedule, { once: true });
  return () => {
    cancelled = true;
    window.removeEventListener("load", schedule);
    if (idleId !== null) window.cancelIdleCallback(idleId);
    if (timerId !== null) globalThis.clearTimeout(timerId);
  };
}
