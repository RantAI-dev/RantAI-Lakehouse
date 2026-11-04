import * as React from "react";
import type { CommonCopy } from "@/lib/copy";
import { CONTACT_EMAIL, FAMILY_LINKS, FOOTER_SECTIONS, LINKS } from "@/lib/home-content";
import { Container } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";
import { Reveal } from "@/components/shared/reveal";

type Column = { title: string; links: readonly { label: string; href: string }[] };

/** S9 — footer: the family links and the legal line. */
export function SiteFooter({ copy }: { copy: CommonCopy["footer"] }) {
  const columns: Column[] = [
    {
      title: copy.columns.product,
      links: FOOTER_SECTIONS.map((s) => ({ label: copy.sections[s.key], href: s.href })),
    },
    { title: copy.columns.rantai, links: FAMILY_LINKS },
    {
      title: copy.columns.connect,
      links: [
        { label: copy.userGuide, href: LINKS.guide },
        { label: "GitHub", href: LINKS.source },
        { label: CONTACT_EMAIL, href: LINKS.email },
      ],
    },
  ];

  return (
    <footer className="relative border-t border-[#E8F4FF]/10 bg-[#020514] py-16">
      <Container className="grid gap-12 md:grid-cols-[1.4fr_repeat(3,1fr)]">
        <div>
          <MascotMark className="size-8" />
          <p className="mt-4 max-w-[18rem] text-[15px] leading-6 text-[#E8F4FF]/75">{copy.tagline}</p>
        </div>
        {columns.map((col) => (
          <nav key={col.title} aria-label={col.title}>
            <h2 className="text-[11px] leading-4 font-medium tracking-[0.2em] text-[#E8F4FF]/55 uppercase">
              {col.title}
            </h2>
            <ul className="mt-4 space-y-2.5">
              {col.links.map((l) => (
                <li key={l.href + l.label}>
                  <a
                    href={l.href}
                    className="text-[15px] leading-6 text-[#E8F4FF]/85 transition-colors hover:text-[#44A6FA]"
                    {...(l.href.startsWith("http") ? { target: "_blank", rel: "noreferrer noopener" } : {})}
                  >
                    {l.label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </Container>
      <Container className="mt-14">
        <Reveal y={12}>
          <p className="text-[12px] text-[#E8F4FF]/55">{copy.legal}</p>
        </Reveal>
      </Container>
    </footer>
  );
}
