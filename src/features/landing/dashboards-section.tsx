"use client";

import * as React from "react";
import { DASHBOARDS } from "./landing-copy";
import { Container, DisplayHeading, LeadText } from "./landing-ui";

/** S5 — "Dashboards live on the shore": answers come back up as dashboards. */
export function DashboardsSection() {
  return (
    <section
      id="dashboards"
      className="relative bg-[linear-gradient(180deg,#050A30_0%,#0B1A4A_80%,#0B1A4A_100%)] py-28 md:py-36"
    >
      <Container>
        <DisplayHeading className="max-w-[16ch]">{DASHBOARDS.headline}</DisplayHeading>
        <LeadText className="mt-5 max-w-[38rem]">{DASHBOARDS.sub}</LeadText>
        <ul className="mt-14 grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
          {DASHBOARDS.tiles.map((tile) => (
            <li key={tile.id} className="rounded-2xl border border-[#E8F4FF]/10 bg-[#E8F4FF]/[0.04] p-5">
              <p className="lk-mono text-[11px] tracking-[0.18em] text-[#44A6FA] uppercase">{tile.kind}</p>
              <p className="mt-2 text-[15px]">{tile.title}</p>
            </li>
          ))}
        </ul>
        <p className="lk-mono mt-8 text-[12px] text-[#E8F4FF]/60">{DASHBOARDS.caption}</p>
      </Container>
    </section>
  );
}
