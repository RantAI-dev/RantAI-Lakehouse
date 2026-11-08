import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { GET_STARTED_COMMANDS, LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { CopyButton } from "@/components/shared/copy-button";
import { Reveal } from "@/components/shared/reveal";
import { GetStartedPlay } from "./get-started-play";

/**
 * S7b — "Get started" (WEB-5 D20): the install path on the page itself,
 * near the surface, after "Yours to keep" and before the final call. Three
 * steps from the README's Quickstart, each with its commands in a terminal
 * block (monospace is for terminals: DESIGN.md §4) and a copy button per
 * command; then the link on to the full Quickstart. "Install it" in the top
 * bar and the hero lands here.
 *
 * The steps hang from one line with a bubble at each, rising to the
 * surface the visitor is about to reach: the depth motif, not a generic
 * numbered list. A server section; only the copy buttons are client code.
 * The commands wrap on a phone (no sideways scroll), and the `$` prompt is
 * not selectable, so selecting a line by hand copies only the command.
 *
 * WEB-5 O23 + owner QA 2026-11-08 ("better to have a description, but no need
 * for the data below"): the terminals show only the commands, with a one-line
 * note under each. The line fills as the steps pass the middle of the screen
 * (a CSS scroll timeline; without one it is full), and each step's number
 * lights, over about 200 ms, when its block comes into view
 * (`get-started-play.tsx`). The server renders everything settled, so the
 * section is complete without JavaScript and under reduced motion.
 */
export function GetStartedSection({ copy }: Readonly<{ copy: HomeCopy["getStarted"] }>) {
  const copyText = { copy: copy.copy, copied: copy.copied, failed: copy.copyFailed };
  return (
    <section id="get-started" aria-labelledby="get-started-heading" className="lk-fallback-bg relative scroll-mt-16 bg-[var(--lk-n-bg)] py-24 md:py-32">
      <Container className="grid grid-cols-[minmax(0,1fr)] gap-12 md:grid-cols-[minmax(0,0.9fr)_minmax(0,1.3fr)] md:gap-16">
        {/* O9: centred in the screen while the steps pass (app/globals.css). */}
        <div className="lk-sticky-center-md">
          <Eyebrow>{copy.eyebrow}</Eyebrow>
          <DisplayHeading id="get-started-heading" size="md" className="mt-5">
            {copy.headline}
          </DisplayHeading>
          <LeadText className="mt-5">{copy.sub}</LeadText>
          <CtaLink href={LINKS.install} variant="ghost" target="_blank" className="mt-8 max-md:hidden">
            {copy.more}
            <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 motion-reduce:transition-none" />
          </CtaLink>
        </div>

        <GetStartedPlay>
        <ol className="lk-gs-steps relative">
          {/* The line the steps hang from: a dim track, and the fill that follows the scroll. Solid, no gradient (owner, O35 follow-up). */}
          <span aria-hidden="true" className="absolute top-3 bottom-8 left-[11px] w-px bg-[#44A6FA]/20" />
          <span aria-hidden="true" className="lk-gs-fill absolute top-3 bottom-8 left-[11px] w-px bg-[#8fcbff]/80" />
          {copy.steps.map((step, i) => (
            <Reveal as="li" key={step.title} delay={i * 0.08} y={20} className="relative min-w-0 pb-7 pl-12 last:pb-0">
              <span
                aria-hidden="true"
                className="lk-gs-num absolute top-1 left-0 grid size-6 place-items-center rounded-full bg-[var(--lk-n-bg)] text-[12px] font-medium tabular-nums"
              >
                {i + 1}
              </span>
              <h3 className="text-[18px] leading-7 font-medium text-[#E8F4FF]">{step.title}</h3>
              <div className="lk-gs-term mt-2 overflow-hidden rounded-xl bg-[var(--lk-n-surface)]">
                <ul className="divide-y divide-white/6">
                  {GET_STARTED_COMMANDS[i].map((cmd, j) => {
                    const id = `get-started-${i}-${j}`;
                    return (
                      <li key={cmd} className="flex items-center gap-1 py-0.5 pr-1 pl-4">
                        <code className="min-w-0 flex-1 overflow-x-auto py-2.5 pr-6 text-[13px] leading-5 whitespace-nowrap text-[#E8F4FF] [mask-image:linear-gradient(90deg,#000_calc(100%-24px),transparent)] [scrollbar-width:none] sm:text-[13.5px] [&::-webkit-scrollbar]:hidden" tabIndex={0}>
                          <span aria-hidden="true" className="mr-2 text-[#44A6FA] select-none">
                            $
                          </span>
                          <span id={id}>
                            {cmd}
                          </span>
                        </code>
                        <CopyButton targetId={id} text={copyText} className="h-10! min-w-10 justify-center bg-transparent px-2.5! ring-0! hover:bg-white/[0.08] [&[data-state=idle]>span:last-child]:sr-only" />
                      </li>
                    );
                  })}
                </ul>
              </div>
              <p className="mt-2.5 text-[14px] leading-6 text-pretty text-[#E8F4FF]/65">
                <Inline text={step.note} />
              </p>
            </Reveal>
          ))}
        </ol>
        </GetStartedPlay>

        <CtaLink
          href={LINKS.install}
          variant="ghost"
          target="_blank"
          className="justify-self-start max-md:h-auto max-md:py-3 max-md:text-left max-md:whitespace-normal md:hidden"
        >
          {copy.more}
          <ArrowUpRight aria-hidden="true" className="size-4" />
        </CtaLink>
      </Container>
    </section>
  );
}

/** A step's note: text between backticks is an inline code span. */
function Inline({ text }: Readonly<{ text: string }>) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 === 1 ? (
          <code key={i} className="rounded-md bg-white/[0.08] px-1.5 py-0.5 text-[12.5px] [overflow-wrap:anywhere] text-[#E8F4FF]">
            {part}
          </code>
        ) : (
          <React.Fragment key={i}>{part}</React.Fragment>
        ),
      )}
    </>
  );
}
