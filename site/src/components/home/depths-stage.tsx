"use client";

import * as React from "react";
import { AnimatePresence, m, useMotionValueEvent, useTransform, type MotionValue } from "framer-motion";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS, type DepthLayer } from "@/lib/home-content";
import { useLandingMode } from "@/components/effects/scene-mode";
import { Container } from "@/components/shared/primitives";
import { useProgress } from "@/components/shared/progress-section";
import { activeLayerIndex } from "@/lib/scroll-timeline";
import { DepthDataCard } from "./depth-data-card";
import { DepthKey } from "./depth-key";

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
      <DepthGauge progress={progress} active={active} />
      <Container className="relative h-full">
        <div className="grid h-full items-center gap-6 pt-20 pb-10 max-md:grid-rows-[auto_1fr] max-md:items-start max-md:gap-3 max-md:pl-6 md:grid-cols-[minmax(0,420px)_1fr] md:pl-36">
          {/* WEB-5 O21: the chapter shows what the depth does to the data.
              The key says the whole idea at once, the text is a title and
              one line, and the card below is the same few rows at this
              depth (depth-data-card.tsx). It sits in the text column, so
              the 3D slabs behind the other cell stay in view. */}
          <div className="rounded-3xl max-md:bg-[#020514]/55 max-md:p-4 max-md:backdrop-blur-md">
            <DepthKey active={active} words={copy.key} />
            <AnimatePresence mode="wait" initial={false}>
              <m.div
                key={layer.id}
                initial={{ opacity: 0, y: 18, filter: "blur(6px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                exit={{ opacity: 0, y: -14, filter: "blur(6px)" }}
                transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
                className="mt-4 md:mt-6"
                aria-live="polite"
              >
                <p
                  className="text-[12px] font-medium tracking-[0.22em] uppercase"
                  style={{ color: layer.accent }}
                >
                  {layer.label} · {layer.meters}
                </p>
                <h3 className="lk-display mt-2 text-[clamp(2rem,9vw,2.5rem)] leading-[1.04] text-balance text-[#E8F4FF] md:mt-3 md:text-[clamp(2.5rem,3.9vw,3.4rem)]">
                  {text.title}
                </h3>
                <p className="mt-2 max-w-[26rem] text-[15px] leading-[1.5] text-pretty text-[#E8F4FF]/85 md:mt-3 md:text-[16px]">
                  {text.body}
                </p>
              </m.div>
            </AnimatePresence>
            <DepthDataCard stage={active} copy={copy.card} className="mt-4 md:mt-6" />
          </div>
          {/* The 3D stack renders on the canvas behind this cell; without one, a CSS stack stands in. */}
          <div className="lk-depth-stack-cell relative h-full min-h-0 md:min-h-[280px]">
            {mode !== "full" && <CssStack active={active} />}
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
function DepthGauge({ progress, active }: { progress: MotionValue<number>; active: 0 | 1 | 2 }) {
  // The marker rides the same eased path as the camera: it rests on each tick
  // at the middle of that depth's third.
  const top = useTransform(progress, [0, 1 / 6, 1 / 2, 5 / 6, 1], ["6%", "14%", "50%", "86%", "94%"]);
  const accent = DEPTH_LAYERS[active].accent;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute top-[18%] bottom-[14%] left-3 z-10 w-16 md:left-8 md:w-36"
    >
      <span className="absolute top-0 left-[7px] h-full w-px bg-gradient-to-b from-transparent via-[#E8F4FF]/30 to-transparent" />
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
            {l.meters} {l.label}
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
 * Stand-in for the 3D stack when there is no canvas: three tilted glass
 * plates in CSS, the active one lit and the others drawn apart. Same story,
 * flatter.
 */
function CssStack({ active }: { active: 0 | 1 | 2 }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center [perspective:1200px]" aria-hidden="true">
      <div className="relative h-[300px] w-[min(86%,440px)] [transform-style:preserve-3d] [transform:rotateX(58deg)_rotateZ(-16deg)]">
        {DEPTH_LAYERS.map((l, i) => (
          <Plate key={l.id} layer={l} index={i} active={active} />
        ))}
      </div>
    </div>
  );
}

function Plate({ layer, index, active }: { layer: DepthLayer; index: number; active: 0 | 1 | 2 }) {
  const isActive = index === active;
  const lift = (index - active) * 70 + (index - 1) * 6;
  return (
    <m.div
      className="absolute inset-x-0 top-1/2 h-[190px] -translate-y-1/2 rounded-[22px] border backdrop-blur-sm"
      initial={false}
      animate={{
        translateZ: -lift,
        opacity: isActive ? 1 : 0.7,
        boxShadow: isActive ? `0 0 60px 6px ${layer.accent}55` : `0 0 0px 0px ${layer.accent}00`,
      }}
      transition={{ duration: 0.6, ease: [0.16, 1, 0.3, 1] }}
      style={{
        borderColor: `${layer.accent}${isActive ? "cc" : "55"}`,
        background: `linear-gradient(135deg, ${layer.accent}${isActive ? "40" : "22"}, rgba(5,10,48,0.55))`,
      }}
    />
  );
}
