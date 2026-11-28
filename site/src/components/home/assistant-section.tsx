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
 *
 * O35 (owner: no gradients, "mending warna solid aja"): the two drifting
 * radial lights behind the stage are gone; the section sits on the
 * page-wide water, with its solid colour as the no-JS fallback.
 */

type AssistantCopy = HomeCopy["assistant"];

/** One icon per chip, in `assistant.features` order: charts, pipelines, Query Studio. */
const FEATURE_ICONS = [BarChart3, Workflow, SquareTerminal] as const;

export function AssistantSection({ copy }: { copy: AssistantCopy }) {
  return (
    // WEB-5 audit #19: with JavaScript and motion, it comes up 30 vh under the depths' leaving stage
    // (its own top is padding, and the stage's card is gone above by then), so the hand-over is not
    // a screen of empty water. Without JavaScript its background is opaque, so it keeps its place.
    <section
      id="assistant"
      className="lk-fallback-bg relative overflow-hidden bg-[var(--lk-n-bg)] motion-safe:[html[data-scene]_&]:-mt-[30vh]"
    >
      <Container className="relative z-10 grid min-h-[100svh] items-center gap-10 py-24 max-md:pb-12 md:grid-cols-[1fr_1.1fr] md:gap-14 md:py-28">
        <div>
          <DisplayHeading size="lg" className="max-md:text-[clamp(2.2rem,10vw,2.8rem)]">
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
                  className="inline-flex items-center gap-1.5 rounded-full bg-[#E8F4FF]/[0.07] px-3 py-1 text-[12px] leading-5 text-[#E8F4FF]/80 transition-colors duration-200 hover:bg-[#E8F4FF]/[0.12] hover:text-[#E8F4FF] md:text-[13px]"
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
