"use client";

import * as React from "react";
import { SOURCES } from "./landing-copy";
import { Container, DisplayHeading, LeadText } from "./landing-ui";

/** S2 — "Everything flows in": the sources that land in the lakehouse today. */
export function SourcesSection() {
  return (
    <section id="sources" className="relative py-28 md:py-36">
      <Container className="text-center">
        <DisplayHeading>{SOURCES.headline}</DisplayHeading>
        <LeadText className="mx-auto mt-5 max-w-[36rem]">{SOURCES.sub}</LeadText>
        <ul className="mx-auto mt-14 flex max-w-[860px] flex-wrap justify-center gap-3">
          {SOURCES.items.map((item) => (
            <li
              key={item}
              className="lk-mono rounded-full border border-[#44A6FA]/30 bg-[#44A6FA]/10 px-4 py-2 text-[13px] text-[#E8F4FF]"
            >
              {item}
            </li>
          ))}
        </ul>
        <p className="lk-mono mx-auto mt-10 max-w-[36rem] text-[12px] leading-5 tracking-wide text-[#E8F4FF]/60">
          {SOURCES.caption}
        </p>
      </Container>
    </section>
  );
}
