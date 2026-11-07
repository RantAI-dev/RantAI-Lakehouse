import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { CONTACT_EMAIL, FOOTER_SECTIONS, LINKS, RANTAI_PRODUCTS, SOCIAL_LINKS } from "@/lib/home-content";
import { Container } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";
import { Reveal } from "@/components/shared/reveal";
import { SocialIcon } from "@/components/shared/social-icons";
import { cn } from "@/lib/utils";

/**
 * S9 — the footer, in the RantAI family's shape (WEB-5 D17, after
 * rantai-page's `components/layout/footer.tsx`, in the lake's look): the
 * mark and tagline with "Follow RantAI", then the page's sections, the ways
 * to reach us, and the RantAI products with their official marks (this one
 * marked as the current site) and rantai.dev, then the legal line with the
 * plain e-mail address.
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

  return (
    <footer className="relative border-t border-[#E8F4FF]/10 bg-[#020514] pt-16 pb-10">
      {/* Phones: the mark across, then Product and Connect side by side, then the products across. */}
      <Container className="grid grid-cols-2 gap-x-6 gap-y-10 md:grid-cols-[1.3fr_1fr_0.8fr_1.1fr] md:gap-10">
        <div className="col-span-2 md:col-span-1">
          <a
            href={page === "home" ? "#hero" : home}
            className="inline-flex items-center gap-2.5 rounded-lg text-[15px] font-medium tracking-tight text-[#E8F4FF] transition-opacity duration-200 ease-out hover:opacity-80"
          >
            <MascotMark className="h-6 w-auto" />
            RantAI Lakehouse
          </a>
          <p className="mt-4 max-w-[18rem] text-[15px] leading-6 text-pretty text-[#E8F4FF]/70">{copy.tagline}</p>
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
        {columns.map((col) => (
          <nav key={col.title} aria-label={col.title} className={cn(col.title === copy.columns.products && "max-md:col-span-2")}>
            <h2 className="text-[11px] leading-4 font-medium tracking-[0.2em] text-[#E8F4FF]/60 uppercase">{col.title}</h2>
            <ul className="mt-3 md:mt-4 md:space-y-2.5">
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
      </Container>
      <Container className="mt-14">
        <Reveal y={12}>
          <div className="flex flex-col gap-2 border-t border-[#E8F4FF]/10 pt-6 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-[12px] text-[#E8F4FF]/60">{copy.legal}</p>
            <a href={LINKS.email} className={cn(linkClass, "self-start text-[13px] text-[#E8F4FF]/70 sm:self-auto")}>
              {CONTACT_EMAIL}
            </a>
          </div>
        </Reveal>
      </Container>
    </footer>
  );
}
