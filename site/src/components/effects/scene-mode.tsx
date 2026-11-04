"use client";

import * as React from "react";
import { afterPaint } from "@/lib/after-paint";
import { hasFastWebGL } from "./scene/webgl-support";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * Which hero the visitor gets, decided once on the client.
 *
 * - `pending`: server render, hydration, and until the page is usable
 *   (`afterPaint`, WEB-5 D5). The DOM hero shows.
 * - `full`: WebGL works and motion is allowed: the live lake.
 * - `still`: WebGL works but `prefers-reduced-motion: reduce`: one rendered
 *   frame, no dive, no tracking (plan §2.8).
 * - `none`: no fast WebGL (`hasFastWebGL`: no WebGL2, or a software GPU), or
 *   the context was lost: the DOM hero stays for good.
 *
 * Sections read this to pick their own fallback (for example the depth stack
 * becomes CSS when there is no canvas), so the page reads end to end in every
 * mode.
 */
export type SceneMode = "pending" | "full" | "still" | "none";

type LandingModeValue = {
  mode: SceneMode;
  /** The first frame of the canvas is on screen. */
  sceneReady: boolean;
  reducedMotion: boolean;
  markReady: () => void;
  markFailed: () => void;
};

const LandingModeContext = React.createContext<LandingModeValue>({
  mode: "pending",
  sceneReady: false,
  reducedMotion: false,
  markReady: () => {},
  markFailed: () => {},
});

/**
 * The WebGL answer, as a tiny external store: "unknown" until the page is
 * usable, then the probe's result. Probing creates a context, so it runs
 * once and the answer is kept; the scene's chunk is only requested once the
 * answer is "yes", so the `three` download starts after the first paint too.
 */
let webglAnswer: "unknown" | "yes" | "no" = "unknown";
const webglListeners = new Set<() => void>();
let webglScheduled = false;
function subscribeWebgl(onChange: () => void): () => void {
  webglListeners.add(onChange);
  if (!webglScheduled) {
    webglScheduled = true;
    afterPaint(() => {
      webglAnswer = hasFastWebGL() ? "yes" : "no";
      for (const l of webglListeners) l();
    });
  }
  return () => {
    webglListeners.delete(onChange);
  };
}

/**
 * WEB-5 review fix R11 (SHOULD-FIX): a deep link (`/#governance`) used to
 * land about a screen short. The browser scrolls to the hash smoothly
 * (`html` has `scroll-behavior: smooth`) while hydration is still growing
 * the dashboards section from one screen to its measured travel, and the
 * smooth scroll ends at its old target. It predates D14 (the R7 build does
 * the same).
 *
 * The fix lands on the hash once more after the page has settled, rather
 * than giving the dashboards their final height before hydration. That
 * height is measured from the tiles and the fonts (dashboards-scroller.tsx),
 * so a server-side copy would be a second formula to keep in step, and
 * without JavaScript it would add a screen of pinned scroll with a rail
 * that never slides. Landing once after the settle also covers the other
 * late change, the live hero's extra 70 vh (R10).
 *
 * "Settled" is the first mode other than `pending` (after `load` and an
 * idle period, `afterPaint`), once the fonts are in, one frame later. The
 * scroll happens only if the hash names a `section` and the visitor has not
 * scrolled, touched, pressed a key or clicked since hydration, so nobody
 * who has started moving is moved. It runs once per page load.
 */
const VISITOR_INPUT = ["wheel", "touchstart", "keydown", "pointerdown"] as const;
let visitorMoved = false;
let watchingVisitor = false;
let landedOnHash = false;
function watchVisitor(): void {
  if (watchingVisitor) return;
  watchingVisitor = true;
  const moved = () => {
    visitorMoved = true;
    for (const type of VISITOR_INPUT) window.removeEventListener(type, moved, true);
  };
  for (const type of VISITOR_INPUT) window.addEventListener(type, moved, { capture: true, passive: true });
}
function landOnHash(): void {
  if (landedOnHash) return;
  landedOnHash = true;
  const id = decodeURIComponent(window.location.hash.slice(1));
  if (!id) return;
  const target = document.getElementById(id);
  if (!target || target.tagName !== "SECTION") return;
  void document.fonts.ready.then(() =>
    requestAnimationFrame(() => {
      // `scroll-mt-*` on the section keeps it clear of the fixed top bar.
      if (!visitorMoved) target.scrollIntoView({ behavior: "instant", block: "start" });
    }),
  );
}

export function LandingModeProvider({ children }: { children: React.ReactNode }) {
  // Both reads are client-only; `useSyncExternalStore` gives the server and the
  // first hydration pass the neutral value and then switches, with no
  // mismatch and no setState-in-effect.
  const reduced = useReducedMotionSafe();
  const webgl = React.useSyncExternalStore<"unknown" | "yes" | "no">(
    subscribeWebgl,
    () => webglAnswer,
    () => "unknown",
  );
  const [failed, setFailed] = React.useState(false);
  // Ready is remembered per mode, so a new canvas (reduced-motion toggled
  // live) starts un-ready and the fallback fades only after its first frame.
  const [readyFor, setReadyFor] = React.useState<SceneMode | null>(null);

  const mode: SceneMode =
    webgl === "unknown" ? "pending" : webgl === "no" || failed ? "none" : reduced ? "still" : "full";

  const markReady = React.useCallback(() => setReadyFor(mode), [mode]);
  const markFailed = React.useCallback(() => setFailed(true), []);

  // WEB-5 D14 S1: the mode on <html>, for CSS that depends on it (the hero
  // is longer while the live lake runs, app/globals.css). Written when the
  // mode changes, which is at most a few times per visit, never per scroll.
  //
  // WEB-5 review fix R10 (SHOULD-FIX): `full` makes the hero 70 vh longer,
  // and it lands after the first paint, when a visitor who reloaded
  // mid-page or followed a deep link is already scrolled. Chrome's scroll
  // anchoring keeps the view in place; a browser without it would move the
  // page under the visitor. So the first section on screen is measured
  // before and after the write, and the page is scrolled by the difference.
  // Where the browser already anchored, the difference is 0 and this does
  // nothing. "instant", because `html` scrolls smoothly (app/globals.css).
  React.useEffect(() => {
    const root = document.documentElement;
    if (root.dataset.scene === mode) return;
    const anchor = Array.from(document.querySelectorAll<HTMLElement>("section[id]")).find(
      (s) => s.getBoundingClientRect().bottom > 0,
    );
    const before = anchor?.getBoundingClientRect().top;
    root.dataset.scene = mode;
    if (!anchor || before === undefined) return;
    const moved = anchor.getBoundingClientRect().top - before;
    if (Math.abs(moved) >= 1) window.scrollBy({ top: moved, behavior: "instant" });
  }, [mode]);

  // R11: watch for the visitor's own scrolling from hydration on, and land
  // on a deep link's section once the mode has settled (see `landOnHash`).
  React.useEffect(watchVisitor, []);
  React.useEffect(() => {
    if (mode !== "pending") landOnHash();
  }, [mode]);

  const value = React.useMemo<LandingModeValue>(
    () => ({
      mode,
      sceneReady: readyFor === mode && (mode === "full" || mode === "still"),
      reducedMotion: reduced,
      markReady,
      markFailed,
    }),
    [mode, readyFor, reduced, markReady, markFailed],
  );
  return <LandingModeContext.Provider value={value}>{children}</LandingModeContext.Provider>;
}

export function useLandingMode(): LandingModeValue {
  return React.useContext(LandingModeContext);
}
