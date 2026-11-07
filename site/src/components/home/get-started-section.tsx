import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { GET_STARTED_COMMANDS, LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { CopyButton } from "@/components/shared/copy-button";
import { Reveal } from "@/components/shared/reveal";

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
 */
export function GetStartedSection({ copy }: Readonly<{ copy: HomeCopy["getStarted"] }>) {
  const copyText = { copy: copy.copy, copied: copy.copied, failed: copy.copyFailed };
  return (
    <section id="get-started" aria-labelledby="get-started-heading" className="lk-fallback-bg relative scroll-mt-16 bg-[#050A30] py-24 md:py-32">
      <Container className="grid gap-12 md:grid-cols-[minmax(0,0.9fr)_minmax(0,1.3fr)] md:gap-16">
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

        <ol className="relative">
          {/* The line the steps hang from, fading out at the bottom toward the surface. */}
          <span aria-hidden="true" className="absolute top-3 bottom-8 left-[11px] w-px bg-gradient-to-b from-[#44A6FA]/60 via-[#44A6FA]/25 to-transparent" />
          {copy.steps.map((step, i) => (
            <Reveal as="li" key={step.title} delay={i * 0.08} y={20} className="relative pb-10 pl-12 last:pb-0">
              <span
                aria-hidden="true"
                className="absolute top-1 left-0 grid size-6 place-items-center rounded-full bg-[#020514] text-[12px] font-medium text-[#8fcbff] tabular-nums ring-1 ring-[#44A6FA]/60 shadow-[0_0_14px_2px_rgba(68,166,250,0.35)]"
              >
                {i + 1}
              </span>
              <h3 className="text-[19px] leading-7 font-medium text-[#E8F4FF]">{step.title}</h3>
              <div className="mt-3 overflow-hidden rounded-2xl bg-[#020514]/80 ring-1 ring-[#E8F4FF]/12 shadow-[0_24px_60px_-30px_rgba(2,5,20,0.9)]">
                <p className="border-b border-[#E8F4FF]/8 px-4 py-2 text-[11px] leading-4 font-medium tracking-[0.14em] text-[#E8F4FF]/50 uppercase">
                  {copy.terminal}
                </p>
                <ul className="divide-y divide-[#E8F4FF]/6">
                  {GET_STARTED_COMMANDS[i].map((cmd, j) => {
                    const id = `get-started-${i}-${j}`;
                    return (
                      <li key={cmd} className="flex items-start gap-3 px-4 py-3">
                        <code className="min-w-0 flex-1 pt-1.5 text-[13px] leading-5 break-all whitespace-pre-wrap text-[#E8F4FF] sm:text-[14px]">
                          <span aria-hidden="true" className="mr-2 text-[#44A6FA] select-none">
                            $
                          </span>
                          <span id={id}>{cmd}</span>
                        </code>
                        <CopyButton targetId={id} text={copyText} />
                      </li>
                    );
                  })}
                </ul>
              </div>
              <p className="mt-3 max-w-[36rem] text-[15px] leading-[1.6] text-pretty text-[#E8F4FF]/75">
                <Inline text={step.note} />
              </p>
            </Reveal>
          ))}
        </ol>

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

/** A message with `code` spans: the file and variable names a reader types. */
function Inline({ text }: Readonly<{ text: string }>) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 === 1 ? (
          <code key={i} className="rounded-md bg-[#E8F4FF]/[0.08] px-1.5 py-0.5 text-[13px] whitespace-nowrap text-[#E8F4FF]">
            {part}
          </code>
        ) : (
          <React.Fragment key={i}>{part}</React.Fragment>
        ),
      )}
    </>
  );
}
