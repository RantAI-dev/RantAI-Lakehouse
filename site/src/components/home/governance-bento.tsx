import * as React from "react";
import { BadgeCheck, Check, Folder, LayoutDashboard, Lock, LockOpen, Search } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { BentoFxHost } from "./bento-fx-host";
import { ColumnLineage } from "./column-lineage";
import { LiveTile } from "./live-tile";

/**
 * "Guarded by default", the bento (WEB-5 owner QA O16): five tiles, each a
 * small living illustration of something shipped, with a 2-4 word title and
 * one line (every line a claim). One wide tile and four small ones: lineage
 * spans two cells on desktop, a single column on phones.
 *
 * Server components; the motion is CSS (".lk-gv-" in app/globals.css),
 * started and paused by the same `LiveTile` observer as the dashboards'
 * tiles: an entrance once, then a calm loop while on screen, a lift on
 * hover and focus. Reduced motion and no JavaScript show the final state.
 * The access tile follows the table's "View as" switch through the
 * section's `data-role`, in CSS.
 */

type Bento = HomeCopy["governance"]["bento"];

/**
 * O37 (the clean rule of O35): a flat dark surface, one hairline border (the
 * one that glows at the pointer, `.lk-mb-tile`), content set apart by fill
 * and space. No lift, no tilt.
 */
const tile =
  "lk-mb-tile relative flex flex-col overflow-hidden rounded-3xl bg-[var(--lk-n-surface)] p-6 outline-none focus-visible:ring-2 focus-visible:ring-[#44A6FA]/70 max-md:p-4 md:px-8 md:py-6";

function Words({ title, line }: { title: string; line: string }) {
  return (
    <div className="mt-auto pt-5 max-md:pt-3 md:pt-4">
      <h3 className="text-[16px] leading-6 font-medium text-balance text-[#E8F4FF]">{title}</h3>
      <p className="mt-0.5 text-[13.5px] leading-[1.5] text-pretty text-white/55">{line}</p>
    </div>
  );
}

export function GovernanceBento({ copy }: { copy: Bento }) {
  return (
    <div className="lk-mb relative mt-4 max-md:mt-3">
      {/* O37: the spotlight over the whole grid, moved by the deferred controller. */}
      <span aria-hidden="true" className="lk-mb-spot pointer-events-none absolute -inset-10 z-10" />
      <BentoFxHost />
    {/* Owner QA 2026-11-25 ("gausah di scroll scrolll ke kanan atau kiri"): below 768 px the tiles
        stack, each as tall as its own content, with tighter padding and gaps (the M3 swipe row is
        gone; the lineage keeps its vertical drawing). */}
    <ul className="grid gap-4 max-md:gap-2.5 md:grid-cols-6">
      <LiveTile label={copy.lineage.title} className={`${tile} md:col-span-4`}>
        <ColumnLineage />
        <Words title={copy.lineage.title} line={copy.lineage.line} />
      </LiveTile>
      <LiveTile label={copy.proof.title} className={`${tile} md:col-span-2`}>
        <Proof badge={copy.proof.badge} />
        <Words title={copy.proof.title} line={copy.proof.line} />
      </LiveTile>
      <LiveTile label={copy.audit.title} className={`${tile} md:col-span-2`}>
        <Audit text={copy.audit} />
        <Words title={copy.audit.title} line={copy.audit.line} />
      </LiveTile>
      <LiveTile label={copy.catalog.title} className={`${tile} md:col-span-2`}>
        <Catalog text={copy.catalog} />
        <Words title={copy.catalog.title} line={copy.catalog.line} />
      </LiveTile>
      <LiveTile label={copy.access.title} className={`${tile} md:col-span-2`}>
        <Access text={copy.access} />
        <Words title={copy.access.title} line={copy.access.line} />
      </LiveTile>
    </ul>
    </div>
  );
}

/** An e-mail cell that masks and unmasks, with the test's check. */
function Proof({ badge }: { badge: string }) {
  return (
    <div aria-hidden="true" className="flex flex-col items-start gap-3">
      <div className="relative h-10 w-full max-w-[16rem] overflow-hidden rounded-xl bg-[var(--lk-n-raised)] px-3">
        <span className="lk-gv-plain absolute inset-y-0 left-3 flex items-center text-[14px] text-[#E8F4FF]">mira.okafor@example.com</span>
        {/* O36: masking shown as masked text that keeps the shape, never a grey bar. */}
        <span className="lk-gv-mask absolute inset-y-0 left-3 flex items-center gap-1.5 text-[14px] text-white/55">
          <Lock className="size-3 text-[#8fcbff]/70" />
          m•••@•••.com
        </span>
      </div>
      <span className="lk-gv-badge inline-flex items-center gap-1.5 rounded-full bg-[#7be0b0]/10 px-2.5 py-1 text-[12px] font-medium text-[#7be0b0]">
        <BadgeCheck className="size-3.5" />
        {badge}
      </span>
    </div>
  );
}

/** Log rows ticking in; the risky one waits, then is approved, now and then. */
function Audit({ text }: { text: Bento["audit"] }) {
  return (
    <ul aria-hidden="true" className="flex flex-col gap-1.5">
      {text.rows.map((r, i) => (
        <li
          key={r}
          // Owner QA 2026-11-25: a phone shows the last two entries (the second and the one that waits).
          className={`lk-gv-row flex items-center gap-2 rounded-xl bg-[var(--lk-n-raised)] px-3 py-1.5 text-[12.5px] text-white/85 ${i === 0 ? "max-md:hidden" : ""}`}
          style={{ animationDelay: `${0.15 + i * 0.25}s` }}
        >
          <span className="size-1.5 shrink-0 rounded-full bg-[#44A6FA]" />
          <span className="min-w-0 flex-1 truncate">{r}</span>
          {i === text.rows.length - 1 ? (
            <span className="relative inline-grid shrink-0">
              <span className="lk-gv-wait col-start-1 row-start-1 inline-flex items-center gap-1 rounded-full bg-[#F2C14E]/12 px-2 py-0.5 text-[11px] font-medium text-[#F2C14E] max-md:text-[12px]">
                <span className="lk-pulse size-1.5 rounded-full bg-[#F2C14E]" />
                {text.awaiting}
              </span>
              <span className="lk-gv-ok col-start-1 row-start-1 inline-flex items-center gap-1 justify-self-end rounded-full bg-[#7be0b0]/12 px-2 py-0.5 text-[11px] font-medium text-[#7be0b0] max-md:text-[12px]">
                <Check className="size-3" />
                {text.approved}
              </span>
            </span>
          ) : (
            <Check className="size-3.5 shrink-0 text-white/45" />
          )}
        </li>
      ))}
    </ul>
  );
}

/** A search that types a table name, and its results with their marks. */
function Catalog({ text }: { text: Bento["catalog"] }) {
  return (
    <div aria-hidden="true" className="flex flex-col gap-1.5">
      <div className="flex h-8 items-center gap-2 rounded-xl bg-[var(--lk-n-raised)] px-3">
        <Search className="size-3.5 text-white/50" />
        {/* The query is a table name, code: monospace, typed in steps of one character. */}
        <span className="lk-gv-type overflow-hidden font-mono text-[12.5px] whitespace-nowrap text-[#E8F4FF]" style={{ "--lk-n": text.query.length } as React.CSSProperties}>
          {text.query}
        </span>
        <span className="lk-gv-caret h-4 w-px bg-[#44A6FA]" />
      </div>
      <div className="lk-gv-result flex items-center gap-2 rounded-xl px-3 py-1">
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-[#E8F4FF]">gold.revenue_by_city</span>
        <span className="rounded-full bg-[#7be0b0]/12 px-2 py-0.5 text-[10.5px] font-medium text-[#7be0b0] max-md:text-[12px]">{text.certified}</span>
        <span className="rounded-full bg-white/8 px-2 py-0.5 text-[10.5px] text-white/75 max-md:text-[12px]">#{text.tag}</span>
      </div>
      <div className="lk-gv-result flex items-center gap-2 rounded-xl px-3 py-1 opacity-60" style={{ animationDelay: "1.6s" }}>
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-white/70 line-through decoration-white/30">silver.revenue_old</span>
        <span className="rounded-full bg-[#F2C14E]/12 px-2 py-0.5 text-[10.5px] font-medium text-[#F2C14E] max-md:text-[12px]">{text.deprecated}</span>
      </div>
    </div>
  );
}

/** Folders and a dashboard; its lock opens for Admin and stays shut for Analyst (the table's switch). */
function Access({ text }: { text: Bento["access"] }) {
  return (
    <div aria-hidden="true" className="flex flex-col gap-1 text-[13px] text-white/85">
      {text.folders.map((f, i) => (
        <div key={f} className="flex items-center gap-2">
          <Folder className="size-4 text-[#44A6FA]/80" />
          {f}
          {i === 1 && (
            <div className="ml-auto flex -space-x-1.5">
              <span className="grid size-6 place-items-center rounded-full bg-[#44A6FA]/25 text-[10px] font-semibold text-[#bfe0ff] ring-2 max-md:text-[12px] ring-[#0b1a2d]">A</span>
              <span className="grid size-6 place-items-center rounded-full bg-white/12 text-[10px] font-semibold text-white/80 ring-2 max-md:text-[12px] ring-[#0b1a2d]">An</span>
            </div>
          )}
        </div>
      ))}
      <div className="ml-6 flex items-center gap-2 rounded-xl bg-[var(--lk-n-raised)] px-3 py-1.5">
        <LayoutDashboard className="size-4 text-white/70" />
        {text.dashboard}
        <span className="relative ml-auto inline-grid">
          <span className="lk-gv-closed col-start-1 row-start-1 inline-flex items-center gap-1 text-[11.5px] text-[#8fcbff] max-md:text-[12px]">
            <Lock className="size-3.5" />
            {text.closed}
          </span>
          <span className="lk-gv-open col-start-1 row-start-1 inline-flex items-center gap-1 justify-self-end text-[11.5px] text-[#7be0b0] max-md:text-[12px]">
            <LockOpen className="size-3.5" />
            {text.open}
          </span>
        </span>
      </div>
    </div>
  );
}
