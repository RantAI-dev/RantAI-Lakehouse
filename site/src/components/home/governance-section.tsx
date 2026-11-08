import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { GovernanceBento } from "./governance-bento";
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
 *
 * WEB-5 owner QA O16 + O17: one short line under the headline, the table
 * rebuilt as a product-grade data table, and the facts as a bento of small
 * living tiles (`governance-bento.tsx`), so the section shows more than it
 * says (DESIGN.md §2).
 */

export function GovernanceSection({ copy }: { copy: HomeCopy["governance"] }) {
  return (
    <section
      id="governance"
      // WEB-5 audit #19: less padding below, so the gap to "Yours to keep" is not a band of empty water.
      className="lk-fallback-bg relative scroll-mt-16 bg-[var(--lk-n-bg)] pt-28 pb-16 md:pt-36 md:pb-20"
    >
      <Container>
        <DisplayHeading>
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <Reveal>
          <LeadText className="mt-5 max-w-[40rem]">{copy.sub}</LeadText>
        </Reveal>

        <Reveal className="mt-10" y={40}>
          <GovernanceTable copy={copy} />
        </Reveal>

        {/* O16: the five facts as a bento of living tiles, instead of a ragged text grid. */}
        <GovernanceBento copy={copy.bento} />
      </Container>
    </section>
  );
}

