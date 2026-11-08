"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { useLandingMode } from "@/components/effects/scene-mode";
import type { LakeEngine } from "@/components/effects/scene/lake-engine";
import { QuietBoundary } from "@/components/shared/quiet-boundary";
import { cn } from "@/lib/utils";

const LakeScene = dynamic(() => import("@/components/effects/scene/lake-scene"), { ssr: false });

/**
 * The real lake behind the lab's hero: the same scene chunk and the same mode
 * choice as the home page (`scene-mode.tsx`), so a device without fast WebGL
 * gets the DOM picture, here too. Hands the engine up so the lab can set its
 * day look and where the creature stands.
 */
export function LabScene({
  observe,
  onEngine,
}: {
  observe: React.RefObject<HTMLElement | null>;
  onEngine: (engine: LakeEngine | null) => void;
}) {
  const { mode, sceneReady, markReady, markFailed } = useLandingMode();
  if (mode !== "full" && mode !== "still") return null;
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-0 z-0 transition-opacity duration-400",
        sceneReady ? "opacity-100" : "opacity-0",
      )}
    >
      <QuietBoundary onError={markFailed}>
        <LakeScene mode={mode} observe={observe} onReady={markReady} onFail={markFailed} onEngine={onEngine} />
      </QuietBoundary>
    </div>
  );
}
