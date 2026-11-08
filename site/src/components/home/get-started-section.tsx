import * as React from "react";
import { ArrowUpRight, Check } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { GET_STARTED_COMMANDS, LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { CopyButton } from "@/components/shared/copy-button";
import { Reveal } from "@/components/shared/reveal";
import { GetStartedPlay } from "./get-started-play";
import { GET_STARTED_OUTPUT } from "./get-started-output";

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
 * WEB-5 O23 (owner: "udah bagus cuma kek kurang animasi"): the same layout,
 * alive. The line fills as the steps pass the middle of the screen and
 * each step's number lights there (CSS scroll timelines; where a browser
 * has none, the line is full and the numbers lit). When a terminal comes
 * into view, what its commands print appears, dimmed, line by line, with a
 * green check on the line that says it worked (`get-started-output.ts`),
 * and the last one ends ready on localhost:3000. The commands themselves
 * show at once (O45 rollback, audit #5: typed one terminal at a time,
 * steps 2 and 3 sat blank). The server renders every terminal finished, so
 * the section is complete without JavaScript and under reduced motion.
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
              <div data-term="" className="lk-gs-term mt-2 overflow-hidden rounded-xl bg-[var(--lk-n-surface)]">
                <ul className="divide-y divide-white/6">
                  {GET_STARTED_COMMANDS[i].map((cmd, j) => {
                    const id = `get-started-${i}-${j}`;
                    return (
                      <li key={cmd} data-cmd className="flex items-center gap-1 py-0.5 pr-1 pl-4">
                        <code className="min-w-0 flex-1 overflow-x-auto py-2.5 pr-6 text-[13px] leading-5 whitespace-nowrap text-[#E8F4FF] [mask-image:linear-gradient(90deg,#000_calc(100%-24px),transparent)] [scrollbar-width:none] sm:text-[13.5px] [&::-webkit-scrollbar]:hidden" tabIndex={0}>
                          <span aria-hidden="true" className="mr-2 text-[#44A6FA] select-none">
                            $
                          </span>
                          <span id={id} data-real className="lk-gs-real">
                            {cmd}
                          </span>
                        </code>
                        <CopyButton targetId={id} text={copyText} className="h-10! min-w-10 justify-center bg-transparent px-2.5! ring-0! hover:bg-white/[0.08] [&[data-state=idle]>span:last-child]:sr-only" />
                      </li>
                    );
                  })}
                </ul>
                {/* What the commands print, dimmed as output. */}
                <div aria-hidden="true" className="lk-gs-out border-t border-white/6 px-4 py-2.5">
                  {GET_STARTED_OUTPUT[i].map((line) => (
                    <p
                      key={line.text}
                      className="lk-gs-line flex gap-2 font-mono text-[12px] leading-5 whitespace-pre-wrap text-white/55 [overflow-wrap:anywhere]"
                    >
                      {!line.comment && <span className={line.ok ? "text-[#7be0b0]" : "text-transparent"}>✓</span>}
                      <span className={line.comment ? "min-w-0 text-white/40" : "min-w-0"}>{line.text}</span>
                    </p>
                  ))}
                  {i === copy.steps.length - 1 && (
                    <p className="lk-gs-ready mt-3 inline-flex items-center gap-2 rounded-full bg-[#7be0b0]/10 px-3 py-1 text-[12px] leading-5 font-medium text-[#7be0b0]">
                      <Check aria-hidden="true" className="size-3.5" strokeWidth={2.5} />
                      {copy.ready}
                    </p>
                  )}
                </div>
              </div>
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
