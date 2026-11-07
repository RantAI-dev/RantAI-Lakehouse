import * as React from "react";
import { CtaLink } from "@/components/shared/primitives";
import { SourceIcon, type SourceIconId } from "@/components/shared/brand-icons";
import { LabWhale as WhaleSvg } from "../lab-whale";
import type { LabCopy } from "../lab-shell";
import { LivingWaterline } from "./living-waterline";
import "./lab1.css";

/**
 * Lab 1, "Waterline" (a WEB-5 hero exploration; delete with its folder).
 * A cinematic split: a living waterline about 45 % down the screen (34 % on
 * phones) bulges towards the pointer and ripples where it is pressed. Above
 * it the night sky, the moon and the headline; below it a cross-section of
 * the product: source marks sink as droplets through faint Bronze, Silver
 * and Gold strata and settle into a small answer. The whale floats on the
 * line, its head above and its belly below, bent a little by the water.
 *
 * Server-rendered; only the waterline (an SVG path redrawn each frame while
 * on screen) is a client leaf. The droplets are CSS. Under reduced motion
 * the line is drawn once, still, and the droplets rest on their strata.
 */
const DROPS: readonly { id: SourceIconId; x: number }[] = [
  { id: "postgresql", x: 14 },
  { id: "apachekafka", x: 82 },
  { id: "mysql", x: 31 },
  { id: "hubspot", x: 66 },
  { id: "mongodb", x: 22 },
  { id: "spreadsheet", x: 90 },
  { id: "googleanalytics", x: 42 },
  { id: "rest", x: 58 },
];

const STRATA = [
  { name: "Bronze", note: "raw, as it came", colour: "#b98157" },
  { name: "Silver", note: "cleaned and checked", colour: "#aab6c2" },
  { name: "Gold", note: "ready for answers", colour: "#d9b45a" },
] as const;

export function WaterlineHero({ copy }: Readonly<{ copy: LabCopy }>) {
  const whale = (part: "top" | "bottom") => (
    <div className={`l1-whale-${part}`}>
      <WhaleSvg idPrefix={`l1${part}`} className="block size-full" />
    </div>
  );
  return (
    <section aria-labelledby="l1-title" className="l1 relative isolate h-[100svh] min-h-[620px] overflow-hidden bg-[#071040] text-[#E8F4FF]">
      {/* The sky: a few still stars and the moon, flat. */}
      <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 h-[var(--l1-line)]">
        {Array.from({ length: 26 }, (_, i) => (
          <span
            key={i}
            className="l1-star absolute rounded-full bg-[#E8F4FF]"
            style={{ left: `${(i * 37.7) % 100}%`, top: `${(i * 23.3) % 88}%`, width: i % 5 ? 2 : 3, height: i % 5 ? 2 : 3, animationDelay: `${(i * 0.73) % 6}s` }}
          />
        ))}
        <span className="l1-moon absolute rounded-full bg-[#E8F4FF]" />
      </div>

      <LivingWaterline>
        {whale("top")}
        {whale("bottom")}
      </LivingWaterline>

      {/* The moon's broken reflection on the water. */}
      <div aria-hidden="true" className="l1-reflection pointer-events-none absolute">
        {[0, 1, 2, 3, 4].map((i) => (
          <span key={i} style={{ width: `${70 - i * 12}%`, animationDelay: `${i * 0.6}s` }} />
        ))}
      </div>

      {/* Below the line: the product, in cross-section. */}
      <div aria-hidden="true" className="l1-deep pointer-events-none absolute inset-x-0 bottom-0">
        {STRATA.map((s, i) => (
          <div key={s.name} className="l1-stratum absolute inset-x-0" style={{ "--l1-c": s.colour, top: `${18 + i * 22}cqh` } as React.CSSProperties}>
            <span className="l1-stratum-label">
              <b>{s.name}</b> <span className="max-md:hidden">· {s.note}</span>
            </span>
          </div>
        ))}
        {DROPS.map((d, i) => (
          <span key={d.id} className="l1-drop" style={{ "--x": d.x, "--d": `${i * 1.25}s`, "--r": `${28 + (i % 3) * 22}cqh` } as React.CSSProperties}>
            <SourceIcon id={d.id} className="size-4" />
          </span>
        ))}
        <span className="l1-answer">
          <span className="l1-answer-bars">
            <i style={{ height: "45%" }} />
            <i style={{ height: "70%" }} />
            <i style={{ height: "100%" }} />
            <i style={{ height: "60%" }} />
          </span>
          Revenue by city
        </span>
      </div>

      {/* Above the line: the words, in the sky. */}
      <div className="l1-copy relative z-20 mx-auto flex h-[var(--l1-line)] max-w-7xl flex-col justify-end px-5 pb-[clamp(1.5rem,4vh,3rem)] sm:px-8">
        <h1 id="l1-title" className="lk-display max-w-[11ch] text-[clamp(3rem,7.2vw,7.5rem)] leading-[0.92] tracking-[-0.02em] text-[#E8F4FF]">
          Your data has a home now.
        </h1>
        <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-3 max-md:mt-4">
          <p className="max-w-[32ch] text-[15px] leading-6 text-[#E8F4FF]/75 max-sm:text-[14px]">
            One lakehouse with dashboards built in.<span className="max-sm:hidden"> Every source, in open tables.</span>
          </p>
          <div className="flex items-center gap-2">
            <CtaLink href="/#get-started" size="sm">
              {copy.cta.install}
            </CtaLink>
            <CtaLink href={copy.contactHref} size="sm" variant="ghost">
              {copy.cta.talk}
            </CtaLink>
          </div>
        </div>
      </div>
    </section>
  );
}
