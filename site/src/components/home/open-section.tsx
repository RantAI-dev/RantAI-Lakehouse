import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { SplitWords } from "@/components/shared/reveal";
import { CodePanel, ConsoleHome, HostingSwitch, TablesDiagram } from "./open-visuals";
import { OpenShowcase } from "./open-showcase";

/**
 * S7 — "Yours to keep": open source, open tables, your servers or ours, and
 * the console that runs on them (WEB-5 D19), just before the section that
 * installs it (D20).
 *
 * WEB-5 O22 made each promise a picture of itself (`open-visuals.tsx`): the
 * Lakehouse's own code with its licence, one Iceberg table read by several
 * engines, a switch that moves the stack from a rack to a cloud, and the
 * console's home recreated in HTML.
 *
 * WEB-5 O38 (owner: "jangan lah ada dua bento grid gini"; governance keeps
 * the bento): a sticky showcase instead. From 1024 px the four promises are
 * a list on the left, about one screen of scroll each, and one large panel
 * on the right stays at the middle of the screen and swaps to the current
 * promise's picture (`open-showcase.tsx` marks it; CSS, `.lk-show`, does the
 * rest). Each list item is a link to its own block, so a click scrolls to
 * it. Below 1024 px nothing is sticky: title, line and picture, four times.
 * Without JavaScript the same sequence shows at every width (a <noscript>
 * style), and under reduced motion the swap has no fade.
 */

const NO_SCRIPT_CSS =
  "@media (min-width:64rem){.lk-show{grid-template-columns:minmax(0,1fr)!important;row-gap:16px!important}.lk-show-mark{display:none!important}.lk-show-item,.lk-show-visual{grid-column:auto!important;grid-row:auto!important;position:relative!important;top:auto!important;margin-top:0!important;height:auto!important}.lk-show-visual{opacity:1!important;visibility:visible!important;transform:none!important;margin-bottom:32px!important;max-width:40rem!important}.lk-show-item .lk-show-title,.lk-show-item .lk-show-line{color:inherit!important}}";

export function OpenSection({ copy, cta }: { copy: HomeCopy["open"]; cta: CommonCopy["cta"] }) {
  const [source, tables, servers] = copy.cards;
  const items = [
    {
      title: source.title,
      body: source.body,
      visual: (
        <CodePanel
          copy={copy.code}
          action={
            <CtaLink href={LINKS.source} variant="ghost" size="sm" target="_blank" className="bg-[#0a1230]">
              {cta.source}
              <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 motion-reduce:transition-none" />
            </CtaLink>
          }
        />
      ),
    },
    { title: tables.title, body: tables.body, visual: <TablesDiagram copy={copy.tables} /> },
    { title: servers.title, body: servers.body, visual: <HostingSwitch copy={copy.hosting} /> },
    {
      title: copy.console.label,
      body: copy.console.caption,
      visual: (
        <figure className="h-full">
          <figcaption className="sr-only">{copy.console.alt}</figcaption>
          <div aria-hidden="true" className="h-full">
            <ConsoleHome windowTitle={copy.console.window} />
          </div>
        </figure>
      ),
    },
  ];
  return (
    <section id="open" className="lk-fallback-bg relative scroll-mt-16 bg-[#050A30] py-28 md:py-36">
      <noscript dangerouslySetInnerHTML={{ __html: `<style>${NO_SCRIPT_CSS}</style>` }} />
      <Container>
        <DisplayHeading>
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <OpenShowcase>
          <div className="lk-show mt-12 md:mt-16" data-active="0">
            {items.map((item, i) => (
              <React.Fragment key={item.title}>
                {/* The scroll marker for this promise: one screen tall, from 1024 px only. */}
                <div id={`open-${i + 1}`} className="lk-show-mark" data-i={i} aria-hidden="true" />
                <div
                  className="lk-show-item"
                  data-i={i}
                  data-on={i === 0 ? "" : undefined}
                  style={{ "--i": i } as React.CSSProperties}
                >
                  <a href={`#open-${i + 1}`} data-show={i} className="group block rounded-md focus-visible:outline-offset-4">
                    <h3 className="lk-show-title text-[22px] leading-8 font-medium text-balance text-[#E8F4FF] md:text-[24px]">
                      {item.title}
                    </h3>
                    <p className="lk-show-line mt-1.5 max-w-[28rem] text-[15px] leading-[1.6] text-pretty text-[#E8F4FF]/75">
                      {item.body}
                    </p>
                  </a>
                </div>
                <div className="lk-show-visual" data-i={i} data-on={i === 0 ? "" : undefined}>
                  {item.visual}
                </div>
              </React.Fragment>
            ))}
          </div>
        </OpenShowcase>
      </Container>
    </section>
  );
}
