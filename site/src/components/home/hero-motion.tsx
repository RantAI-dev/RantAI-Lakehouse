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

/**
 * The copy leaves as the dive starts, only over the live lake.
 *
 * WEB-5 O5: the owner found the old exit "muddy": an 8 px blur on the
 * 7 rem headline. Now it fades, rises a little and settles back a touch
 * smaller, with at most a 3 px blur at the very end, and it is gone at
 * 0.38 of the hero (0.84 vh of scroll), before the waterline enters the
 * screen from below (about 0.87 vh, `DIVE` and `LENS` in
 * lib/scroll-timeline.ts), so the water never passes over text.
 */
export function HeroCopyMotion({ children }: { children: React.ReactNode }) {
  const { mode } = useLandingMode();
  const scrollYProgress = useProgress();
  const live = mode === "full";
  const copyOpacity = useTransform(scrollYProgress, [0.04, 0.38], live ? [1, 0] : [1, 1]);
  const copyY = useTransform(scrollYProgress, [0, 0.38], live ? [0, -28] : [0, 0]);
  const copyScale = useTransform(scrollYProgress, [0, 0.38], live ? [1, 0.975] : [1, 1]);
  const copyBlur = useTransform(scrollYProgress, [0.16, 0.38], live ? ["blur(0px)", "blur(3px)"] : ["blur(0px)", "blur(0px)"]);
  return (
    <m.div
      className="max-w-[660px] origin-[50%_40%] md:origin-[0%_50%]"
      // WEB-5 D5: the h1 inside is the page's largest paint, so it gets
      // no opacity, travel or filter until the live lake runs; the
      // server HTML and the DOM hero carry no style here at all.
      style={live ? { opacity: copyOpacity, y: copyY, scale: copyScale, filter: copyBlur } : undefined}
      // Keyed on `live` for the same reason as the reduced-motion keys
      // (review fix R1): the scroll styles arrive after hydration.
      key={live ? "live" : "still"}
    >
      {children}
    </m.div>
  );
}

/**
 * WEB-5 O5: the dark wash that keeps the hero copy readable leaves with
 * it, over the live lake. It used to stay, and when the hero's pinned stage
 * scrolled away its bottom edge crossed the underwater view as a hard
 * horizontal band (the owner's −2 m to −5 m frames). It is gone at 0.46 of
 * the hero, before the stage starts to move (0.55). The DOM hero keeps it.
 */
export function HeroWashFade({ children }: { children: React.ReactNode }) {
  const { mode } = useLandingMode();
  const live = mode === "full";
  const washOpacity = useTransform(useProgress(), [0.28, 0.46], live ? [1, 0] : [1, 1]);
  return (
    <m.div aria-hidden="true" className="pointer-events-none absolute inset-0" style={live ? { opacity: washOpacity } : undefined} key={live ? "live" : "still"}>
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
