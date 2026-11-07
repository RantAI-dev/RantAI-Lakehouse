import * as React from "react";
import { ArrowUpRight, Mail } from "lucide-react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { CONTACT_EMAIL, FOOTER_SECTIONS, LINKS, RANTAI_PRODUCTS, SOCIAL_LINKS } from "@/lib/home-content";
import { DisplayHeading, Eyebrow } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";
import { SplitWords } from "@/components/shared/reveal";
import { SocialIcon } from "@/components/shared/social-icons";
import { BackToTop, FooterWordmark, Reveal } from "./footer-motion";
import { SourceFlow } from "./source-flow";
import { SourceIcon, type SourceIconId } from "@/components/shared/brand-icons";

/**
 * WEB-5 O14: the sources the footer card offers, ten of the sources the
 * product connects to today (the sources section's own, claimed there),
 * with their marks (`components/shared/brand-icons.tsx`).
 */
const FLOW_SOURCES: readonly { name: string; icon: SourceIconId }[] = [
  // O46: the first five get a chip; the rest are behind the "+N" select.
  { name: "PostgreSQL", icon: "postgresql" },
  { name: "MySQL", icon: "mysql" },
  { name: "MongoDB", icon: "mongodb" },
  { name: "Salesforce", icon: "app" },
  { name: "Excel", icon: "spreadsheet" },
  { name: "Kafka", icon: "apachekafka" },
  { name: "SQL Server", icon: "database" },
  { name: "HubSpot", icon: "hubspot" },
  { name: "CSV", icon: "spreadsheet" },
  { name: "REST API", icon: "rest" },
];

/**
 * S9 — the footer, on rantai-page's footer structure in the lake's look
 * (WEB-5 owner QA O7, `components/layout/footer.tsx`, `footer-motion.tsx`
 * and `footer-prompt.tsx`), so every product site has the same footer:
 * 1. a rounded call-to-action card, Lakehouse's own (O14): pick a source and
 *    watch it flow through Bronze, Silver and Gold into a dashboard, then
 *    start with it (an e-mail; on the contact page, the form);
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
                logo: <img src={p.logo} alt="" width={Math.round(14 * p.ratio)} height={14} loading="lazy" decoding="async" className="h-3.5 w-auto max-w-5 shrink-0" />,
              }
            : {
                label: p.label,
                href: page === "home" ? "#hero" : home,
                current: true,
                logo: <MascotMark className="h-3.5 w-auto max-w-5 shrink-0" />,
              },
        ),
        {
          label: "rantai.dev",
          href: LINKS.rantai,
          external: true,
          // O15: the RantAI mark too (rantai-page public/rant-ai.png at 2x), made white
          // the way rantai-page shows it on dark (navbar `overDark`: brightness-0 invert).
          // eslint-disable-next-line @next/next/no-img-element -- a static mark; the export has no image optimizer
          logo: <img src="/logo/rantai.png" alt="" width={14} height={14} loading="lazy" decoding="async" className="h-3.5 w-auto max-w-5 shrink-0 brightness-0 invert" />,
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
    // No background of its own (O28): at rest the page-wide water (or, on the contact page, the
    // abyss) shows here anyway, and once the pull lifts the footer with a transform it would
    // paint its own dark fill over the water.
    <footer className="flex flex-col gap-3 px-3 pb-3 sm:gap-4 sm:px-4 sm:pb-4">
      {/* 1. The call to action, as a card (rantai-page's FooterCtaCard). On
          the home page it is the action after the story's end (the final
          section); on the contact page it points back up to the form
          instead of repeating it. */}
      {/* O46: compact, two columns, a flat solid panel with a hairline (no water, no rings). */}
      <section aria-labelledby="footer-cta-heading" className="relative isolate rounded-[28px] bg-[#070F3A] ring-1 ring-[#E8F4FF]/10">
        <div className="mx-auto max-w-6xl">
          <SourceFlow
            names={FLOW_SOURCES.map((s) => s.name)}
            marks={FLOW_SOURCES.map((s) => <SourceIcon key={s.name} id={s.icon} />)}
            text={cta}
            to={CONTACT_EMAIL}
            page={page}
            installHref={page === "home" ? "#get-started" : `${home}#get-started`}
            intro={
              <>
                <Eyebrow>{cta.eyebrow}</Eyebrow>
                <DisplayHeading id="footer-cta-heading" size="sm" className="mt-4 max-w-[16ch]">
                  <SplitWords text={cta.title} />
                </DisplayHeading>
                <p className="mt-3 max-w-[40ch] text-[15px] leading-6 text-[#E8F4FF]/70">{cta.line}</p>
              </>
            }
          />
        </div>
      </section>

      {/* WEB-5 owner QA O15: from here down, a 1:1 copy of rantai-page's
          footer.tsx ("Links sit straight on the page…"): the same markup,
          classes, spacing and type, with the lake's colours by role
          (foreground → foam, muted-foreground → foam at 65 %) and this
          site's content. */}
      {/* Links sit straight on the page, with a faded wordmark below */}
      {/* overflow-clip (rantai: overflow-hidden): the same crop, but its paint edge can widen
          during the wordmark's pull without changing layout (O28, footer-motion.tsx). */}
      <div className="relative overflow-clip px-6 pt-14 sm:px-10 lg:px-16 lg:pt-20">
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
                        {/* O27: every mark sits in the same centred 20 px slot, so all the labels start at one x. */}
                        {link.logo && <span className="mr-1 inline-flex w-5 shrink-0 justify-center">{link.logo}</span>}
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
