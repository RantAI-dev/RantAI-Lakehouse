"use client";

import * as React from "react";
import { GOVERNANCE } from "./landing-copy";
import { Container, DisplayHeading, LeadText } from "./landing-ui";

/** S6 — "Guarded by default": roles, masking and lineage. */
export function GovernanceSection() {
  return (
    <section id="governance" className="relative bg-[#0B1A4A] py-28 md:py-36">
      <Container>
        <DisplayHeading>{GOVERNANCE.headline}</DisplayHeading>
        <LeadText className="mt-5 max-w-[40rem]">{GOVERNANCE.sub}</LeadText>
        <div className="mt-12 overflow-x-auto rounded-2xl border border-[#E8F4FF]/10">
          <table className="w-full min-w-[560px] text-left">
            <thead>
              <tr className="lk-mono text-[12px] tracking-[0.14em] text-[#E8F4FF]/60 uppercase">
                {GOVERNANCE.columns.map((c) => (
                  <th key={c} className="px-4 py-3 font-medium">
                    {c}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {GOVERNANCE.rows.map((r) => (
                <tr key={r.email} className="border-t border-[#E8F4FF]/10 text-[14px]">
                  <td className="px-4 py-3">{r.name}</td>
                  <td className="lk-mono px-4 py-3">{r.email}</td>
                  <td className="px-4 py-3">{r.city}</td>
                  <td className="lk-mono px-4 py-3">{r.amount}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <ul className="mt-10 grid gap-6 md:grid-cols-3">
          {GOVERNANCE.facts.map((f) => (
            <li key={f.label}>
              <p className="lk-mono text-[11px] tracking-[0.2em] text-[#44A6FA] uppercase">{f.label}</p>
              <p className="mt-2 text-[15px] leading-6 text-[#E8F4FF]/85">{f.text}</p>
            </li>
          ))}
        </ul>
      </Container>
    </section>
  );
}
