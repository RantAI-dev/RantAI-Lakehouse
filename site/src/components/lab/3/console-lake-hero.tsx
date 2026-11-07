import * as React from "react";
import { CtaLink } from "@/components/shared/primitives";
import { LabWhale as WhaleSvg } from "../lab-whale";
import type { LabCopy } from "../lab-shell";
import { LakeConsole } from "./lake-console";
import "./lab3.css";

/**
 * Lab 3, "Console on the lake" (a WEB-5 hero exploration; delete with its
 * folder). Product first: the hero is a floating assistant panel (O35's
 * minimal style) resting on a calm night lake, with its reflection under
 * it. A question types itself; the answer's chart rises out of the water as
 * droplets that settle into bars (or points, or a count). The whale is the
 * assistant: its head at the panel's edge, on the waterline, nods as it
 * answers. Three suggested questions replay it with another chart. All of
 * it is illustrative; nothing is asked of a server.
 */
export function ConsoleLakeHero({ copy }: Readonly<{ copy: LabCopy }>) {
  return (
    <section aria-labelledby="l3-title" className="l3 relative isolate min-h-[100svh] overflow-hidden bg-[#030820] text-[#E8F4FF]">
      <div aria-hidden="true" className="pointer-events-none absolute inset-0">
        {Array.from({ length: 18 }, (_, i) => (
          <span
            key={i}
            className="absolute rounded-full bg-[#E8F4FF]/40"
            style={{ left: `${(i * 41.3) % 100}%`, top: `${(i * 17.9) % 52}%`, width: 2, height: 2 }}
          />
        ))}
      </div>
      <div className="relative mx-auto grid min-h-[100svh] max-w-7xl items-end gap-10 px-5 pt-24 pb-[16vh] sm:px-8 lg:grid-cols-[minmax(0,4fr)_minmax(0,7fr)] lg:gap-14 lg:pt-28 max-lg:pb-[12vh]">
        <div className="relative z-20 lg:pb-10">
          <h1 id="l3-title" className="lk-display max-w-[12ch] text-[clamp(2.5rem,4.4vw,3.75rem)] leading-[1.02] tracking-[-0.015em]">
            Ask your data. It answers.
          </h1>
          <p className="mt-4 max-w-[34ch] text-[15px] leading-6 text-[#E8F4FF]/72">
            Plain-language questions, answered from your own lakehouse, with the chart to show it.
          </p>
          <div className="mt-6 flex flex-wrap gap-3">
            <CtaLink href="/#get-started">{copy.cta.install}</CtaLink>
            <CtaLink href={copy.contactHref} variant="ghost">
              {copy.cta.talk}
            </CtaLink>
          </div>
        </div>
        <LakeConsole
          whale={<WhaleSvg surface spout={false} idPrefix="l3" viewBox="104 70 392 252" className="block size-full" />}
        />
      </div>
    </section>
  );
}
