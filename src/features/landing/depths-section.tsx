"use client";

import * as React from "react";
import { DEPTHS } from "./landing-copy";
import { Container, DisplayHeading, Eyebrow } from "./landing-ui";

/** S3 — "Three depths": the medallion layers, Bronze to Gold. */
export function DepthsSection() {
  return (
    <section id="depths" className="relative py-28 md:py-36">
      <Container>
        <Eyebrow>Three depths</Eyebrow>
        <DisplayHeading className="mt-4">{DEPTHS.headline}</DisplayHeading>
        <ol className="mt-14 grid gap-5 md:grid-cols-3">
          {DEPTHS.layers.map((layer) => (
            <li
              key={layer.id}
              className="rounded-3xl border border-[#E8F4FF]/10 bg-[#E8F4FF]/[0.04] p-7"
              style={{ boxShadow: `inset 0 1px 0 0 ${layer.accent}55` }}
            >
              <p className="lk-mono text-[12px] tracking-[0.2em] uppercase" style={{ color: layer.accent }}>
                {layer.label} · {layer.meters}
              </p>
              <h3 className="lk-display mt-3 text-[32px] leading-[1.05] text-[#E8F4FF]">{layer.title}</h3>
              <p className="mt-3 text-[16px] leading-[1.6] text-[#E8F4FF]/80">{layer.body}</p>
            </li>
          ))}
        </ol>
      </Container>
    </section>
  );
}
