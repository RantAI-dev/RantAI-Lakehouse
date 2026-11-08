import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS } from "@/lib/home-content";
import { Container, DisplayHeading, Eyebrow } from "@/components/shared/primitives";
import { ProgressSection } from "@/components/shared/progress-section";
import { DepthsStage } from "./depths-stage";
import { DepthDataCard } from "./depth-data-card";

/**
 * WEB-5 O21: without JavaScript the pinned stage cannot follow the scroll,
 * so it would show Bronze for the whole section. The page then reads as with
 * reduced motion instead: no pin, and every depth with its final card. A
 * `<noscript>` style, because it is the one switch that exists before any
 * script runs and costs nothing for everyone else.
 */
const NO_SCRIPT_CSS =
  ".lk-depth-pin{height:auto!important}.lk-depth-stage{display:none!important}.lk-depth-list{display:block!important}";

/**
 * S3 — "Three depths": the medallion layers, Bronze to Gold.
 *
 * A 300 vh section with a pinned stage. With the canvas, the slabs are 3D
 * behind this stage and this DOM is the sticky text panel and the depth
 * gauge; both read the same progress as the engine (`scroll-timeline.ts`),
 * so the lit slab, the copy and the gauge marker always agree. Without the
 * canvas the same stage shows a CSS stack in its place. With reduced motion
 * (and, since O21, without JavaScript) there is no pin at all: the three
 * depths are a plain list, each with its data card at rest.
 *
 * WEB-5 review fix R5: a server component. The pinned stage is the client
 * leaf in `depths-stage.tsx`; the reduced-motion list renders here.
 */
export function DepthsSection({ copy }: { copy: HomeCopy["depths"] }) {
  return (
    <ProgressSection
      range={["start start", "end end"]}
      id="depths"
      // WEB-5 audit #19: with JavaScript and motion, the pinned stage starts a quarter screen before the
      // sources' stage has left (their content never meets: the sources' copy is above its middle by
      // then), so the hand-over is not a screen of empty water.
      className="lk-depth-pin relative h-[300vh] motion-reduce:h-auto motion-safe:[html[data-scene]_&]:-mt-[25vh]"
    >
      <h2 className="sr-only">{copy.eyebrow}</h2>
      <noscript dangerouslySetInnerHTML={{ __html: `<style>${NO_SCRIPT_CSS}</style>` }} />

      {/* Pinned, scroll-driven stage. */}
      <DepthsStage copy={copy} />

      {/* Reduced motion (and no JavaScript): no pin, no canvas, all three depths on the page, each with its card at rest. */}
      <div className="lk-depth-list hidden py-28 motion-reduce:block">
        <Container>
          <Eyebrow aria-hidden="true">{copy.eyebrow}</Eyebrow>
          <DisplayHeading className="mt-4" aria-hidden="true">
            {copy.headline}
          </DisplayHeading>
          <ol className="mt-10 grid gap-5 lg:grid-cols-3">
            {DEPTH_LAYERS.map((l, i) => (
              <li
                key={l.id}
                className="flex flex-col rounded-3xl bg-[var(--lk-n-surface)] p-5 md:p-6"
                style={{ boxShadow: `inset 0 1px 0 0 ${l.accent}66` }}
              >
                <p className="text-[12px] tracking-[0.2em] uppercase" style={{ color: l.accent }}>
                  {l.label} · {l.meters}
                </p>
                <h3 className="lk-display mt-3 text-[30px] leading-[1.08] text-[#E8F4FF]">{copy.layers[l.id].title}</h3>
                <p className="mt-3 text-[15px] leading-[1.55] text-[#E8F4FF]/85">{copy.layers[l.id].body}</p>
                <DepthDataCard stage={i as 0 | 1 | 2} copy={copy.card} animate={false} className="mt-auto pt-5" />
              </li>
            ))}
          </ol>
        </Container>
      </div>
    </ProgressSection>
  );
}
