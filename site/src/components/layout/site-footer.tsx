import * as React from "react";
import { ArrowUp, ArrowUpRight } from "lucide-react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { CONTACT_EMAIL, FOOTER_SECTIONS, LINKS, RANTAI_PRODUCTS, SOCIAL_LINKS } from "@/lib/home-content";
import { CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { SocialIcon } from "@/components/shared/social-icons";
import { cn } from "@/lib/utils";
import { BackToTop } from "./back-to-top";
import { FooterPrompt } from "./footer-prompt";

/**
 * S9 — the footer, on rantai-page's footer structure in the lake's look
 * (WEB-5 owner QA O7, `components/layout/footer.tsx`, `footer-motion.tsx`
 * and `footer-prompt.tsx`), so every product site has the same footer:
 * 1. a rounded call-to-action card with a typewriter prompt that opens an
 *    e-mail (on the contact page, a pointer back to the form);
 * 2. the links: the mark, tagline and "Follow RantAI" (D17), then Product,
 *    Connect, and the RantAI products with their official marks, this one
 *    marked as the current site;
 * 3. a bottom bar: the legal line, the address and Back to top;
 * 4. a giant faded "Lakehouse" along the bottom edge.
 *
 * Shared by the home page and the contact page (D15); on the contact page
 * the section links lead back to the home page's sections. A server
 * component: every hover, press and focus state is CSS (DESIGN.md §3), so
 * the footer adds no JavaScript.
 */

type FooterLink = { label: string; href: string; external?: boolean; logo?: React.ReactNode; current?: boolean };

/** Hover: the text brightens and a hairline grows under it from the left (rantai-page's link underline). */
const linkClass =
  "group inline-flex min-h-10 items-center gap-2 text-[15px] leading-6 text-[#E8F4FF]/80 transition-[color,background-size] duration-200 ease-out hover:text-[#E8F4FF] active:text-[#44A6FA] md:min-h-0 bg-[linear-gradient(currentColor,currentColor)] bg-[length:0%_1px] bg-[position:0_100%] bg-no-repeat hover:bg-[length:100%_1px] motion-reduce:transition-none";

export function SiteFooter({
  locale,
  page = "home",
  copy,
}: {
  locale: Locale;
  page?: "home" | "contact";
  copy: CommonCopy["footer"];
}) {
  const home = localizedPath(locale, "/");
  const columns: { title: string; links: FooterLink[] }[] = [
    {
      title: copy.columns.product,
      links: FOOTER_SECTIONS.map((s) => ({
        label: copy.sections[s.key],
        href: page === "home" ? s.href : `${home}${s.href}`,
      })),
    },
    {
      title: copy.columns.connect,
      links: [
        // D15: the contact page, in the visitor's language; the address itself is in the legal line.
        { label: copy.contact, href: localizedPath(locale, "/contact") },
        { label: copy.userGuide, href: LINKS.guide, external: true },
        { label: "GitHub", href: LINKS.source, external: true },
      ],
    },
    {
      title: copy.columns.products,
      links: [
        ...RANTAI_PRODUCTS.map((p) =>
          p.href
            ? {
                label: p.label,
                href: p.href,
                external: true,
                // Decorative: the product's name is the link text.
                // eslint-disable-next-line @next/next/no-img-element -- a static SVG mark; the export has no image optimizer
                logo: <img src={p.logo} alt="" width={Math.round(14 * p.ratio)} height={14} loading="lazy" decoding="async" className="h-3.5 w-auto shrink-0" />,
              }
            : {
                label: p.label,
                href: page === "home" ? "#hero" : home,
                current: true,
                logo: <MascotMark className="h-3.5 w-auto shrink-0" />,
              },
        ),
        { label: "rantai.dev", href: LINKS.rantai, external: true },
      ],
    },
  ];

  const cta = copy.cta;
  return (
    <footer className="flex flex-col gap-3 bg-[#020514] px-3 pb-3 sm:gap-4 sm:px-4 sm:pb-4">
      {/* 1. The call to action, as a card (rantai-page's FooterCtaCard). On
          the home page it is the action after the story's end (the final
          section); on the contact page it points back up to the form
          instead of repeating it. */}
      <section
        aria-labelledby="footer-cta-heading"
        className="relative isolate overflow-hidden rounded-[28px] bg-[linear-gradient(180deg,#050A30_0%,#0A1F55_62%,#0b2a66_100%)] ring-1 ring-[#E8F4FF]/10"
      >
        <FooterWater />
        <div className="mx-auto flex max-w-3xl flex-col items-center px-6 py-16 text-center max-sm:items-start max-sm:px-6 max-sm:py-14 max-sm:text-left sm:py-28">
          <Eyebrow>{cta.eyebrow}</Eyebrow>
          <DisplayHeading id="footer-cta-heading" size="md" className="mt-5 max-w-[18ch]">
            <SplitWords text={page === "home" ? cta.title : cta.contactTitle} />
          </DisplayHeading>
          <LeadText className="mt-5 max-w-xl max-sm:text-[16px]">{page === "home" ? cta.line : cta.contactLine}</LeadText>
          {page === "home" ? (
            <>
              <Reveal className="mt-10 w-full max-sm:mt-8" delay={0.15} y={16}>
                <FooterPrompt text={cta} to={CONTACT_EMAIL} />
              </Reveal>
              <a
                href="#get-started"
                className="group mt-8 inline-flex min-h-10 items-center gap-1.5 text-[15px] text-[#E8F4FF]/70 transition-colors duration-200 ease-out hover:text-[#E8F4FF]"
              >
                {cta.install}
                <ArrowUp aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:-translate-y-0.5 motion-reduce:transition-none" />
              </a>
            </>
          ) : (
            <CtaLink href="#contact-heading" variant="ghost" className="mt-8">
              <ArrowUp aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:-translate-y-0.5 motion-reduce:transition-none" />
              {cta.backToForm}
            </CtaLink>
          )}
        </div>
      </section>

      {/* 2. The links: the mark, tagline and socials at left; three columns at right. */}
      <div className="relative overflow-hidden px-3 pt-14 sm:px-8 lg:px-14 lg:pt-20">
        <div className="mx-auto grid max-w-[1312px] gap-12 xl:grid-cols-[minmax(0,1fr)_auto] xl:gap-20">
          <div className="max-w-sm">
            <a
              href={page === "home" ? "#hero" : home}
              aria-label={copy.home}
              className="inline-flex items-center gap-2.5 rounded-lg text-[17px] font-medium tracking-tight text-[#E8F4FF] transition-opacity duration-200 ease-out hover:opacity-80"
            >
              <MascotMark className="h-6 w-auto" />
              RantAI Lakehouse
            </a>
            <p className="mt-4 text-[15px] leading-6 text-pretty text-[#E8F4FF]/70">{copy.tagline}</p>
            <p className="mt-6 text-[12px] font-medium tracking-[0.04em] text-[#E8F4FF]/60">{copy.follow}</p>
            <ul className="mt-2 -ml-2.5 flex items-center gap-1">
              {SOCIAL_LINKS.map((s) => (
                <li key={s.id}>
                  <a
                    href={s.href}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label={s.label}
                    className="grid size-10 place-items-center rounded-full text-[#E8F4FF]/65 transition-[color,background-color,transform] duration-200 ease-out hover:-translate-y-0.5 hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:translate-y-0 active:scale-95 motion-reduce:transition-none motion-reduce:hover:translate-y-0"
                  >
                    <SocialIcon id={s.id} />
                  </a>
                </li>
              ))}
            </ul>
          </div>
          {/* Phones: Product and Connect side by side, the products across. */}
          <div className="grid grid-cols-2 gap-x-6 gap-y-10 sm:grid-cols-3 sm:gap-x-10 lg:gap-x-16">
            {columns.map((col) => (
              <nav
                key={col.title}
                aria-label={col.title}
                className={cn(col.title === copy.columns.products && "max-sm:col-span-2")}
              >
                <h2 className="text-[11px] leading-4 font-medium tracking-[0.2em] text-[#E8F4FF]/60 uppercase">{col.title}</h2>
                <ul className="mt-3 sm:mt-4 sm:space-y-2.5">
                  {col.links.map((l) => (
                    <li key={l.href + l.label}>
                      <a
                        href={l.href}
                        className={cn(linkClass, l.current && "text-[#E8F4FF]")}
                        aria-current={l.current && page === "home" ? "page" : undefined}
                        {...(l.external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
                      >
                        {l.logo}
                        {l.label}
                        {l.current && (
                          <span className="rounded-full bg-[#44A6FA]/15 px-2 py-0.5 text-[11px] leading-4 font-medium text-[#8fcbff]">
                            {copy.current}
                          </span>
                        )}
                        {l.external && (
                          <ArrowUpRight
                            aria-hidden="true"
                            className="size-3.5 shrink-0 opacity-45 transition-[translate,opacity] duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 group-hover:opacity-100 motion-reduce:transition-none"
                          />
                        )}
                      </a>
                    </li>
                  ))}
                </ul>
              </nav>
            ))}
          </div>
        </div>

        {/* 3. The bottom bar: the legal line; the address and Back to top. */}
        <div className="mx-auto mt-12 flex max-w-[1312px] flex-col gap-2 border-t border-[#E8F4FF]/10 pt-6 sm:mt-16 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
          <p className="text-[13px] text-[#E8F4FF]/60">{copy.legal}</p>
          <div className="flex items-center justify-between gap-x-6 sm:justify-start">
            <a href={LINKS.email} className={cn(linkClass, "text-[13px] text-[#E8F4FF]/70")}>
              {CONTACT_EMAIL}
            </a>
            <BackToTop label={copy.backToTop} />
          </div>
        </div>

        {/* 4. The giant faded wordmark along the bottom edge (rantai-page's FooterWordmark). */}
        {/* Static: cropped by the page's end, too little of it is ever in view for a reveal to fire. */}
        <div className="pointer-events-none mt-4 -mb-[0.22em] select-none">
          <p
            aria-hidden="true"
            className="bg-[linear-gradient(180deg,rgba(232,244,255,0.1),rgba(232,244,255,0))] bg-clip-text text-center text-[clamp(3.5rem,15.5vw,15.5rem)] leading-[0.85] font-semibold tracking-[-0.06em] whitespace-nowrap text-transparent"
          >
            Lakehouse
          </p>
        </div>
      </div>
    </footer>
  );
}

/**
 * The card's water, the lake's version of rantai-page's ASCII wave: a
 * waterline across the lower third with the moon's glow on it and two rings
 * spreading slowly from where the creature would surface. Decorative; the
 * rings stop under reduced motion (`.lk-ring`).
 */
function FooterWater() {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-0 -z-10">
      <div className="absolute top-[8%] left-1/2 size-[50vmin] -translate-x-1/2 rounded-full bg-[radial-gradient(circle,rgba(232,244,255,0.12)_0%,rgba(68,166,250,0.06)_40%,transparent_70%)]" />
      <div className="absolute inset-x-0 bottom-[22%] h-px bg-gradient-to-r from-transparent via-[#44A6FA]/50 to-transparent" />
      <div className="absolute inset-x-0 bottom-0 h-[22%] bg-[linear-gradient(180deg,rgba(68,166,250,0.10),transparent)]" />
      <div className="absolute bottom-[22%] left-1/2 w-[min(70%,760px)] -translate-x-1/2 translate-y-1/2">
        <span className="lk-ring absolute top-0 left-0 block aspect-[6/1] w-full -translate-y-1/2 rounded-[50%] border border-[#44A6FA]/35" />
        <span
          className="lk-ring absolute top-0 left-0 block aspect-[6/1] w-full -translate-y-1/2 rounded-[50%] border border-[#44A6FA]/25"
          style={{ animationDelay: "2s" }}
        />
      </div>
    </div>
  );
}
