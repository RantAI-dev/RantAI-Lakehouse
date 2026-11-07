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
      // WEB-5 O47: the first-load intro reveals the copy through this hook (hero-intro.tsx).
      data-intro-copy=""
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
 *
 * WEB-5 O11: on a phone the wash rises from the bottom over most of the
 * stage, and the 3D whale's eyes, in the room above the copy, read grey
 * through it. Over the live lake on a phone the wash now fades out above
 * the copy: it is clear from 48 px above the bottom of the creature's room
 * (`data-hero-room`, which R12 keeps at least 16 px above the eyebrow) and
 * full at that bottom, so the copy keeps all of its contrast.
 */
export function HeroWashFade({ children }: { children: React.ReactNode }) {
  const { mode } = useLandingMode();
  const live = mode === "full";
  const ref = React.useRef<HTMLDivElement>(null);
  const washOpacity = useTransform(useProgress(), [0.28, 0.46], live ? [1, 0] : [1, 1]);

  React.useEffect(() => {
    const el = ref.current;
    const room = el?.parentElement?.querySelector<HTMLElement>("[data-hero-room]");
    if (!live || !el || !room) return;
    const place = () => {
      if (window.innerWidth >= 768) {
        el.style.removeProperty("mask-image");
        el.style.removeProperty("-webkit-mask-image");
        return;
      }
      const bottom = room.getBoundingClientRect().bottom - el.getBoundingClientRect().top;
      const mask = `linear-gradient(180deg, transparent ${Math.max(0, bottom - 48).toFixed(0)}px, #000 ${bottom.toFixed(0)}px)`;
      el.style.setProperty("mask-image", mask);
      el.style.setProperty("-webkit-mask-image", mask);
    };
    place();
    const ro = new ResizeObserver(place);
    ro.observe(room);
    ro.observe(el);
    return () => ro.disconnect();
  }, [live]);

  return (
    <m.div
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0"
      style={live ? { opacity: washOpacity } : undefined}
      key={live ? "live" : "still"}
    >
      {children}
    </m.div>
  );
}

/** The scroll cue fades over the first few percent of the section. */
export function ScrollCueFade({ children }: { children: React.ReactNode }) {
  const cueOpacity = useTransform(useProgress(), [0, 0.05], [1, 0]);
  return (
    <m.div data-intro-scroll-cue="" style={{ opacity: cueOpacity }}>
      {children}
    </m.div>
  );
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
  // WEB-5 O47: the night-lake picture (not the creature's room) is where the intro's sky fades up.
  const isPicture = !("data-hero-room" in rest);
  return (
    <div
      aria-hidden="true"
      className={cn(className, "transition-opacity duration-700", sceneReady && "opacity-0")}
      data-intro-backdrop={isPicture ? "" : undefined}
      {...rest}
    >
      {children}
    </div>
  );
}
