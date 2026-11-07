import * as React from "react";
import { QUESTIONS, type Question } from "./lake-data";
import { LakeController } from "./lake-controller";

/**
 * Lab 3's panel, server-rendered: every answer is in the markup, and the
 * stage wrapper's `data-pick` and `data-stage` say which one shows and how
 * far it has come (CSS in lab3.css). `LakeController` (a small client leaf)
 * types the question and moves the stages:
 * 1 the question types itself, 2 the assistant thinks, 3 its words arrive
 * and the whale nods, 4 the chart's droplets rise out of the water, 5 they
 * settle into the chart and the answer is lit. Without JavaScript, and
 * under reduced motion, the first answer is shown complete.
 */
export function LakeConsole({ whale }: Readonly<{ whale: React.ReactNode }>) {
  return (
    <div className="l3-stage relative z-10 w-full min-w-0 max-w-[600px] justify-self-end max-lg:justify-self-center" data-pick="0" data-stage="5">
      <div className="l3-panel relative z-10">
        <div className="flex items-center justify-between px-4 pt-3.5 pb-2 sm:px-5">
          <span className="flex items-center gap-2 text-[13px] font-medium">
            <svg aria-hidden="true" viewBox="0 0 16 16" className="size-3.5 text-[#8fcbff]">
              <path d="M8 1.5c.4 3.3 1.9 5 5.5 6.5-3.6 1.5-5.1 3.2-5.5 6.5-.4-3.3-1.9-5-5.5-6.5C6.1 6.5 7.6 4.8 8 1.5Z" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
            </svg>
            Assistant
          </span>
          <span className="text-[11px] text-[#E8F4FF]/45">Illustrative data</span>
        </div>

        <div className="flex min-h-[248px] flex-col gap-3 px-4 pb-3 sm:min-h-[268px] sm:px-5">
          <p className="l3-q max-w-[88%] self-end rounded-2xl rounded-br-md bg-[#18265a] px-3.5 py-2 text-[14px] leading-5">
            <span className="l3-q-text">{QUESTIONS[0].q}</span>
            <span aria-hidden="true" className="l3-caret" />
          </p>
          <div aria-live="polite" className="flex min-h-0 flex-1 flex-col">
            <span aria-hidden="true" className="l3-dots">
              <i />
              <i />
              <i />
            </span>
            {QUESTIONS.map((q, i) => (
              <div key={q.q} className="l3-ans min-h-0 flex-1 flex-col" data-i={i} data-kind={q.kind}>
                <p className="l3-a text-[14px] leading-5 text-[#E8F4FF]/85">{q.a}</p>
                <Chart q={q} />
              </div>
            ))}
          </div>
        </div>

        <div className="border-t border-[#E8F4FF]/[0.07] px-4 pt-3 pb-3.5 sm:px-5">
          <div className="flex gap-1.5 overflow-x-auto [scrollbar-width:none]" role="group" aria-label="Suggested questions">
            {QUESTIONS.map((x, i) => (
              <button
                key={x.q}
                type="button"
                data-ask={i}
                aria-pressed={i === 0}
                className="l3-chip shrink-0 rounded-full border px-3 py-1.5 text-[12px] whitespace-nowrap transition-[background-color,border-color,color] duration-200 ease-out"
              >
                {x.q}
              </button>
            ))}
          </div>
          <div className="mt-2.5 flex h-10 items-center justify-between rounded-full bg-[#020514]/60 pr-1 pl-4 text-[13px] text-[#E8F4FF]/40 ring-1 ring-[#E8F4FF]/10 max-lg:ml-12">
            Ask about your data…
            <span aria-hidden="true" className="grid size-8 place-items-center rounded-full bg-[var(--lk-primary)] text-white">
              <svg viewBox="0 0 16 16" className="size-4">
                <path d="M8 13V3.5M3.5 8 8 3.5 12.5 8" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
          </div>
        </div>
      </div>

      {/* The lake: from the panel's foot to the bottom of the screen, across it. */}
      <div aria-hidden="true" className="l3-lake">
        {[12, 30, 47, 63, 78].map((top, i) => (
          <span key={top} style={{ top: `${top}%`, left: `${(i * 23 + 8) % 70}%`, animationDelay: `${i * 0.9}s` }} />
        ))}
      </div>
      <div aria-hidden="true" className="l3-reflection" />
      {/* The assistant: the whale's head at the panel's corner, on the waterline. */}
      <div aria-hidden="true" className="l3-whale">
        {whale}
      </div>
      <LakeController typeMs={32} lengths={QUESTIONS.map((q) => q.data.length)} />
    </div>
  );
}

function Chart({ q }: Readonly<{ q: Question }>) {
  const n = q.data.length;
  const max = Math.max(...q.data.map((d) => d.value));
  const min = q.kind === "line" ? Math.min(...q.data.map((d) => d.value)) * 0.85 : 0;
  const best = q.data.findIndex((d) => d.value === max);
  const h = (v: number) => ((v - min) / (max - min)) * (q.kind === "bars" ? 72 : 92);
  return (
    <div className="l3-chart relative mt-3 flex-1">
      {q.kind === "count" && (
        <p className="l3-count lk-display absolute top-0 left-0 text-[40px] leading-none text-[#E8F4FF]">
          {q.total}
          <span className="ml-2 font-sans text-[12px] text-[#E8F4FF]/55">orders</span>
        </p>
      )}
      <div className={q.kind === "count" ? "l3-plot l3-plot-short" : "l3-plot"}>
        {q.kind === "line" && (
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="l3-line absolute inset-0 size-full overflow-visible">
            <polyline
              fill="none"
              stroke="#8fcbff"
              strokeWidth="1.6"
              vectorEffect="non-scaling-stroke"
              strokeLinejoin="round"
              points={q.data.map((d, i) => `${((i + 0.5) / n) * 100},${100 - h(d.value)}`).join(" ")}
            />
          </svg>
        )}
        {q.data.map((d, i) => (
          <span
            key={`${d.label}-${i}`}
            className="l3-bar"
            data-best={i === best ? "" : undefined}
            style={{ "--i": i, "--h": `${h(d.value)}%`, left: `${((i + 0.5) / n) * 100}%`, "--n": n } as React.CSSProperties}
          >
            {q.kind === "bars" && (
              <span className="l3-bar-label">
                <b>{d.shown}</b>
                {d.label}
              </span>
            )}
          </span>
        ))}
      </div>
      {/* The water the chart rises from, with a ring where each droplet breaks the surface. */}
      <div className="l3-pool">
        {q.data.map((d, i) => (
          <span key={`r-${d.label}-${i}`} className="l3-ring" style={{ left: `${((i + 0.5) / n) * 100}%`, animationDelay: `${i * 70}ms` }} />
        ))}
      </div>
    </div>
  );
}
