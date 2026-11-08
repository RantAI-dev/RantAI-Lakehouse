"use client";

import * as React from "react";
import { getTod, subscribeTod, toggleTod, TOD_FADE_MS } from "@/lib/tod";
import { createLakeEngine, type EngineMode, type LakeEngine } from "./lake-engine";

/**
 * The canvas and the effect that owns the engine. Default export so
 * `next/dynamic` can load it lazily (`ssr: false`): `three` stays out of the
 * first paint, which is the DOM hero (plan §2.9).
 */
export default function LakeScene({
  mode,
  observe,
  onReady,
  onFail,
  onEngine,
}: {
  mode: EngineMode;
  /** The element whose visibility gates the render loop. */
  observe: React.RefObject<HTMLElement | null>;
  onReady: () => void;
  onFail: () => void;
  /** The lab (`/lab/hero/`) takes the engine to set its day look and framing; the home page passes nothing. */
  onEngine?: (engine: LakeEngine | null) => void;
}) {
  const hostRef = React.useRef<HTMLDivElement>(null);
  // Held in refs so a parent re-render never rebuilds the renderer.
  const onReadyRef = React.useRef(onReady);
  const onFailRef = React.useRef(onFail);
  const onEngineRef = React.useRef(onEngine);
  React.useEffect(() => {
    onReadyRef.current = onReady;
    onFailRef.current = onFail;
    onEngineRef.current = onEngine;
  });

  React.useEffect(() => {
    const host = hostRef.current;
    const target = observe.current;
    if (!host || !target) return;
    // A fresh canvas per effect run: React StrictMode runs effects twice in
    // development, and a canvas whose context `dispose()` already released
    // cannot hand out a new one.
    const canvas = document.createElement("canvas");
    canvas.className = "block h-full w-full";
    host.appendChild(canvas);
    let engine: LakeEngine;
    try {
      engine = createLakeEngine({
        canvas,
        mode,
        observe: target,
        onContextLost: () => onFailRef.current(),
        // The home page: the sun and the moon in the sky also change day and night.
        onSkyTap: onEngineRef.current ? undefined : toggleTod,
      });
    } catch {
      // No usable GPU, or the context could not be created: the DOM hero stays.
      canvas.remove();
      onFailRef.current();
      return;
    }
    // One frame later, so the first rendered frame is on screen before the
    // DOM fallback fades.
    onEngineRef.current?.(engine);
    // The home page's day and night (the lab drives its own through `onEngine`).
    let unsubscribe = () => {};
    if (!onEngineRef.current) {
      const show = (ms: number) => engine.setDay(getTod() === "day" ? 1 : 0, ms);
      show(0);
      unsubscribe = subscribeTod(() => show(TOD_FADE_MS));
    }
    const id = requestAnimationFrame(() => onReadyRef.current());
    return () => {
      onEngineRef.current?.(null);
      unsubscribe();
      cancelAnimationFrame(id);
      engine.dispose();
      canvas.remove();
    };
  }, [mode, observe]);

  return <div ref={hostRef} className="h-full w-full" aria-hidden="true" />;
}
