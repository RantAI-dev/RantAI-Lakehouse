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
  React.useEffect(() => {
    document.documentElement.dataset.scene = mode;
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
