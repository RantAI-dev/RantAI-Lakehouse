import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS } from "@/lib/home-content";
import { Container, DisplayHeading, Eyebrow } from "@/components/shared/primitives";
import { ProgressSection } from "@/components/shared/progress-section";
import { DepthsStage } from "./depths-stage";

/**
 * S3 — "Three depths": the medallion layers, Bronze to Gold.
 *
 * A 300 vh section with a pinned stage. With the canvas, the slabs are 3D
 * behind this stage and this DOM is the sticky text panel and the depth
 * gauge; both read the same progress as the engine (`scroll-timeline.ts`),
 * so the lit slab, the copy and the gauge marker always agree. Without the
 * canvas the same stage shows a CSS stack in its place. With reduced motion
 * there is no pin at all: the three depths are a plain list of cards.
 *
 * WEB-5 review fix R5: a server component. The pinned stage is the client
 * leaf in `depths-stage.tsx`; the reduced-motion list renders here.
 */
export function DepthsSection({ copy }: { copy: HomeCopy["depths"] }) {
  return (
    <ProgressSection range={["start start", "end end"]} id="depths" className="relative h-[300vh] motion-reduce:h-auto">
      <h2 className="sr-only">{copy.eyebrow}</h2>

      {/* Pinned, scroll-driven stage. */}
      <DepthsStage copy={copy} />

      {/* Reduced motion: no pin, no canvas, all three depths on the page. */}
      <div className="hidden py-28 motion-reduce:block">
        <Container>
          <Eyebrow aria-hidden="true">{copy.eyebrow}</Eyebrow>
          <DisplayHeading className="mt-4" aria-hidden="true">
            {copy.headline}
          </DisplayHeading>
          <ol className="mt-12 grid gap-5 md:grid-cols-3">
            {DEPTH_LAYERS.map((l) => (
              <li
                key={l.id}
                className="rounded-3xl border border-[#E8F4FF]/10 bg-[#E8F4FF]/[0.04] p-7"
                style={{ boxShadow: `inset 0 1px 0 0 ${l.accent}66` }}
              >
                <p className="text-[12px] tracking-[0.2em] uppercase" style={{ color: l.accent }}>
                  {l.label} · {l.meters}
                </p>
                <h3 className="lk-display mt-3 text-[34px] leading-[1.05] text-[#E8F4FF]">{copy.layers[l.id].title}</h3>
                <p className="mt-3 text-[16px] leading-[1.6] text-[#E8F4FF]/85">{copy.layers[l.id].body}</p>
              </li>
            ))}
          </ol>
        </Container>
      </div>
    </ProgressSection>
  );
}
