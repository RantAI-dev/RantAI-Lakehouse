"use client";

import * as React from "react";
import { m, useTransform, type MotionValue } from "motion/react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { Reveal } from "@/components/shared/reveal";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { useSectionProgress } from "@/hooks/use-section-progress";

/**
 * S2 — "Everything flows in": the sources that land in the lakehouse today.
 *
 * The section is 230 vh with a pinned 100 vh stage. Each source is a glowing
 * droplet that falls from above the stage and settles into the grid as the
 * visitor scrolls, so scrolling back up lifts them out again. With reduced
 * motion the section is a plain block and the pills simply sit in the grid.
 */
export function SourcesSection({ copy }: { copy: HomeCopy["sources"] }) {
  const ref = React.useRef<HTMLElement>(null);
  const reduced = useReducedMotionSafe();
  // 0 when the section's top meets the bottom of the viewport; 1 at the end of the pin.
  const scrollYProgress = useSectionProgress(ref, ["start end", "end end"]);
  const headOpacity = useTransform(scrollYProgress, [0.04, 0.2], [0, 1]);
  const headY = useTransform(scrollYProgress, [0.04, 0.2], [48, 0]);

  return (
    <section ref={ref} id="sources" className="relative h-[230vh] motion-reduce:h-auto">
      <div className="sticky top-0 flex h-[100svh] flex-col items-center justify-center overflow-hidden motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
        <Container className="text-center">
          <m.div
            // WEB-5 D5: scroll fades it in from opacity 0, which the server
            // HTML carries; `data-reveal` shows it in full without JavaScript.
            data-reveal
            // WEB-1 review round 1 SHOULD-FIX 5: on a phone the copy sits over the
            // glow of the mascot's eye seen through the water, so it gets a dark
            // backing there (body text stays above 4.5:1 on it).
            className="relative z-10 max-md:rounded-3xl max-md:bg-[#020b2a]/70 max-md:p-4"
            style={reduced ? { opacity: 1, y: 0 } : { opacity: headOpacity, y: headY }}
          >
            <DisplayHeading>{copy.headline}</DisplayHeading>
            <LeadText className="mx-auto mt-5 max-w-[36rem]">{copy.sub}</LeadText>
          </m.div>
          {/* WEB-5 T6: 19 sources instead of 9. On phones the pills, gaps and
              margins are tighter so the pinned 100svh stage still holds the
              headline, every pill and the caption; desktop is unchanged. */}
          <ul className="relative z-0 mx-auto mt-7 flex max-w-[760px] flex-wrap justify-center gap-1.5 md:mt-12 md:gap-3">
            {copy.items.map((item, i) => (
              <SourcePill key={item} label={item} index={i} count={copy.items.length} progress={scrollYProgress} animate={!reduced} />
            ))}
          </ul>
          <Reveal className="mx-auto mt-6 max-w-[36rem] md:mt-10">
            <p className="lk-mono text-[11px] leading-4 md:text-[12px] md:leading-5 tracking-wide text-[#E8F4FF]/65">{copy.caption}</p>
          </Reveal>
        </Container>
      </div>
    </section>
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

const PILL_CLASS =
  "lk-mono rounded-full border px-2.5 py-1 text-[11px] text-[#E8F4FF] md:px-4 md:py-2 md:text-[13px]";

function SourcePill({
  label,
  index,
  count,
  progress,
  animate,
}: {
  label: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
  animate: boolean;
}) {
  if (!animate) {
    return <li className={`${PILL_CLASS} border-[#44A6FA]/30 bg-[#44A6FA]/10`}>{label}</li>;
  }
  return <FallingPill label={label} index={index} count={count} progress={progress} />;
}

function FallingPill({
  label,
  index,
  count,
  progress,
}: {
  label: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
}) {
  // Staggered: the first droplet leaves at 0.10, the last arrives by 0.80.
  // WEB-5 T6: the step shrinks when the list grows (19 sources now), so the
  // last droplet still lands inside the pin; nine pills keep the old 0.034.
  const start = 0.1 + index * stagger(count);
  const end = start + 0.3;
  const dir = index % 2 === 0 ? -1 : 1;
  const y = useTransform(progress, [start, end], ["-110vh", "0vh"], { ease: settle });
  const x = useTransform(progress, [start, end], [`${dir * (36 + index * 9)}px`, "0px"], { ease: settle });
  const rotate = useTransform(progress, [start, end], [((index % 3) - 1) * 16, 0], { ease: settle });
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
      className={PILL_CLASS}
      style={{ y, x, rotate, opacity, filter, scaleY, boxShadow: glow, borderColor: border, backgroundColor: fill }}
    >
      {label}
    </m.li>
  );
}
