import * as React from "react";
import { ArrowUp, ArrowUpRight, Mail } from "lucide-react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { CONTACT_EMAIL, FOOTER_SECTIONS, LINKS, RANTAI_PRODUCTS, SOCIAL_LINKS } from "@/lib/home-content";
import { CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";
import { Reveal as LiftReveal, SplitWords } from "@/components/shared/reveal";
import { SocialIcon } from "@/components/shared/social-icons";
import { BackToTop, FooterWordmark, Reveal } from "./footer-motion";
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

// rantai-page footer.tsx: underline that grows from the left on hover.
const linkUnderline =
  "bg-[linear-gradient(currentColor,currentColor)] bg-[length:0%_1px] bg-[position:0_100%] bg-no-repeat pb-0.5 transition-[background-size,color] duration-300 hover:bg-[length:100%_1px] motion-reduce:transition-none";

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
        // D15: the contact page, in the visitor's language; the address itself is in the bottom bar.
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
                // rantai-page's ProductLogo, Dark variant (decorative: the name is the link text).
                // eslint-disable-next-line @next/next/no-img-element -- a static SVG mark; the export has no image optimizer
                logo: <img src={p.logo} alt="" width={Math.round(14 * p.ratio)} height={14} loading="lazy" decoding="async" className="mr-1 h-3.5 w-auto shrink-0" />,
              }
            : {
                label: p.label,
                href: page === "home" ? "#hero" : home,
                current: true,
                logo: <MascotMark className="mr-1 h-3.5 w-auto shrink-0" />,
              },
        ),
        {
          label: "rantai.dev",
          href: LINKS.rantai,
          external: true,
          // O15: the RantAI mark too (rantai-page public/rant-ai.png at 2x), made white
          // the way rantai-page shows it on dark (navbar `overDark`: brightness-0 invert).
          // eslint-disable-next-line @next/next/no-img-element -- a static mark; the export has no image optimizer
          logo: <img src="/logo/rantai.png" alt="" width={14} height={14} loading="lazy" decoding="async" className="mr-1 h-3.5 w-auto shrink-0 brightness-0 invert" />,
        },
      ],
    },
  ];
  // rantai-page's footerSocials: the four networks, then the address as a fifth icon.
  const socials = [
    ...SOCIAL_LINKS.map((s) => ({ label: s.label, href: s.href, icon: <SocialIcon id={s.id} size={16} />, external: true })),
    {
      label: copy.email.replace("{email}", CONTACT_EMAIL),
      href: LINKS.email,
      icon: <Mail aria-hidden="true" className="size-[17px]" />,
      external: false,
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
              <LiftReveal className="mt-10 w-full max-sm:mt-8" delay={0.15} y={16}>
                <FooterPrompt text={cta} to={CONTACT_EMAIL} />
              </LiftReveal>
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

      {/* WEB-5 owner QA O15: from here down, a 1:1 copy of rantai-page's
          footer.tsx ("Links sit straight on the page…"): the same markup,
          classes, spacing and type, with the lake's colours by role
          (foreground → foam, muted-foreground → foam at 65 %) and this
          site's content. */}
      {/* Links sit straight on the page, with a faded wordmark below */}
      <div className="relative overflow-hidden px-6 pt-14 sm:px-10 lg:px-16 lg:pt-20">
        <div className="grid gap-12 xl:grid-cols-[minmax(0,1fr)_auto] xl:gap-20">
          <Reveal className="max-w-sm">
            <a
              href={page === "home" ? "#hero" : home}
              aria-label={copy.home}
              className="inline-flex items-center gap-2.5 text-[#E8F4FF] transition-opacity hover:opacity-75"
            >
              <MascotMark className="h-7 w-auto" />
              <span className="text-[19px] font-semibold tracking-[-0.03em]">RantAI Lakehouse</span>
            </a>
            <p className="mt-4 text-sm leading-relaxed text-[#E8F4FF]/65">{copy.tagline}</p>
            <ul className="mt-5 -ml-2 flex items-center gap-1">
              {socials.map((social) => (
                <li key={social.label}>
                  <a
                    href={social.href}
                    aria-label={social.label}
                    {...(social.external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
                    className="grid size-9 place-items-center rounded-full text-[#E8F4FF]/55 transition-colors hover:bg-[#E8F4FF]/5 hover:text-[#E8F4FF]"
                  >
                    {social.icon}
                  </a>
                </li>
              ))}
            </ul>
          </Reveal>

          <nav aria-label={copy.nav} className="grid gap-y-9 sm:grid-cols-3 sm:gap-x-10 sm:gap-y-10 lg:gap-x-16">
            {columns.map((column, i) => (
              <Reveal key={column.title} index={i + 1}>
                <p className="text-sm font-medium text-[#E8F4FF]">{column.title}</p>
                <ul className="mt-2 sm:mt-4 sm:space-y-3">
                  {column.links.map((link) => (
                    <li key={link.label}>
                      <a
                        href={link.href}
                        aria-current={link.current && page === "home" ? "page" : undefined}
                        {...(link.external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
                        className={`group inline-flex items-center gap-1 text-sm text-[#E8F4FF]/65 transition-colors hover:text-[#E8F4FF] max-sm:min-h-10 max-sm:bg-none max-sm:text-[15px] ${linkUnderline}`}
                      >
                        {link.logo}
                        {link.label}
                        {/* The site's own content: this product is the current site. */}
                        {link.current && (
                          <span className="ml-1 rounded-full bg-[#44A6FA]/15 px-2 py-0.5 text-[11px] leading-4 font-medium text-[#8fcbff]">
                            {copy.current}
                          </span>
                        )}
                        {link.external && (
                          <ArrowUpRight
                            aria-hidden="true"
                            className="size-3.5 opacity-50 transition-[translate,opacity] duration-300 group-hover:translate-x-0.5 group-hover:-translate-y-0.5 group-hover:opacity-100 motion-reduce:transition-none"
                          />
                        )}
                      </a>
                    </li>
                  ))}
                </ul>
              </Reveal>
            ))}
          </nav>
        </div>

        <div className="mt-12 flex flex-col gap-2 border-t border-[#E8F4FF]/10 pt-6 text-sm text-[#E8F4FF]/65 sm:mt-16 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
          {/* The year is the build's: the export is static. */}
          <p>
            © {new Date().getFullYear()} RantAI
            <span className="mx-1.5 opacity-50">·</span>
            {copy.rights}
          </p>
          <div className="flex items-center justify-between gap-x-6 sm:justify-start">
            <a
              href={LINKS.email}
              className={`hover:text-[#E8F4FF] max-sm:inline-flex max-sm:min-h-10 max-sm:items-center max-sm:bg-none ${linkUnderline}`}
            >
              {CONTACT_EMAIL}
            </a>
            <BackToTop label={copy.backToTop} />
          </div>
        </div>

        {/* rantai-page's clamp is for six letters ("RantAI"); "Lakehouse" has nine, so
            each bound is scaled by 6/9, or the word runs off the page (measured in O7). */}
        <FooterWordmark
          text="Lakehouse"
          className="mt-4 mb-[-0.2em] text-[clamp(4rem,16vw,14.5rem)] leading-[0.85] font-semibold tracking-[-0.06em]"
        />
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
