import * as React from "react";
import { BarChart3, SquareTerminal, Workflow } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { SplitWords } from "@/components/shared/reveal";
import { AssistantChat } from "./assistant-chat";

/**
 * S4 — "Ask, don't dig": the assistant, with a chat that plays once when it
 * comes into view.
 *
 * WEB-5 O19: the chat no longer follows the scroll. It was a 260 vh pinned
 * stage that typed and un-typed with every scroll step; now the section is
 * one screen tall and the panel tells its own story in time
 * (`assistant-chat.tsx`), then rests. The chat is DOM, not a screenshot, and
 * nothing in it talks to an API. The mock is decorative (`aria-hidden`); a
 * visually hidden sentence describes it (O32: no visible "Illustration"
 * tag). Without JavaScript and with reduced motion the finished
 * conversation is shown at once.
 *
 * A server component: the only client code is the panel's player.
 */

type AssistantCopy = HomeCopy["assistant"];

/** One icon per chip, in `assistant.features` order: charts, pipelines, Query Studio. */
const FEATURE_ICONS = [BarChart3, Workflow, SquareTerminal] as const;

export function AssistantSection({ copy }: { copy: AssistantCopy }) {
  return (
    <section id="assistant" className="lk-fallback-bg relative overflow-hidden bg-[#050A30]">
      <Caustics />
      <Container className="relative z-10 grid min-h-[100svh] items-center gap-10 py-24 md:grid-cols-[1fr_1.1fr] md:gap-14 md:py-28">
        <div>
          <DisplayHeading size="lg" className="max-md:text-[clamp(2.6rem,12vw,3.4rem)]">
            <SplitWords text={copy.headline} />
          </DisplayHeading>
          <LeadText className="mt-3 max-w-[30rem] max-md:text-[15px] max-md:leading-[1.5] md:mt-5">{copy.sub}</LeadText>
          {/* O19 follow-up, "lots of visuals, little text": what else it does, as chips, each one a claim. */}
          <ul className="mt-5 flex max-w-[32rem] flex-wrap gap-1.5 md:mt-6 md:gap-2">
            {copy.features.map((f, i) => {
              const Icon = FEATURE_ICONS[i] ?? BarChart3;
              return (
                <li
                  key={f}
                  className="inline-flex items-center gap-1.5 rounded-full border border-[#E8F4FF]/14 bg-[#E8F4FF]/[0.05] px-3 py-1 text-[12px] leading-5 text-[#E8F4FF]/80 transition-colors duration-200 hover:border-[#44A6FA]/45 hover:text-[#E8F4FF] md:text-[13px]"
                >
                  <Icon aria-hidden="true" className="size-3.5 shrink-0 text-[#8fcbff]" strokeWidth={2} />
                  {f}
                </li>
              );
            })}
          </ul>
        </div>
        <AssistantChat copy={copy} />
      </Container>
    </section>
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
