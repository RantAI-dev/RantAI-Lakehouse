"use client";

import * as React from "react";
import { m, useTransform, type MotionValue } from "framer-motion";
import { useProgress } from "@/components/shared/progress-section";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The sources section's client leaves (WEB-5 review fix R5): the headline
 * box's scroll fade and the falling pills. The headline, lead and caption
 * text arrive as server-rendered `children` or as plain strings.
 */

/** Fades the headline box in as the section arrives (opacity and travel). */
export function SourcesHeadingFade({ children }: { children: React.ReactNode }) {
  const reduced = useReducedMotionSafe();
  // 0 when the section's top meets the bottom of the viewport; 1 at the end of the pin.
  const scrollYProgress = useProgress();
  const headOpacity = useTransform(scrollYProgress, [0.04, 0.2], [0, 1]);
  const headY = useTransform(scrollYProgress, [0.04, 0.2], [48, 0]);
  return (
    <m.div
      // WEB-5 D5: scroll fades it in from opacity 0, which the server
      // HTML carries; `data-reveal` shows it in full without JavaScript.
      data-reveal
      // Audit #4: no dark plate behind the copy on phones any more (it showed
      // as a box while the chips flew in behind it). The whale's eyes have no
      // glow since O48, so the text keeps its contrast on the water, and the
      // chips are only drawn inside the list's own area (sources-section.tsx).
      className="relative z-10"
      // Keyed on `reduced`: before motion's features load an `m` element keeps
      // the style it first rendered (review fix R1), so a switch must remount it.
      key={reduced ? "reduced" : "motion"}
      style={reduced ? { opacity: 1, y: 0 } : { opacity: headOpacity, y: headY }}
    >
      {children}
    </m.div>
  );
}

/**
 * The order the pills land in, from their places in the wrapped list
 * (WEB-5 owner QA O13): the bottom row first, then each row above it, left
 * to right. A droplet then only ever falls through slots that are still
 * empty, so a pill in flight never crosses one that has landed. Read from
 * layout (`offsetTop`, which transforms do not change), again whenever the
 * list rewraps; until then, list order.
 */
function useLandingOrder(listRef: React.RefObject<HTMLLIElement | null>, count: number): { order: number[]; shown: number } {
  const [state, setState] = React.useState(() => ({ order: Array.from({ length: count }, (_, i) => i), shown: count }));
  React.useEffect(() => {
    const list = listRef.current?.parentElement;
    if (!list) return;
    const measure = () => {
      const pills = [...list.querySelectorAll<HTMLLIElement>(":scope > li[data-pill]")];
      // Owner QA 2026-11-25: on a phone only some pills are on show (the others are visually
      // hidden, 1 px wide); only those are ranked, and the rest land last, unseen.
      const onShow = pills.map((p) => p.offsetWidth > 1);
      const seen = pills.filter((_, i) => onShow[i]);
      const rows = [...new Set(seen.map((p) => p.offsetTop))].sort((a, b) => b - a);
      const ranked = pills
        .map((p, i) => ({ i, row: onShow[i] ? rows.indexOf(p.offsetTop) : rows.length, left: p.offsetLeft }))
        .sort((a, b) => a.row - b.row || a.left - b.left);
      const next = Array.from({ length: pills.length }, () => 0);
      ranked.forEach((r, k) => (next[r.i] = k));
      setState((prev) =>
        prev.shown === seen.length && prev.order.length === next.length && prev.order.every((v, i) => v === next[i])
          ? prev
          : { order: next, shown: seen.length },
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(list);
    return () => ro.disconnect();
  }, [listRef, count]);
  return state;
}

/**
 * The pills, as the items of the server-rendered list. WEB-5 O20: each
 * pill's mark arrives rendered by the server (`marks`, with its hover
 * colour in `colours`), so the brand paths travel as HTML and stay out of
 * the first JavaScript (they were 11 KB of it when imported here).
 */
export function SourcePills({
  items,
  marks,
  colours,
  phone,
  more,
}: {
  items: readonly string[];
  marks: readonly React.ReactNode[];
  colours: readonly string[];
  /** Owner QA 2026-11-25: the pills on show below 1024 px; the others are visually hidden there. */
  phone: readonly number[];
  /** The line after them on a phone, "+11 more". */
  more: string;
}) {
  const reduced = useReducedMotionSafe();
  const progress = useProgress();
  // A hidden, empty item, so the order can find the list it sits in (hidden
  // items are not announced and take no place in the wrap).
  const markerRef = React.useRef<HTMLLIElement>(null);
  const { order, shown } = useLandingOrder(markerRef, items.length);
  return (
    <>
      <li ref={markerRef} hidden />
      {items.map((item, i) => (
        <SourcePill
          key={item}
          label={item}
          mark={marks[i]}
          colour={colours[i] ?? "#8fcbff"}
          index={order[i] ?? i}
          count={shown}
          progress={progress}
          animate={!reduced}
          className={phone.includes(i) ? undefined : "max-lg:sr-only"}
        />
      ))}
      {/* Plain text, not a pill: it is not a control. The list above already names every source. */}
      <MoreLine label={more} progress={progress} animate={!reduced} />
    </>
  );
}

const MORE_CLASS = "inline-flex items-center px-2 py-1 text-[12px] text-[#E8F4FF]/60 lg:hidden";

/** Owner QA 2026-11-25: "+11 more" on a phone, fading in once the last pill has landed. */
function MoreLine({ label, progress, animate }: { label: string; progress: MotionValue<number>; animate: boolean }) {
  const opacity = useTransform(progress, [0.66, 0.76], [0, 1]);
  if (!animate) {
    return (
      <li aria-hidden="true" className={MORE_CLASS}>
        {label}
      </li>
    );
  }
  return (
    <m.li aria-hidden="true" data-reveal className={MORE_CLASS} style={{ opacity }}>
      {label}
    </m.li>
  );
}

/**
 * Ease-out cubic: the droplet decelerates into its slot and stops there.
 * WEB-1 review round 1 SHOULD-FIX 1: this used to overshoot by about 10 % of
 * the fall (around 100 px on a desktop), which pushed the last pills down over
 * the caption. Transforms never change layout, so the grid itself already
 * holds its settled height; the pills just must not travel past it.
 */
const settle = (t: number): number => 1 - Math.pow(1 - t, 3);

/** Gap between two droplets' departures, so the last of `count` arrives by 0.80. */
export const stagger = (count: number): number => (count > 1 ? Math.min(0.034, 0.4 / (count - 1)) : 0);

/**
 * WEB-5 O20: the mark sits before the name, in the foam at 85 %, and turns
 * its brand colour on hover while the pill lifts a little and its border
 * brightens (200 ms; at once under reduced motion). The lift uses the
 * `translate` property and the border an important colour, because the
 * fall drives the pill's `transform` and colours inline.
 */
const PILL_CLASS =
  "group/pill inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[12px] text-[#E8F4FF] transition-[translate,border-color] duration-200 ease-out hover:-translate-y-0.5 hover:border-[#8fcbff]/70! motion-reduce:transition-none md:gap-2 md:px-4 md:py-2 md:text-[13px]";

function PillContent({ label, mark }: { label: string; mark: React.ReactNode }) {
  return (
    <>
      {mark}
      {label}
    </>
  );
}

function SourcePill({
  label,
  mark,
  colour,
  index,
  count,
  progress,
  animate,
  className,
}: {
  label: string;
  mark: React.ReactNode;
  colour: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
  animate: boolean;
  className?: string;
}) {
  if (!animate) {
    return (
      <li data-pill className={`${PILL_CLASS} border-[#44A6FA]/30 bg-[#44A6FA]/10 ${className ?? ""}`} style={{ "--lk-brand": colour } as React.CSSProperties}>
        <PillContent label={label} mark={mark} />
      </li>
    );
  }
  return <FallingPill label={label} mark={mark} colour={colour} index={index} count={count} progress={progress} className={className} />;
}

function FallingPill({
  label,
  mark,
  colour,
  index,
  count,
  progress,
  className,
}: {
  label: string;
  mark: React.ReactNode;
  colour: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
  className?: string;
}) {
  // Staggered: the first droplet leaves at 0.10, the last arrives by 0.80.
  // WEB-5 T6: the step shrinks when the list grows (19 sources now), so the
  // last droplet still lands inside the pin; nine pills keep the old 0.034.
  // O13: `index` is the landing order (bottom row first), not list order.
  const start = 0.1 + index * stagger(count);
  const end = start + 0.3;
  const dir = index % 2 === 0 ? -1 : 1;
  const y = useTransform(progress, [start, end], ["-110vh", "0vh"], { ease: settle });
  // WEB-5 owner QA O13: the drift and the tilt used to reach 198 px and 16°,
  // so a falling pill swept across its landed neighbours. Now a droplet
  // sways a little high up and is straight, in its own column, well before
  // it reaches the row, so it never crosses a pill beside it.
  const x = useTransform(progress, [start, start + 0.2], [`${dir * (8 + (index % 4) * 4)}px`, "0px"], { ease: settle });
  const rotate = useTransform(progress, [start, start + 0.2], [((index % 3) - 1) * 6, 0], { ease: settle });
  const opacity = useTransform(progress, [start, start + 0.06, end], [0, 0.55, 1]);
  const filter = useTransform(progress, [start, end - 0.05], ["blur(4px)", "blur(0px)"]);
  // Stretched while falling, compressed for a beat on arrival.
  const scaleY = useTransform(progress, [start, start + 0.08, end - 0.06, end], [1.5, 1.3, 1.0, 1]);
  const glow = useTransform(
    progress,
    [start, end - 0.02, end + 0.04],
    [
      "0 0 30px 8px rgba(68,166,250,0.65)",
      "0 0 18px 3px rgba(68,166,250,0.35)",
      "0 0 0px 0px rgba(68,166,250,0)",
    ],
  );
  const border = useTransform(
    progress,
    [start, end],
    ["rgba(124,196,255,0.9)", "rgba(68,166,250,0.3)"],
  );
  const fill = useTransform(progress, [start, end], ["rgba(68,166,250,0.35)", "rgba(68,166,250,0.1)"]);
  return (
    <m.li
      data-reveal
      data-pill
      className={`${PILL_CLASS} ${className ?? ""}`}
      style={{
        y,
        x,
        rotate,
        opacity,
        filter,
        scaleY,
        boxShadow: glow,
        borderColor: border,
        backgroundColor: fill,
        ["--lk-brand" as string]: colour,
      }}
    >
      <PillContent label={label} mark={mark} />
    </m.li>
  );
}
