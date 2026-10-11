"use client";

import * as React from "react";
import { CTA, FINAL, LINKS } from "./landing-copy";
import { Container, CtaLink, DisplayHeading } from "./landing-ui";
import { MascotLogo } from "./mascot-svg";

/** S8 — the last call: the creature, the headline, the two main buttons. */
export function FinalSection() {
  return (
    <section
      id="final"
      className="relative flex min-h-[100svh] items-center overflow-hidden bg-[linear-gradient(180deg,#050A30_0%,#0A1F55_70%,#030B2A_100%)] py-28"
    >
      <Container className="flex flex-col items-center text-center">
        <MascotLogo className="w-[min(60vw,300px)]" />
        <DisplayHeading className="mt-10 max-w-[14ch]">{FINAL.headline}</DisplayHeading>
        <div className="mt-9 flex flex-wrap items-center justify-center gap-3">
          <CtaLink href={LINKS.console}>{CTA.console}</CtaLink>
          <CtaLink href={LINKS.talk} variant="ghost">
            {CTA.talk}
          </CtaLink>
        </div>
      </Container>
    </section>
  );
}
