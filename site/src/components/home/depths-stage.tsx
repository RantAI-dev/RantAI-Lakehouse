"use client";

import * as React from "react";
import { AnimatePresence, m, useMotionValueEvent, useTransform, type MotionValue } from "framer-motion";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS } from "@/lib/home-content";
import { useLandingMode } from "@/components/effects/scene-mode";
import { Container } from "@/components/shared/primitives";
import { useProgress } from "@/components/shared/progress-section";
import { activeLayerIndex } from "@/lib/scroll-timeline";
import { DepthDataCard } from "./depth-data-card";

/**
 * The depths section's pinned, scroll-driven stage: the depth gauge, the
 * panel that swaps to the lit depth, and the CSS stack when there is no
 * canvas. The client leaf of `depths-section.tsx` (WEB-5 review fix R5);
 * the reduced-motion list of all three depths renders on the server there.
 *
 * WEB-5 O21: the panel shows what each depth does, not only says it: the
 * three-step key, a short title and line, and the data card, whose rows
 * change as the lit depth changes. The card follows the same `active` as
 * the copy and the gauge, so all three turn at the same scroll position.
 */
export function DepthsStage({ copy }: { copy: HomeCopy["depths"] }) {
  const { mode } = useLandingMode();
  const progress = useProgress();
  // WEB-5 D5: React state changes only when the lit depth changes (twice
  // over the whole section), never per scroll event; the ref holds the
  // current index so the handler can compare without calling setState.
  const [active, setActive] = React.useState<0 | 1 | 2>(0);
  const activeRef = React.useRef<0 | 1 | 2>(0);
  useMotionValueEvent(progress, "change", (v) => {
    const next = activeLayerIndex(v);
    if (next === activeRef.current) return;
    activeRef.current = next;
    setActive(next);
  });
  const layer = DEPTH_LAYERS[active];
  const text = copy.layers[layer.id];

  return (
    <div className="lk-depth-stage sticky top-0 h-[100svh] overflow-hidden motion-reduce:hidden">
      {/* The gauge hugs the screen's left edge, outside the 1120 px column. */}
      <DepthGauge progress={progress} active={active} words={copy.key} />
      <Container className="relative h-full">
        <div className="lk-depth-grid-stage grid h-full items-center gap-6 pt-20 pb-10 max-md:grid-rows-[auto_1fr] max-md:items-start max-md:gap-5 max-md:pl-6 md:grid-cols-[minmax(0,2fr)_minmax(0,3fr)] md:gap-12">
          {/* WEB-5 O21: the chapter shows what the depth does to the data.
              O40 (owner: "masih kerasa rame sekali ... tablenya di sebelah
              kanan aja dan di gabung dengan tampilan balok"): the left column
              is the words only, a title and one line (the key stepper is
              gone: the page's depth gauge names the layer); the right column
              is one focal pair, the data card resting on its layer's slab. */}
          <AnimatePresence mode="wait" initial={false}>
            <m.div
              key={layer.id}
              initial={{ opacity: 0, y: 18, filter: "blur(6px)" }}
              animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
              exit={{ opacity: 0, y: -14, filter: "blur(6px)" }}
              transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
              aria-live="polite"
            >
              <h3 className="lk-display text-[clamp(2.25rem,10vw,2.75rem)] leading-[1.04] text-balance text-[#E8F4FF] md:text-[clamp(2.6rem,3.8vw,3.4rem)]">
                {/* The depth's name for assistive tech; on screen the gauge says it. */}
                <span className="sr-only">
                  {layer.label} {layer.meters}:{" "}
                </span>
                {text.title}
              </h3>
              <p className="mt-3 max-w-[24rem] text-[16px] leading-[1.55] text-pretty text-[#E8F4FF]/75 md:mt-4 md:text-[17px]">
                {text.body}
              </p>
            </m.div>
          </AnimatePresence>
          {/* The card on its slab. With the canvas the 3D slab is drawn under
              the card's box (`data-depths-card`, lake-engine.ts); without one,
              the CSS slab below stands in, in the same place. */}
          <div className="lk-depth-pair relative mx-auto w-full max-w-[460px] md:mr-0">
            <DepthDataCard stage={active} copy={copy.card} className="lk-depth-on-slab relative z-10" />
            {mode !== "full" && <CssSlab active={active} />}
          </div>
        </div>
      </Container>
    </div>
  );
}

/** Where each depth's tick sits along the gauge, in percent of its height. */
const TICKS = [14, 50, 86] as const;

/**
 * The gauge on the left edge: three ticks and a marker that follows the
 * scroll. On phones the labels drop away (the panel's own eyebrow says the
 * same thing) and only the line and the marker remain.
 */
function DepthGauge({
  progress,
  active,
  words,
}: {
  progress: MotionValue<number>;
  active: 0 | 1 | 2;
  /** WEB-5 O40: each depth's word ("raw", "clean", "ready"), now that the key above the card is gone. */
  words: Record<"bronze" | "silver" | "gold", string>;
}) {
  // The marker rides the same eased path as the camera: it rests on each tick
  // at the middle of that depth's third.
  const top = useTransform(progress, [0, 1 / 6, 1 / 2, 5 / 6, 1], ["6%", "14%", "50%", "86%", "94%"]);
  const accent = DEPTH_LAYERS[active].accent;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute top-[18%] bottom-[14%] left-3 z-10 w-16 md:left-8 md:w-36"
    >
      <span className="absolute top-0 left-[7px] h-full w-px bg-[#E8F4FF]/20" />
      {DEPTH_LAYERS.map((l, i) => (
        <div
          key={l.id}
          className="absolute left-0 flex -translate-y-1/2 items-center gap-3"
          style={{ top: `${TICKS[i]}%` }}
        >
          <span
            className="block h-px w-4 transition-colors duration-500"
            style={{ background: active === i ? l.accent : "rgba(232,244,255,0.35)" }}
          />
          <span
            className={cn(
              "hidden text-[11px] tracking-[0.12em] whitespace-nowrap uppercase transition-colors duration-500 md:inline",
            )}
            style={{ color: active === i ? l.accent : "rgba(232,244,255,0.5)" }}
          >
            {l.meters} {l.label} · {words[l.id]}
          </span>
        </div>
      ))}
      <m.span
        className="absolute left-[3px] block size-[9px] -translate-y-1/2 rounded-full"
        style={{
          top,
          background: accent,
          boxShadow: `0 0 14px 3px ${accent}99`,
        }}
      />
    </div>
  );
}

/**
 * Stand-in for the 3D slab when there is no canvas (WEB-5 O40): one slab of
 * the current depth, in perspective, that the card rests on. Its top face
 * shows in the strip the card leaves free below it (`--lk-depths-card-lift`
 * in app/globals.css) and its front edge under that. Solid tints only.
 */
function CssSlab({ active }: { active: 0 | 1 | 2 }) {
  const accent = DEPTH_LAYERS[active].accent;
  return (
    <div aria-hidden="true" className="lk-depth-slab" style={{ "--lk-slab": accent } as React.CSSProperties}>
      <span className="lk-depth-slab-top" />
      <span className="lk-depth-slab-front" />
    </div>
  );
}
