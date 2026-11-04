import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { ProgressSection } from "@/components/shared/progress-section";
import { Reveal } from "@/components/shared/reveal";
import { SourcePills, SourcesHeadingFade } from "./sources-motion";

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
      <div className="sticky top-0 flex h-[100svh] flex-col items-center justify-center overflow-hidden max-md:pt-16 motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
        <Container className="text-center">
          <SourcesHeadingFade>
            <DisplayHeading>{copy.headline}</DisplayHeading>
            <LeadText className="mx-auto mt-3 max-w-[36rem] max-md:leading-[1.45] md:mt-5">{copy.sub}</LeadText>
          </SourcesHeadingFade>
          {/* WEB-5 T6: 19 sources instead of 9. On phones the pills, gaps and
              margins are tighter so the pinned 100svh stage still holds the
              headline, every pill and the caption; desktop is unchanged. */}
          <ul className="relative z-0 mx-auto mt-5 flex max-w-[760px] flex-wrap justify-center gap-1.5 md:mt-12 md:gap-3">
            <SourcePills items={copy.items} />
          </ul>
          <Reveal className="mx-auto mt-4 max-w-[36rem] md:mt-10">
            <p className="text-[11px] leading-4 md:text-[12px] md:leading-5 tracking-wide text-[#E8F4FF]/65">{copy.caption}</p>
          </Reveal>
        </Container>
      </div>
    </ProgressSection>
  );
}
