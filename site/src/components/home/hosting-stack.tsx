import * as React from "react";
import { ArrowDown, Check, Cpu, Database, FolderArchive, GitBranch, Layers, LayoutDashboard, Lock, ShieldCheck, Waypoints } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { SourceIcon } from "@/components/shared/brand-icons";

/**
 * "Your servers or ours", drawn with the real stack (owner QA 2026-11-08): the services in
 * docker-compose.yml's core, as a grid of cards inside a boundary, the same grid in both views.
 * Two radio buttons (no script, a real keyboard control) switch the boundary's label and colour
 * (`.lk-hs:has(:checked)`, 220 ms); the grid stays where it is.
 * - Your servers: "Your network", with the data staying inside. Shown, the cards boot like a real
 *   `compose up`: each goes from "Starting" to "Started" with a check, one after another, then a
 *   terminal strip prints the command and "8 services started". Hovering a card lights the cards it
 *   talks to (`data-rel`).
 * - Run for you: "Run by RantAI for you", your sources connect in from outside, and a link says to
 *   talk to us about it. No uptime, SLA or price is claimed: nothing is, in the backlog.
 */
const SERVICES = [
  { id: "console", name: "Console", icon: LayoutDashboard, rel: "api" },
  { id: "api", name: "API", icon: Cpu, rel: "console postgres clickhouse lakekeeper openfga" },
  { id: "postgres", name: "Postgres", icon: Database, rel: "api lakekeeper openfga dagster" },
  { id: "clickhouse", name: "ClickHouse", icon: Layers, rel: "api lakekeeper rustfs dagster" },
  { id: "lakekeeper", name: "Lakekeeper", icon: Waypoints, rel: "api postgres clickhouse rustfs dagster" },
  { id: "rustfs", name: "RustFS", icon: FolderArchive, rel: "clickhouse lakekeeper dagster" },
  { id: "openfga", name: "OpenFGA", icon: ShieldCheck, rel: "api postgres" },
  { id: "dagster", name: "Dagster", icon: GitBranch, rel: "postgres clickhouse lakekeeper rustfs" },
] as const;

export function HostingStack({ copy, contactHref }: { copy: HomeCopy["open"]["hosting"]; contactHref: string }) {
  return (
    <div className="lk-hs flex h-full flex-col gap-2.5 p-3 sm:p-4">
      <fieldset className="lk-open-seg relative inline-grid w-full grid-cols-2 rounded-xl p-1 text-[13px] leading-5 font-medium">
        <legend className="sr-only">{copy.label}</legend>
        <span aria-hidden="true" className="lk-hs-thumb absolute inset-y-1 left-1 w-[calc(50%-4px)] rounded-lg" />
        <label className="lk-open-seg-opt relative z-10 cursor-pointer rounded-lg px-3 py-1.5 text-center max-lg:py-2.5">
          <input type="radio" name="lk-host" value="yours" defaultChecked className="sr-only" />
          {copy.yours}
        </label>
        <label className="lk-open-seg-opt relative z-10 cursor-pointer rounded-lg px-3 py-1.5 text-center max-lg:py-2.5">
          <input type="radio" name="lk-host" value="ours" className="sr-only" />
          {copy.ours}
        </label>
      </fieldset>

      {/* Run for you: your sources connect in from outside the boundary (reserved in both views, so the grid never moves). */}
      <div aria-hidden="true" className="lk-hs-sources flex h-6 items-center gap-2 text-[11.5px] leading-4 text-[#E8F4FF]/60 max-lg:text-[12px]">
        <span>{copy.sourcesLabel}</span>
        {(["postgresql", "mysql", "hubspot"] as const).map((id) => (
          <span key={id} className="grid size-6 place-items-center rounded-full bg-white/[0.07] text-[#E8F4FF]/80">
            <SourceIcon id={id} />
          </span>
        ))}
        <ArrowDown className="size-3.5 text-[#8fcbff]" strokeWidth={2} />
      </div>

      <div className="lk-hs-box relative flex flex-1 flex-col rounded-2xl p-2.5 sm:p-3">
        <div className="lk-hs-head flex items-center gap-1.5 text-[11.5px] leading-4 font-medium max-lg:text-[12px]">
          <Lock aria-hidden="true" className="size-3 shrink-0" strokeWidth={2.25} />
          <span className="lk-hs-name relative grid">
            <span className="lk-hs-name-yours col-start-1 row-start-1">{copy.yourNetwork}</span>
            <span className="lk-hs-name-ours col-start-1 row-start-1">{copy.ranBy}</span>
          </span>
          <span className="ml-auto grid text-right text-[#E8F4FF]/50">
            <span className="lk-hs-name-yours col-start-1 row-start-1">{copy.inside}</span>
            <span className="lk-hs-name-ours col-start-1 row-start-1">{copy.insideOurs}</span>
          </span>
        </div>
        <ul className="mt-2.5 grid flex-1 auto-rows-fr grid-cols-2 gap-1.5 sm:grid-cols-4 sm:gap-2">
          {SERVICES.map((s, i) => {
            const Icon = s.icon;
            return (
              <li key={s.id} className="lk-hs-card" data-id={s.id} data-rel={s.rel} style={{ "--i": i } as React.CSSProperties}>
                <span className="flex items-center gap-1.5 text-[12px] leading-4 font-medium text-[#E8F4FF]/90 max-lg:text-[13px]">
                  <Icon aria-hidden="true" className="size-3.5 shrink-0 text-[#8fcbff]" strokeWidth={2} />
                  {s.name}
                </span>
                {/* WEB-5 M5 (phone pass): below 1024 px a card is its name and its state; the role line goes. */}
                <span className="mt-0.5 block text-[10.5px] leading-4 text-[#E8F4FF]/55 max-lg:hidden">{copy.roles[s.id]}</span>
                <span className="lk-hs-st mt-1.5 grid text-[10.5px] leading-4 font-medium max-lg:text-[12px]">
                  <span className="lk-hs-st-a col-start-1 row-start-1 text-[#E8F4FF]/45">{copy.starting}…</span>
                  <span className="lk-hs-st-b col-start-1 row-start-1 inline-flex items-center gap-1 text-[#7be0b0]">
                    <Check aria-hidden="true" className="size-3" strokeWidth={2.75} />
                    {copy.started}
                  </span>
                </span>
              </li>
            );
          })}
        </ul>
      </div>

      <div className="lk-hs-foot grid min-h-9 items-center">
        <div aria-hidden="true" className="lk-hs-term col-start-1 row-start-1 rounded-xl bg-[var(--lk-n-bg)] px-3 py-1.5">
          <code className="block truncate !text-[11.5px] !leading-[18px] text-[#E8F4FF]/85 max-lg:!text-[12px]">
            <span className="text-[#8fcbff]/60">$</span> scripts/compose.sh up --build
          </code>
          <code className="lk-hs-done block !text-[11.5px] !leading-[18px] text-[#7be0b0] max-lg:!text-[12px]">✓ {copy.allStarted}</code>
        </div>
        <a
          href={contactHref}
          className="lk-hs-talk col-start-1 row-start-1 inline-flex h-10 items-center gap-1.5 justify-self-start rounded-full px-1 text-[13px] font-medium text-[#8fcbff] underline decoration-[#8fcbff]/30 underline-offset-4 transition-[color,text-decoration-color] duration-200 ease-out outline-none hover:text-white hover:decoration-white/60 focus-visible:ring-2 focus-visible:ring-[#44A6FA]"
        >
          {copy.talk} →
        </a>
      </div>
    </div>
  );
}
