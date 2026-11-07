import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { ProductFrame } from "@/components/shared/product-frame";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { TiltCard } from "@/components/shared/tilt-card";

/**
 * S7 — "Yours to keep": open source, open tables, your servers or ours.
 * WEB-5 D19: then the console itself, the real thing that runs on those
 * servers, just before the section that installs it (D20).
 */
export function OpenSection({ copy, cta }: { copy: HomeCopy["open"]; cta: CommonCopy["cta"] }) {
  return (
    <section id="open" className="lk-fallback-bg relative scroll-mt-16 bg-[#050A30] py-28 md:py-36">
      <Container>
        <DisplayHeading>
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <ul className="mt-14 grid gap-5 md:grid-cols-3">
          {copy.cards.map((c, i) => (
            <Reveal as="li" key={c.title} delay={i * 0.12} y={44} className="[perspective:1000px]">
              <TiltCard className="h-full rounded-3xl border border-[#E8F4FF]/12 bg-[linear-gradient(160deg,rgba(232,244,255,0.07),rgba(232,244,255,0.02))] p-8 shadow-[0_30px_80px_-40px_rgba(2,5,20,0.9)]">
                <span className="lk-display lk-outline-text block text-[96px] leading-[0.9]" aria-hidden="true">
                  {String(i + 1).padStart(2, "0")}
                </span>
                <h3 className="mt-7 text-[20px] leading-7 font-medium text-[#E8F4FF]">{c.title}</h3>
                <p className="mt-2 text-[16px] leading-[1.6] text-[#E8F4FF]/80">{c.body}</p>
              </TiltCard>
            </Reveal>
          ))}
        </ul>
        {/* WEB-5 D19: the console beside its words from md up; on phones the words come first. */}
        <figure className="mt-16 grid items-center gap-8 md:mt-20 md:grid-cols-[minmax(0,1.45fr)_minmax(0,1fr)] md:gap-12">
          <Reveal y={36} className="md:order-1">
            <ProductFrame
              src="/product/console-home.webp"
              width={676}
              height={156}
              alt={copy.console.alt}
              windowTitle={copy.console.window}
              narrow={{ src: "/product/console-home-narrow.webp", width: 420, height: 156 }}
            />
          </Reveal>
          {/* <figcaption> must be the figure's own child, so the reveal sits inside it. */}
          <figcaption className="max-md:-order-1 md:order-2">
            <Reveal delay={0.12}>
              <h3 className="text-[20px] leading-7 font-medium text-balance text-[#E8F4FF]">{copy.console.label}</h3>
              <p className="mt-2 text-[16px] leading-[1.6] text-pretty text-[#E8F4FF]/75">{copy.console.caption}</p>
              <CtaLink href={LINKS.source} variant="ghost" target="_blank" className="mt-7">
                {cta.source}
                <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 motion-reduce:transition-none" />
              </CtaLink>
            </Reveal>
          </figcaption>
        </figure>
      </Container>
    </section>
  );
}
