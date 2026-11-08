import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { SplitWords } from "@/components/shared/reveal";
import { CodePanel, ConsoleHome } from "./open-visuals";
import { HostingStack } from "./hosting-stack";
import { TablesAnatomy } from "./tables-anatomy";
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
  "@media (min-width:64rem){.lk-show{grid-template-columns:minmax(0,1fr)!important;grid-template-rows:none!important;row-gap:16px!important}.lk-show-mark{display:none!important}.lk-show-pin,.lk-show-cols,.lk-show-list,.lk-show-stage{display:contents!important}.lk-show-visual{opacity:1!important;visibility:visible!important;transform:none!important;height:auto!important;min-height:260px!important;max-width:40rem!important;margin-bottom:32px!important;grid-area:auto!important}.lk-show-item .lk-show-title,.lk-show-item .lk-show-line{color:inherit!important}.lk-show-item{border-left-color:rgba(232,244,255,.1)!important}}";

export function OpenSection({ copy, cta, contactHref }: { copy: HomeCopy["open"]; cta: CommonCopy["cta"]; contactHref: string }) {
  const [source, tables, servers] = copy.cards;
  const items = [
    {
      title: source.title,
      body: source.body,
      visual: (
        <CodePanel
          copy={copy.code}
          action={
            <CtaLink href={LINKS.source} variant="ghost" size="sm" target="_blank" className="bg-[var(--lk-n-surface)]">
              {cta.source}
              <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 motion-reduce:transition-none" />
            </CtaLink>
          }
        />
      ),
    },
    { title: tables.title, body: tables.body, visual: <TablesAnatomy copy={copy.tables} /> },
    { title: servers.title, body: servers.body, visual: <HostingStack copy={copy.hosting} contactHref={contactHref} /> },
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
    // WEB-5 audit #19: less padding above and below (the gaps to governance and Get started).
    <section id="open" className="lk-fallback-bg relative scroll-mt-16 bg-[var(--lk-n-bg)] pt-16 pb-20 md:pt-20 md:pb-24">
      <noscript dangerouslySetInnerHTML={{ __html: `<style>${NO_SCRIPT_CSS}</style>` }} />
      <Container>
        <OpenShowcase>
          {/* WEB-5 O44 (owner: the four titles drawn on top of each other): the
              list is ONE sticky box whose items flow one under another, and
              the pictures are ONE sticky box whose pictures stack. Both are
              held for the four scroll markers and released together. Before,
              each item was sticky on its own in the same grid area, and at
              the release the area's end pushed all four into one spot. On
              phones (and without JS, and in a short window) both boxes are
              `display: contents`, so items and pictures interleave by
              `order`: title, line, picture, four times. */}
          <div className="lk-show mt-12 md:mt-0" data-active="0">
            {items.map((item, i) => (
              // The scroll marker for each promise: one screen tall, from 1024 px only.
              <div key={item.title} id={`open-${i + 1}`} className="lk-show-mark" data-i={i} aria-hidden="true" />
            ))}
            {/* Owner QA: from 1024 px the heading, the list and the panel are ONE sticky unit, held at the
                middle of the screen, so the title never scrolls away while the promises swap. */}
            <div className="lk-show-pin">
            <div className="lk-show-head">
              <DisplayHeading>
                <SplitWords text={copy.headline} />
              </DisplayHeading>
            </div>
            <div className="lk-show-cols">
            <ol className="lk-show-list">
              {items.map((item, i) => (
                <li key={item.title} className="lk-show-item" data-i={i} data-on={i === 0 ? "" : undefined}>
                  <a href={`#open-${i + 1}`} data-show={i} className="group block rounded-md focus-visible:outline-offset-4">
                    <h3 className="lk-show-title text-[22px] leading-8 font-medium text-balance text-[#E8F4FF] md:text-[24px]">
                      {item.title}
                    </h3>
                    <p className="lk-show-line mt-1.5 max-w-[28rem] text-[15px] leading-[1.6] text-pretty text-white/75">
                      {item.body}
                    </p>
                  </a>
                </li>
              ))}
            </ol>
            <div className="lk-show-stage">
              {items.map((item, i) => (
                <div key={item.title} className="lk-show-visual" data-i={i} data-on={i === 0 ? "" : undefined}>
                  {item.visual}
                </div>
              ))}
            </div>
            </div>
            </div>
          </div>
        </OpenShowcase>
      </Container>
    </section>
  );
}
