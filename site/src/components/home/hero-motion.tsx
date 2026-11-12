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
  // Over the live hero (220 vh, D14 S1): the copy holds while the camera
  // descends, fades as it nears the waterline (0.3, about 0.66 vh), and is
  // gone just before the surface closes overhead (0.55, about 1.2 vh).
  const scrollYProgress = useProgress();
  const live = mode === "full";
  const copyOpacity = useTransform(scrollYProgress, [0, 0.3, 0.55], live ? [1, 0.7, 0] : [1, 1, 1]);
  const copyY = useTransform(scrollYProgress, [0, 0.55], live ? [0, -72] : [0, 0]);
  const copyBlur = useTransform(scrollYProgress, [0.2, 0.55], live ? ["blur(0px)", "blur(8px)"] : ["blur(0px)", "blur(0px)"]);
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
  const cueOpacity = useTransform(useProgress(), [0, 0.05], [1, 0]);
  return <m.div style={{ opacity: cueOpacity }}>{children}</m.div>;
}

/**
 * The DOM hero fades out once the lake's first frame is on screen. `className`
 * places the faded layer: the whole stage by default, or the creature's own
 * slot (WEB-5 review fix R12, hero-section.tsx).
 */
export function BackdropFade({
  className = "absolute inset-0 -z-10",
  children,
  ...rest
}: {
  className?: string;
  children: React.ReactNode;
} & React.HTMLAttributes<HTMLDivElement>) {
  const { sceneReady } = useLandingMode();
  return (
    <div
      aria-hidden="true"
      className={cn(className, "transition-opacity duration-700", sceneReady && "opacity-0")}
      {...rest}
    >
      {children}
    </div>
  );
}
