import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { ProgressSection } from "@/components/shared/progress-section";
import { SplitWords } from "@/components/shared/reveal";
import { AssistantChat } from "./assistant-chat";

/**
 * S4 — "Ask, don't dig": the assistant, with a scripted chat that scrolling
 * plays and un-plays.
 *
 * The chat is DOM, not a screenshot, and every step is a pure function of
 * scroll progress, so scrolling back rewinds it: the typing, the thinking
 * shimmer, the answer with its bar chart, the second question, and the
 * approval card. Nothing here talks to an API. The mock is decorative
 * (`aria-hidden`); a visually hidden sentence describes it, and it is
 * labelled "Illustration" on screen. With reduced motion the finished
 * conversation is shown at once.
 *
 * WEB-5 review fix R5: a server component; the chat is the client leaf in
 * `assistant-chat.tsx`.
 */

type AssistantCopy = HomeCopy["assistant"];

export function AssistantSection({ copy }: { copy: AssistantCopy }) {
  return (
    <ProgressSection range={["start start", "end end"]} id="assistant" className="lk-fallback-bg relative h-[260vh] bg-[#050A30] motion-reduce:h-auto">
      <div className="sticky top-0 flex h-[100svh] items-center overflow-hidden motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
        <Caustics />
        {/* WEB-5 D14 S4: tighter phone spacing (gap-4, the lead at mt-3, and the
            chat's own spacing in assistant-chat.tsx) so the pinned stage fits a
            360x740 screen, where it overflowed by up to 45 px; desktop is unchanged. */}
        <Container className="relative z-10 grid items-center gap-4 pt-16 md:grid-cols-[1fr_1.1fr] md:gap-14 md:pt-0">
          <div>
            <DisplayHeading size="lg" className="max-md:text-[clamp(2.6rem,12vw,3.4rem)]">
              <SplitWords text={copy.headline} />
            </DisplayHeading>
            <LeadText className="mt-3 max-w-[32rem] max-md:text-[15px] max-md:leading-[1.5] md:mt-5">
              {copy.sub}
            </LeadText>
          </div>
          <AssistantChat copy={copy} />
        </Container>
      </div>
    </ProgressSection>
  );
}

/** Two slow radial lights, like the pattern the sun throws on a pool floor. Off under reduced motion. */
function Caustics() {
  return (
    <div
      aria-hidden="true"
      // WEB-5 D14 S2: the glow fades in over the stage's first quarter and out
      // over its last, so it starts and ends softly on the page-wide water
      // instead of at the stage's edges.
      className="absolute inset-0 overflow-hidden [mask-image:linear-gradient(to_bottom,transparent,#000_25%,#000_75%,transparent)]"
    >
      <div className="lk-fallback-bg absolute inset-0 bg-[radial-gradient(ellipse_at_50%_0%,#0B1A4A_0%,#050A30_55%,#030824_100%)]" />

      <div className="lk-caustic-a absolute -top-1/4 -left-1/4 size-[80%] rounded-full bg-[radial-gradient(circle,rgba(68,166,250,0.20)_0%,transparent_62%)] blur-2xl" />
      <div className="lk-caustic-b absolute -right-1/4 -bottom-1/3 size-[90%] rounded-full bg-[radial-gradient(circle,rgba(68,166,250,0.14)_0%,transparent_60%)] blur-3xl" />
      {/* WEB-1 review round 1 SHOULD-FIX 2: meets the abyss the canvas fades to at
          the end of S3 with no seam. Last, so the glow blobs above cannot show a
          clipped edge at the top of the stage. */}
      {/* WEB-5 D14 S2: with the page-wide water this join is no longer needed, so it is part of the no-JS fallback only. */}
      <div className="lk-fallback-bg absolute inset-x-0 top-0 h-[50%] bg-gradient-to-b from-[#020514] via-[#020514]/60 to-transparent" />
    </div>
  );
}
