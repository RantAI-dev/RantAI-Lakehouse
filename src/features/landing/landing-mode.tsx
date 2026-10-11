"use client";

import * as React from "react";
import { hasWebGL2 } from "./scene/webgl-support";

/**
 * Which hero the visitor gets, decided once on the client.
 *
 * - `pending`: server render and the first client render. The DOM hero shows.
 * - `full`: WebGL works and motion is allowed: the live lake.
 * - `still`: WebGL works but `prefers-reduced-motion: reduce`: one rendered
 *   frame, no dive, no tracking (plan §2.8).
 * - `none`: no WebGL, or the context was lost: the DOM hero stays for good.
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

const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";

function subscribeReduced(onChange: () => void): () => void {
  const mq = window.matchMedia(REDUCED_QUERY);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

const noopSubscribe = () => () => {};

/** Probing creates a context, so it runs once and the answer is kept. */
let webglProbe: boolean | null = null;
function probeWebgl(): boolean {
  if (webglProbe === null) webglProbe = hasWebGL2();
  return webglProbe;
}

export function LandingModeProvider({ children }: { children: React.ReactNode }) {
  // Both reads are client-only; `useSyncExternalStore` gives the server and the
  // first hydration pass the neutral value and then switches, with no
  // mismatch and no setState-in-effect.
  const reduced = React.useSyncExternalStore(
    subscribeReduced,
    () => window.matchMedia(REDUCED_QUERY).matches,
    () => false,
  );
  const webgl = React.useSyncExternalStore<"unknown" | "yes" | "no">(
    noopSubscribe,
    () => (probeWebgl() ? "yes" : "no"),
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
