import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { ProgressSection } from "@/components/shared/progress-section";
import { Reveal } from "@/components/shared/reveal";
import { hoverColour, SourceIcon } from "@/components/shared/brand-icons";
import { SOURCE_ICONS } from "@/lib/home-content";
import { SourcePills, SourcesHeadingFade } from "./sources-motion";

/**
 * WEB-5 O20: a pill's mark, in the foam at 85 %, turning its brand colour
 * (`--lk-brand`, set by the pill) on hover. Kept here, in the server
 * module that renders the marks.
 */
const PILL_MARK_CLASS =
  "size-3.5 shrink-0 text-[#E8F4FF]/85 transition-colors duration-200 ease-out group-hover/pill:text-[var(--lk-brand)] motion-reduce:transition-none md:size-4";

/**
 * S2 — "Everything flows in": the sources that land in the lakehouse today.
 *
 * The section is 230 vh with a pinned 100 vh stage. Each source is a glowing
 * droplet that falls from above the stage and settles into the grid as the
 * visitor scrolls, so scrolling back up lifts them out again. With reduced
 * motion the section is a plain block and the pills simply sit in the grid.
 *
 * WEB-5 review fix R5: a server component; the fade and the pills are the
 * client leaves in `sources-motion.tsx`.
 */
export function SourcesSection({ copy }: { copy: HomeCopy["sources"] }) {
  return (
    <ProgressSection range={["start end", "end end"]} id="sources" className="relative h-[230vh] motion-reduce:h-auto">
      {/* WEB-5 review fix R4 (spacing only, D6): on phones the stage centres its
          content below the 64 px top bar (pt-16) instead of under it, and the
          headline box, the lead's line height and the margins are tighter, so
          at 360x740 neither the headline nor the caption is clipped. */}
      {/* WEB-5 O5: the stage's top edge is a soft mask, not a hard cut, so a droplet
          falling in from above fades in instead of being sliced by a straight line. */}
      <div className="sticky top-0 flex h-[100svh] flex-col items-center justify-center overflow-hidden [mask-image:linear-gradient(180deg,transparent_0,#000_9%)] max-md:pt-16 motion-reduce:static motion-reduce:h-auto motion-reduce:py-28 motion-reduce:[mask-image:none]">
        <Container className="text-center">
          <SourcesHeadingFade>
            <DisplayHeading>{copy.headline}</DisplayHeading>
            <LeadText className="mx-auto mt-3 max-w-[36rem] max-md:leading-[1.45] md:mt-5">{copy.sub}</LeadText>
          </SourcesHeadingFade>
          {/* WEB-5 T6: 19 sources instead of 9. On phones the pills, gaps and
              margins are tighter so the pinned 100svh stage still holds the
              headline, every pill and the caption; desktop is unchanged. */}
          {/* Audit #4: on phones a droplet is only drawn once it is inside the
              list's own area, fading in over a 24 px band above the first row
              (padding, so the settled pills are never faded), so none crosses
              the heading: a mask, not a backing plate. */}
          <ul className="relative z-0 mx-auto mt-5 flex max-w-[760px] flex-wrap justify-center gap-1.5 max-md:mt-0 max-md:pt-6 max-md:[mask-image:linear-gradient(180deg,transparent_0,#000_20px)] md:mt-12 md:gap-3">
            {/* O20: the marks render here, on the server (see SourcePills). */}
            <SourcePills
              items={copy.items}
              marks={SOURCE_ICONS.map((id, i) => <SourceIcon key={i} id={id} className={PILL_MARK_CLASS} />)}
              colours={SOURCE_ICONS.map(hoverColour)}
            />
          </ul>
          {/* Audit #26: the footnote is readable (14 px, ~8:1 on the water) and wraps without a widow. */}
          <Reveal className="mx-auto mt-4 max-w-[34rem] md:mt-10">
            <p className="text-[14px] leading-[1.5] text-pretty text-[#E8F4FF]/75">{copy.caption}</p>
          </Reveal>
        </Container>
      </div>
    </ProgressSection>
  );
}
