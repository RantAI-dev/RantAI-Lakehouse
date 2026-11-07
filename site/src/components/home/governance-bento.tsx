import * as React from "react";
import { BadgeCheck, Check, Folder, LayoutDashboard, Lock, LockOpen, Search } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
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

const tile =
  "flex flex-col rounded-3xl border border-[#E8F4FF]/12 bg-[#E8F4FF]/[0.04] p-5 transition-[translate,border-color,box-shadow] duration-200 ease-out outline-none hover:-translate-y-1 hover:border-[#44A6FA]/45 hover:shadow-[0_0_40px_-10px_rgba(68,166,250,0.5)] focus-visible:-translate-y-1 focus-visible:border-[#44A6FA]/70 motion-reduce:transition-none motion-reduce:hover:translate-y-0 md:p-6";

function Words({ title, line }: { title: string; line: string }) {
  return (
    <div className="mt-auto pt-5">
      <h3 className="text-[17px] leading-6 font-medium text-balance text-[#E8F4FF]">{title}</h3>
      <p className="mt-1 text-[14px] leading-[1.55] text-pretty text-[#E8F4FF]/70">{line}</p>
    </div>
  );
}

export function GovernanceBento({ copy }: { copy: Bento }) {
  return (
    <ul className="mt-6 grid gap-4 md:grid-cols-3 md:gap-5">
      <LiveTile label={copy.lineage.title} className={`${tile} md:col-span-2`}>
        <Lineage nodes={copy.lineage.nodes} />
        <Words title={copy.lineage.title} line={copy.lineage.line} />
      </LiveTile>
      <LiveTile label={copy.proof.title} className={tile}>
        <Proof badge={copy.proof.badge} />
        <Words title={copy.proof.title} line={copy.proof.line} />
      </LiveTile>
      <LiveTile label={copy.audit.title} className={tile}>
        <Audit text={copy.audit} />
        <Words title={copy.audit.title} line={copy.audit.line} />
      </LiveTile>
      <LiveTile label={copy.catalog.title} className={tile}>
        <Catalog text={copy.catalog} />
        <Words title={copy.catalog.title} line={copy.catalog.line} />
      </LiveTile>
      <LiveTile label={copy.access.title} className={tile}>
        <Access text={copy.access} />
        <Words title={copy.access.title} line={copy.access.line} />
      </LiveTile>
    </ul>
  );
}

/** Raw → clean → curated, one column's path, with a pulse travelling along the edges. */
function Lineage({ nodes }: { nodes: string[] }) {
  const colours = ["#C8834A", "#B9C6D3", "#F2C14E"];
  const xs = [16, 222, 428];
  return (
    <svg viewBox="0 0 600 120" className="h-auto w-full overflow-visible" aria-hidden="true">
      {[0, 1].map((i) => (
        <g key={i}>
          <path className="lk-gv-edge lk-draw" style={{ animationDelay: `${0.3 + i * 0.3}s` }} pathLength={1} d={`M${xs[i] + 156} 60 C${xs[i] + 180} 60 ${xs[i + 1] - 24} 60 ${xs[i + 1]} 60`} fill="none" stroke="#E8F4FF" strokeOpacity="0.25" strokeWidth="2" />
          <path className="lk-gv-pulse" style={{ animationDelay: `${i * 1.1}s` }} pathLength={1} d={`M${xs[i] + 156} 60 C${xs[i] + 180} 60 ${xs[i + 1] - 24} 60 ${xs[i + 1]} 60`} fill="none" stroke="#44A6FA" strokeWidth="3" strokeLinecap="round" />
        </g>
      ))}
      {nodes.map((n, i) => (
        <g key={n} className="lk-gv-node" style={{ animationDelay: `${i * 0.3}s` }}>
          <rect x={xs[i]} y="36" width="156" height="48" rx="14" fill="#020514" fillOpacity="0.7" stroke={colours[i]} strokeOpacity="0.55" />
          <circle cx={xs[i] + 18} cy="60" r="4" fill={colours[i]} />
          {/* A column name is code: monospace (DESIGN.md §4). */}
          <text x={xs[i] + 30} y="64.5" fill="#E8F4FF" fontSize="13" fontFamily="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace">
            {n}
          </text>
        </g>
      ))}
    </svg>
  );
}

/** An e-mail cell that masks and unmasks, with the test's check. */
function Proof({ badge }: { badge: string }) {
  return (
    <div aria-hidden="true" className="flex flex-col items-start gap-4">
      <div className="relative h-10 w-full max-w-[16rem] overflow-hidden rounded-xl bg-[#020514]/70 px-3 ring-1 ring-[#E8F4FF]/10">
        <span className="lk-gv-plain absolute inset-y-0 left-3 flex items-center text-[14px] text-[#E8F4FF]">mira.okafor@example.com</span>
        <span className="lk-gv-mask absolute inset-y-0 left-3 flex items-center gap-1.5">
          <span className="lk-redact inline-block h-3.5 w-16 rounded-full" />
          <span className="text-[13px] text-[#E8F4FF]/50">@</span>
          <span className="lk-redact inline-block h-3.5 w-20 rounded-full" />
        </span>
      </div>
      <span className="lk-gv-badge inline-flex items-center gap-1.5 rounded-full bg-[#7be0b0]/10 px-2.5 py-1 text-[12px] font-medium text-[#7be0b0] ring-1 ring-[#7be0b0]/30">
        <BadgeCheck className="size-3.5" />
        {badge}
      </span>
    </div>
  );
}

/** Log rows ticking in; the risky one waits, then is approved, now and then. */
function Audit({ text }: { text: Bento["audit"] }) {
  return (
    <ul aria-hidden="true" className="flex flex-col gap-2">
      {text.rows.map((r, i) => (
        <li
          key={r}
          className="lk-gv-row flex items-center gap-2 rounded-xl bg-[#020514]/60 px-3 py-2 text-[12.5px] text-[#E8F4FF]/85 ring-1 ring-[#E8F4FF]/8"
          style={{ animationDelay: `${0.15 + i * 0.25}s` }}
        >
          <span className="size-1.5 shrink-0 rounded-full bg-[#44A6FA]" />
          <span className="min-w-0 flex-1 truncate">{r}</span>
          {i === text.rows.length - 1 ? (
            <span className="relative inline-grid shrink-0">
              <span className="lk-gv-wait col-start-1 row-start-1 inline-flex items-center gap-1 rounded-full bg-[#F2C14E]/12 px-2 py-0.5 text-[11px] font-medium text-[#F2C14E] ring-1 ring-[#F2C14E]/30">
                <span className="lk-pulse size-1.5 rounded-full bg-[#F2C14E]" />
                {text.awaiting}
              </span>
              <span className="lk-gv-ok col-start-1 row-start-1 inline-flex items-center gap-1 justify-self-end rounded-full bg-[#7be0b0]/12 px-2 py-0.5 text-[11px] font-medium text-[#7be0b0] ring-1 ring-[#7be0b0]/30">
                <Check className="size-3" />
                {text.approved}
              </span>
            </span>
          ) : (
            <Check className="size-3.5 shrink-0 text-[#E8F4FF]/45" />
          )}
        </li>
      ))}
    </ul>
  );
}

/** A search that types a table name, and its results with their marks. */
function Catalog({ text }: { text: Bento["catalog"] }) {
  return (
    <div aria-hidden="true" className="flex flex-col gap-2">
      <div className="flex h-9 items-center gap-2 rounded-xl bg-[#020514]/70 px-3 ring-1 ring-[#E8F4FF]/10">
        <Search className="size-3.5 text-[#E8F4FF]/50" />
        {/* The query is a table name, code: monospace, typed in steps of one character. */}
        <span className="lk-gv-type overflow-hidden font-mono text-[12.5px] whitespace-nowrap text-[#E8F4FF]" style={{ "--lk-n": text.query.length } as React.CSSProperties}>
          {text.query}
        </span>
        <span className="lk-gv-caret h-4 w-px bg-[#44A6FA]" />
      </div>
      <div className="lk-gv-result flex items-center gap-2 rounded-xl px-3 py-2 ring-1 ring-[#E8F4FF]/8">
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-[#E8F4FF]">gold.revenue_by_city</span>
        <span className="rounded-full bg-[#7be0b0]/12 px-2 py-0.5 text-[10.5px] font-medium text-[#7be0b0]">{text.certified}</span>
        <span className="rounded-full bg-[#E8F4FF]/8 px-2 py-0.5 text-[10.5px] text-[#E8F4FF]/75">#{text.tag}</span>
      </div>
      <div className="lk-gv-result flex items-center gap-2 rounded-xl px-3 py-2 opacity-70 ring-1 ring-[#E8F4FF]/6" style={{ animationDelay: "1.6s" }}>
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-[#E8F4FF]/70 line-through decoration-[#E8F4FF]/30">silver.revenue_old</span>
        <span className="rounded-full bg-[#F2C14E]/12 px-2 py-0.5 text-[10.5px] font-medium text-[#F2C14E]">{text.deprecated}</span>
      </div>
    </div>
  );
}

/** Folders and a dashboard; its lock opens for Admin and stays shut for Analyst (the table's switch). */
function Access({ text }: { text: Bento["access"] }) {
  return (
    <div aria-hidden="true" className="flex flex-col gap-1.5 text-[13px] text-[#E8F4FF]/85">
      {text.folders.map((f, i) => (
        <div key={f} className="flex items-center gap-2">
          <Folder className="size-4 text-[#44A6FA]/80" />
          {f}
          {i === 1 && (
            <div className="ml-auto flex -space-x-1.5">
              <span className="grid size-6 place-items-center rounded-full bg-[#44A6FA]/25 text-[10px] font-semibold text-[#bfe0ff] ring-2 ring-[#050A30]">A</span>
              <span className="grid size-6 place-items-center rounded-full bg-[#E8F4FF]/12 text-[10px] font-semibold text-[#E8F4FF]/80 ring-2 ring-[#050A30]">An</span>
            </div>
          )}
        </div>
      ))}
      <div className="ml-6 flex items-center gap-2 rounded-xl bg-[#020514]/60 px-3 py-2 ring-1 ring-[#E8F4FF]/8">
        <LayoutDashboard className="size-4 text-[#E8F4FF]/70" />
        {text.dashboard}
        <span className="relative ml-auto inline-grid">
          <span className="lk-gv-closed col-start-1 row-start-1 inline-flex items-center gap-1 text-[11.5px] text-[#8fcbff]">
            <Lock className="size-3.5" />
            {text.closed}
          </span>
          <span className="lk-gv-open col-start-1 row-start-1 inline-flex items-center gap-1 justify-self-end text-[11.5px] text-[#7be0b0]">
            <LockOpen className="size-3.5" />
            {text.open}
          </span>
        </span>
      </div>
    </div>
  );
}
