"use client";

import * as React from "react";
import { m, useTransform } from "framer-motion";
import { useLandingMode } from "@/components/effects/scene-mode";
import { useProgress } from "@/components/shared/progress-section";
import { cn } from "@/lib/utils";

/**
 * The hero's client leaves (WEB-5 review fix R5): what moves with the
 * scroll or with the scene. The copy, the backdrop's drawing and the cue's
 * text arrive as server-rendered `children`.
 */

/** The copy lifts, fades and blurs as the dive starts, only over the live lake. */
export function HeroCopyMotion({ children }: { children: React.ReactNode }) {
  const { mode } = useLandingMode();
  // Over the whole section (150 vh), the copy is gone by 0.28 (about 42 vh).
  const scrollYProgress = useProgress();
  const live = mode === "full";
  const copyOpacity = useTransform(scrollYProgress, [0, 0.2, 0.3], live ? [1, 0.7, 0] : [1, 1, 1]);
  const copyY = useTransform(scrollYProgress, [0, 0.3], live ? [0, -72] : [0, 0]);
  const copyBlur = useTransform(scrollYProgress, [0.08, 0.3], live ? ["blur(0px)", "blur(8px)"] : ["blur(0px)", "blur(0px)"]);
  return (
    <m.div
      className="max-w-[660px]"
      // WEB-5 D5: the h1 inside is the page's largest paint, so it gets
      // no opacity, travel or filter until the live lake runs; the
      // server HTML and the DOM hero carry no style here at all.
      style={live ? { opacity: copyOpacity, y: copyY, filter: copyBlur } : undefined}
      // Keyed on `live` for the same reason as the reduced-motion keys
      // (review fix R1): the scroll styles arrive after hydration.
      key={live ? "live" : "still"}
    >
      {children}
    </m.div>
  );
}

/** The scroll cue fades over the first few percent of the section. */
export function ScrollCueFade({ children }: { children: React.ReactNode }) {
  const cueOpacity = useTransform(useProgress(), [0, 0.07], [1, 0]);
  return <m.div style={{ opacity: cueOpacity }}>{children}</m.div>;
}

/** The DOM hero fades out once the lake's first frame is on screen. */
export function BackdropFade({ children }: { children: React.ReactNode }) {
  const { sceneReady } = useLandingMode();
  return (
    <div
      aria-hidden="true"
      className={cn("absolute inset-0 -z-10 transition-opacity duration-700", sceneReady && "opacity-0")}
    >
      {children}
    </div>
  );
}
