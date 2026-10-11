"use client";

import * as React from "react";
import { AnimatePresence, motion, useMotionValueEvent, useTransform, type MotionValue } from "motion/react";
import { cn } from "@/lib/utils";
import { DEPTHS, type DepthLayer } from "./landing-copy";
import { useLandingMode } from "./landing-mode";
import { Container, DisplayHeading, Eyebrow } from "./landing-ui";
import { activeLayerIndex } from "./scroll-timeline";
import { useSectionProgress } from "./use-section-progress";

/**
 * S3 — "Three depths": the medallion layers, Bronze to Gold.
 *
 * A 300 vh section with a pinned stage. With the canvas, the slabs are 3D
 * behind this stage and this DOM is the sticky text panel and the depth
 * gauge; both read the same progress as the engine (`scroll-timeline.ts`),
 * so the lit slab, the copy and the gauge marker always agree. Without the
 * canvas the same stage shows a CSS stack in its place. With reduced motion
 * there is no pin at all: the three depths are a plain list of cards.
 */
export function DepthsSection() {
  const ref = React.useRef<HTMLElement>(null);
  const { mode } = useLandingMode();
  const progress = useSectionProgress(ref, ["start start", "end end"]);
  const [active, setActive] = React.useState<0 | 1 | 2>(0);
  useMotionValueEvent(progress, "change", (v) => {
    const next = activeLayerIndex(v);
    setActive((prev) => (prev === next ? prev : next));
  });
  const layer = DEPTHS.layers[active];

  return (
    <section ref={ref} id="depths" className="relative h-[300vh] motion-reduce:h-auto">
      <h2 className="sr-only">Three depths</h2>

      {/* Pinned, scroll-driven stage. */}
      <div className="sticky top-0 h-[100svh] overflow-hidden motion-reduce:hidden">
        {/* The gauge hugs the screen's left edge, outside the 1120 px column. */}
        <DepthGauge progress={progress} active={active} />
        <Container className="relative h-full">
          <div className="grid h-full items-center gap-6 pt-20 pb-10 max-md:grid-rows-[auto_1fr] max-md:items-start max-md:pl-6 md:grid-cols-[minmax(0,420px)_1fr] md:pl-36">
            <div className="rounded-3xl max-md:bg-[#020514]/55 max-md:p-4 max-md:backdrop-blur-md">
              <Eyebrow aria-hidden="true">Three depths</Eyebrow>
              <AnimatePresence mode="wait" initial={false}>
                <motion.div
                  key={layer.id}
                  initial={{ opacity: 0, y: 26, filter: "blur(6px)" }}
                  animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                  exit={{ opacity: 0, y: -20, filter: "blur(6px)" }}
                  transition={{ duration: 0.45, ease: [0.16, 1, 0.3, 1] }}
                  className="mt-5"
                  aria-live="polite"
                >
                  <p
                    className="lk-mono text-[12px] font-medium tracking-[0.22em] uppercase"
                    style={{ color: layer.accent }}
                  >
                    {layer.label} · {layer.meters}
                  </p>
                  <h3 className="lk-display mt-3 text-[clamp(2.5rem,5.2vw,4.25rem)] leading-[1.02] text-[#E8F4FF]">
                    {layer.title}
                  </h3>
                  <p className="mt-4 max-w-[28rem] text-[17px] leading-[1.6] text-[#E8F4FF]/85">{layer.body}</p>
                </motion.div>
              </AnimatePresence>
            </div>
            {/* The 3D stack renders on the canvas behind this cell; without one, a CSS stack stands in. */}
            <div className="relative h-full min-h-[280px]">
              {mode !== "full" && <CssStack active={active} />}
            </div>
          </div>
        </Container>
      </div>

      {/* Reduced motion: no pin, no canvas, all three depths on the page. */}
      <div className="hidden py-28 motion-reduce:block">
        <Container>
          <Eyebrow aria-hidden="true">Three depths</Eyebrow>
          <DisplayHeading className="mt-4" aria-hidden="true">
            {DEPTHS.headline}
          </DisplayHeading>
          <ol className="mt-12 grid gap-5 md:grid-cols-3">
            {DEPTHS.layers.map((l) => (
              <li
                key={l.id}
                className="rounded-3xl border border-[#E8F4FF]/10 bg-[#E8F4FF]/[0.04] p-7"
                style={{ boxShadow: `inset 0 1px 0 0 ${l.accent}66` }}
              >
                <p className="lk-mono text-[12px] tracking-[0.2em] uppercase" style={{ color: l.accent }}>
                  {l.label} · {l.meters}
                </p>
                <h3 className="lk-display mt-3 text-[34px] leading-[1.05] text-[#E8F4FF]">{l.title}</h3>
                <p className="mt-3 text-[16px] leading-[1.6] text-[#E8F4FF]/85">{l.body}</p>
              </li>
            ))}
          </ol>
        </Container>
      </div>
    </section>
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
  const accent = DEPTHS.layers[active].accent;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute top-[18%] bottom-[14%] left-3 z-10 w-16 md:left-8 md:w-36"
    >
      <span className="absolute top-0 left-[7px] h-full w-px bg-gradient-to-b from-transparent via-[#E8F4FF]/30 to-transparent" />
      {DEPTHS.layers.map((l, i) => (
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
              "lk-mono hidden text-[11px] tracking-[0.12em] whitespace-nowrap uppercase transition-colors duration-500 md:inline",
            )}
            style={{ color: active === i ? l.accent : "rgba(232,244,255,0.5)" }}
          >
            {l.meters} {l.label}
          </span>
        </div>
      ))}
      <motion.span
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
        {DEPTHS.layers.map((l, i) => (
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
    <motion.div
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
