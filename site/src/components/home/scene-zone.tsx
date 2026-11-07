"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { useLandingMode } from "@/components/effects/scene-mode";
import { QuietBoundary } from "@/components/shared/quiet-boundary";
import { cn } from "@/lib/utils";

/**
 * `three` loads on demand, after first paint: the visitor sees the DOM hero
 * immediately and the canvas fades in over it (plan §2.9).
 */
const LakeScene = dynamic(() => import("@/components/effects/scene/lake-scene"), { ssr: false });

/**
 * Hero, sources and depths share one canvas behind them. Everything the
 * visitor reads here is real DOM above the canvas, so with no canvas the
 * same sections show over a CSS gradient instead. The sections arrive as
 * `children`, so their text is rendered on the server (WEB-5 D5).
 */
export function SceneZone({ children }: { children: React.ReactNode }) {
  const zoneRef = React.useRef<HTMLDivElement>(null);
  const { mode, sceneReady, markReady, markFailed } = useLandingMode();
  const hasCanvas = mode === "full" || mode === "still";

  return (
    <div id="scene-zone" ref={zoneRef} className="relative">
      {/* Below the hero, with no live canvas behind it, the sections sit on a deep-water gradient. */}
      <div
        aria-hidden="true"
        className={cn(
          "absolute inset-x-0 top-[100svh] bottom-0 bg-[linear-gradient(180deg,#0a1f55_0%,#06143f_30%,#04102f_65%,#020514_100%)] transition-opacity duration-700",
          mode === "full" && sceneReady ? "opacity-0" : "opacity-100",
        )}
      />
      {hasCanvas && (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none z-0 transition-opacity duration-700",
            // Full: pinned behind the hero, sources and depths for as long as
            // the zone lasts (the negative margin gives its flow height back).
            // Still: one frame, in the hero only.
            mode === "full"
              ? "sticky top-0 -mb-[100lvh] h-[100lvh]"
              : "absolute inset-x-0 top-0 h-[100svh]",
            sceneReady ? "opacity-100" : "opacity-0",
          )}
        >
          {/* A scene chunk that fails to load falls back to the DOM hero, like a lost context (review fix R1). */}
          <QuietBoundary onError={markFailed}>
            <LakeScene
              mode={mode === "still" ? "still" : "full"}
              observe={zoneRef}
              onReady={markReady}
              onFail={markFailed}
            />
          </QuietBoundary>
        </div>
      )}
      <div className="relative z-10">{children}</div>
    </div>
  );
}
