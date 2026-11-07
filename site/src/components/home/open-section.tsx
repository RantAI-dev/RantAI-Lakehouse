import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { cn } from "@/lib/utils";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { ProductFrame } from "@/components/shared/product-frame";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { CodePanel, ConsoleHome, HostingSwitch, TablesDiagram } from "./open-visuals";

/**
 * S7 — "Yours to keep": open source, open tables, your servers or ours, and
 * the console that runs on them (WEB-5 D19), just before the section that
 * installs it (D20).
 *
 * WEB-5 O22 (owner: "masih kurang suka sama ui ini juga, harus kita explore
 * lagi deh"): the three equal cards with outline numerals and the bright,
 * soft crop of the console were a generic template that told rather than
 * showed. Now a bento where each promise is a picture of itself
 * (`open-visuals.tsx`):
 * - open source: the Lakehouse's own code, a real function, with its
 *   licence, and "Read the source";
 * - open tables: one Iceberg table, read by several engines at once;
 * - your servers or ours: a switch that moves the same stack from a rack to
 *   a cloud;
 * - the console's home, recreated in HTML (crisp, and in its dark theme), in
 *   the site's window frame.
 * Two rows that end flush from 1024 px up (3 + 3 and 2 + 4 of six columns),
 * two columns on a tablet, one on a phone. A server section: the only
 * client code is the reveals it already had; the switch is radio buttons
 * and every loop is CSS.
 */
export function OpenSection({ copy, cta }: { copy: HomeCopy["open"]; cta: CommonCopy["cta"] }) {
  const [source, tables, servers] = copy.cards;
  return (
    <section id="open" className="lk-fallback-bg relative scroll-mt-16 bg-[#050A30] py-28 md:py-36">
      <Container>
        <DisplayHeading>
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <ul className="mt-12 grid grid-cols-[minmax(0,1fr)] gap-4 md:mt-14 md:grid-cols-2 lg:grid-cols-6 lg:gap-5">
          <Tile
            className="lg:col-span-3"
            delay={0}
            title={source.title}
            body={source.body}
            visual={
              <CodePanel
                copy={copy.code}
                action={
                  <CtaLink href={LINKS.source} variant="ghost" size="sm" target="_blank" className="bg-[#020514]/85 backdrop-blur-sm">
                    {cta.source}
                    <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 motion-reduce:transition-none" />
                  </CtaLink>
                }
              />
            }
          />
          <Tile className="lg:col-span-3" delay={0.08} title={tables.title} body={tables.body} visual={<TablesDiagram copy={copy.tables} />} />
          <Tile className="lg:col-span-2" delay={0.16} title={servers.title} body={servers.body} visual={<HostingSwitch copy={copy.hosting} />} />
          <Tile
            className="lg:col-span-4"
            delay={0.24}
            title={copy.console.label}
            body={copy.console.caption}
            visual={
              <figure className="h-full">
                <figcaption className="sr-only">{copy.console.alt}</figcaption>
                <div aria-hidden="true" className="h-full">
                  <ProductFrame windowTitle={copy.console.window} className="lk-open-frame h-full">
                    <ConsoleHome />
                  </ProductFrame>
                </div>
              </figure>
            }
          />
        </ul>
      </Container>
    </section>
  );
}

/** One bento tile: the picture first, then a short title and one line. */
function Tile({
  title,
  body,
  visual,
  delay,
  className,
}: {
  title: string;
  body: string;
  visual: React.ReactNode;
  delay: number;
  className?: string;
}) {
  return (
    <Reveal as="li" delay={delay} y={32} className={cn("lk-open-tile flex min-w-0 flex-col rounded-3xl p-3 md:p-4", className)}>
      <div className="lk-open-visual min-h-0 flex-1">{visual}</div>
      {/* Room for a title and two lines on every tile, so the titles of a row line up. */}
      <div className="px-2 pt-5 pb-2 md:min-h-[6.8rem] md:px-3">
        <h3 className="text-[20px] leading-7 font-medium text-balance text-[#E8F4FF]">{title}</h3>
        <p className="mt-1.5 max-w-[34rem] text-[15px] leading-[1.55] text-pretty text-[#E8F4FF]/75">{body}</p>
      </div>
    </Reveal>
  );
}
