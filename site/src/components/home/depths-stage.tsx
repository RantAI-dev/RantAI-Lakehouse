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
  // Owner QA: the rail leaves with the stage. The assistant rides up under this section, so the
  // rail must be gone before the assistant's heading shows: `--lk-rail-o` is 1 while the assistant's
  // top is below 95 % of the screen and 0 at 70 %.
  const stageRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const stage = stageRef.current;
    const next = document.getElementById("assistant");
    if (!stage || !next) return;
    let frame = 0;
    let last = "";
    const write = () => {
      frame = 0;
      const vh = window.innerHeight;
      const t = Math.min(1, Math.max(0, (next.getBoundingClientRect().top - 0.7 * vh) / (0.25 * vh)));
      const v = t.toFixed(2);
      if (v !== last) {
        last = v;
        stage.style.setProperty("--lk-rail-o", v);
      }
    };
    const on = () => {
      if (!frame) frame = requestAnimationFrame(write);
    };
    write();
    window.addEventListener("scroll", on, { passive: true });
    window.addEventListener("resize", on);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", on);
      window.removeEventListener("resize", on);
    };
  }, []);

  return (
    <div ref={stageRef} className="lk-depth-stage sticky top-0 h-[100svh] overflow-hidden motion-reduce:hidden">
      <Container className="relative h-full">
        {/* Owner QA (1210 px, 3D path): the rail has its own column, sized by its longest label, so
            the heading always starts a full grid gap (48 px) clear of it, at any width and in either
            language. On phones it is the thin line at the screen's left edge, out of the flow. */}
        <div className="lk-depth-grid-stage grid h-full items-center gap-6 pt-20 pb-10 max-md:grid-rows-[auto_1fr] max-md:items-start max-md:gap-5 max-md:pl-6 md:grid-cols-[8.5rem_minmax(0,2fr)_minmax(0,3fr)] md:gap-12 min-[1200px]:grid-cols-[12.5rem_minmax(0,2fr)_minmax(0,3fr)]">
          {/* Phones: the thin line at the edge, and the heading in the flow (unchanged). */}
          <DepthGauge progress={progress} active={active} words={copy.key} />
          {/* From md: the rail with each depth's heading under its own label. */}
          <DepthRail active={active} copy={copy} />
          {/* WEB-5 O21: the chapter shows what the depth does to the data.
              O40 (owner: "masih kerasa rame sekali ... tablenya di sebelah
              kanan aja dan di gabung dengan tampilan balok"): the left column
              is the words only, a title and one line (the key stepper is
              gone: the page's depth gauge names the layer); the right column
              is one focal pair, the data card resting on its layer's slab. */}
          <AnimatePresence mode="wait" initial={false}>
            <m.div
              className="md:hidden"
              key={layer.id}
              initial={{ opacity: 0, y: 18, filter: "blur(6px)" }}
              animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
              exit={{ opacity: 0, y: -14, filter: "blur(6px)" }}
              transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
              aria-live="polite"
            >
              <h3 className="lk-display text-[clamp(2rem,9vw,2.4rem)] leading-[1.06] text-balance text-[#E8F4FF] md:text-[clamp(2.2rem,3.2vw,2.9rem)]">
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
      className="lk-depth-rail-m pointer-events-none opacity-[var(--lk-rail-o,1)] absolute top-[18%] bottom-[14%] left-3 z-10 w-16 md:hidden"
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
            {l.meters} {l.label}
            <span className="hidden min-[1200px]:inline"> · {words[l.id]}</span>
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
 * From md up: the rail as a column. Each depth is a tick and its label, and the active depth's
 * heading and line sit directly under its label, on the label's own text edge (28 px in: the tick,
 * 16, and the gap, 12). The block opens (grid rows 0fr to 1fr) and the others close, so the
 * labels below it are pushed down and nothing can collide with it, in either language. The dot
 * rests on the active tick. The column has a fixed height, so it does not move while the block
 * changes hands; all of it is eased over 350 ms.
 */
function DepthRail({ active, copy }: { active: 0 | 1 | 2; copy: HomeCopy["depths"] }) {
  return (
    <div className="lk-depth-rail relative opacity-[var(--lk-rail-o,1)] max-md:hidden md:col-span-2" style={{ height: "min(18rem, 100%)" }}>
      <span aria-hidden="true" className="absolute top-1 bottom-1 left-[7px] w-px bg-[#E8F4FF]/20" />
      {DEPTH_LAYERS.map((l, i) => {
        const on = active === i;
        const text = copy.layers[l.id];
        return (
          <div key={l.id} className="relative">
            <div aria-hidden="true" className="flex h-5 items-center gap-3">
              <span
                className="absolute left-[3px] block size-[9px] rounded-full transition-[opacity,scale] duration-[350ms] ease-out"
                style={{ background: l.accent, boxShadow: `0 0 14px 3px ${l.accent}99`, opacity: on ? 1 : 0, scale: on ? 1 : 0.5 }}
              />
              <span className="block h-px w-4 transition-colors duration-[350ms]" style={{ background: on ? l.accent : "rgba(232,244,255,0.35)" }} />
              <span
                className="text-[11px] tracking-[0.12em] whitespace-nowrap uppercase transition-colors duration-[350ms]"
                style={{ color: on ? l.accent : "rgba(232,244,255,0.5)" }}
              >
                {l.meters} {l.label}
                <span className="hidden min-[1200px]:inline"> · {copy.key[l.id]}</span>
              </span>
            </div>
            <div
              className="lk-depth-block grid transition-[grid-template-rows,opacity] duration-[350ms] ease-[cubic-bezier(0.16,1,0.3,1)]"
              style={{ gridTemplateRows: on ? "1fr" : "0fr", opacity: on ? 1 : 0 }}
              aria-hidden={on ? undefined : true}
              aria-live={on ? "polite" : undefined}
              inert={!on}
            >
              <div className="min-h-0 overflow-hidden pl-7">
                <h3 className="lk-display mt-5 text-[clamp(2.1rem,3vw,2.9rem)] leading-[1.06] text-balance text-[#E8F4FF]">
                  <span className="sr-only">
                    {l.label} {l.meters}:{" "}
                  </span>
                  {text.title}
                </h3>
                <p className="mt-3 max-w-[24rem] text-[17px] leading-[1.55] text-pretty text-[#E8F4FF]/75">{text.body}</p>
                <span className="block h-6" />
              </div>
            </div>
          </div>
        );
      })}
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
