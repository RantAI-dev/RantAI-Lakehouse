import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { GovernanceTable } from "./governance-table";

/**
 * S6 — "Guarded by default": roles, masking and lineage.
 *
 * An interactive table. It starts as Admin and, once, flips to Analyst when it
 * scrolls into view, so the visitor sees the names and emails scramble into
 * their masked values; the toggle then lets them go back and forth. Values
 * are fictional people on the reserved example.com domain. Nothing here
 * calls an API: the masking shown is the same rule, not the real engine.
 *
 * WEB-5 review fix R5: a server component; the table is the client leaf in
 * `governance-table.tsx`.
 */

export function GovernanceSection({ copy }: { copy: HomeCopy["governance"] }) {
  return (
    <section
      id="governance"
      className="lk-fallback-bg relative scroll-mt-16 bg-[linear-gradient(180deg,#0d2462_0%,#0B1A4A_22%,#07123a_70%,#050A30_100%)] py-28 md:py-36"
    >
      <Container>
        <DisplayHeading>
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <Reveal>
          <LeadText className="mt-5 max-w-[40rem]">{copy.sub}</LeadText>
        </Reveal>

        <Reveal className="mt-12" y={40}>
          <GovernanceTable copy={copy} />
        </Reveal>

        <ul className="mt-12 grid gap-8 md:grid-cols-3 md:gap-6">
          {copy.facts.map((f, i) => (
            <Reveal as="li" key={f.label} delay={i * 0.1}>
              <p className="text-[11px] tracking-[0.2em] text-[#44A6FA] uppercase">{f.label}</p>
              <p className="mt-2 text-[15px] leading-6 text-[#E8F4FF]/85">{f.text}</p>
            </Reveal>
          ))}
        </ul>
      </Container>
    </section>
  );
}

