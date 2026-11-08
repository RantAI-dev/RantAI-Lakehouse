import * as React from "react";
import { Database, HardDriveDownload, Layers, Table2, Terminal, Zap } from "lucide-react";
import type { HomeCopy } from "@/lib/copy";

/**
 * "Open tables", drawn as the anatomy of one open table (owner QA 2026-11-08: "feels so empty"):
 * three tiers, top to bottom, joined by wires.
 * 1. The engines that read it. Only ClickHouse is claimed as built in (BLD-3); the others are
 *    examples of engines that read Apache Iceberg, under that label. Neutral icons: no licensed
 *    mark for them is in the site (DESIGN.md: never a fake logo).
 * 2. The Iceberg table (`orders`): its columns with their types, and a snapshot timeline that
 *    shows the history is kept; a snapshot's dot names its row count on hover or focus.
 * 3. Your object storage: the plain files as they sit there.
 * The query results use the depths card's illustrative numbers. Everything is markup and CSS
 * (`.lk-tb-*` in app/globals.css): the tiers draw in top-down when the picture is shown, data
 * pulses travel from the files up to each engine, and hovering an engine lights its own path
 * down to the same files. Under reduced motion it is the settled picture.
 */
const ENGINES = [
  { id: "clickhouse", name: "ClickHouse", query: "SELECT sum(amount)", result: "Rp 1.33M", icon: Zap, builtIn: true },
  { id: "duckdb", name: "DuckDB", query: "SELECT count(*)", result: "12", icon: Terminal, builtIn: false },
  { id: "spark", name: "Apache Spark", query: "SUM(amount)", result: "Rp 1.33M", icon: Layers, builtIn: false },
  { id: "trino", name: "Trino", query: "SELECT max(ts)", result: "11-04", icon: Database, builtIn: false },
] as const;
const COLUMNS = [
  ["order_id", "int"],
  ["city", "string"],
  ["amount", "decimal"],
  ["ts", "timestamp"],
] as const;
const SNAPSHOTS = [5, 9, 12] as const;
const FILES = ["metadata/v3.metadata.json", "data/00001.parquet", "data/00002.parquet", "data/00003.parquet"] as const;

export function TablesAnatomy({ copy }: { copy: HomeCopy["open"]["tables"] }) {
  return (
    <div className="lk-tb relative flex h-full min-h-[200px] flex-col justify-between gap-3 overflow-hidden p-4 sm:p-5">
      <p className="sr-only">{copy.sr}</p>
      <div aria-hidden="true" className="contents">
        {/* 1. The engines. */}
        <div className="lk-tb-tier" style={{ "--d": "0.05s" } as React.CSSProperties}>
          <p className="lk-tb-label">{copy.anyEngine}</p>
          <ul className="mt-2 grid grid-cols-2 gap-2 sm:grid-cols-4">
            {ENGINES.map((e, i) => {
              const Icon = e.icon;
              return (
                <li key={e.id} className="lk-tb-engine" data-id={e.id} style={{ "--i": i } as React.CSSProperties}>
                  <span className="flex items-center gap-1.5 text-[12px] leading-4 font-medium text-[#E8F4FF]/90">
                    <Icon className="size-3.5 shrink-0 text-[#8fcbff]" strokeWidth={2} />
                    <span className="truncate">{e.name}</span>
                  </span>
                  {e.builtIn && <span className="mt-0.5 block text-[10.5px] leading-4 text-[#E8F4FF]/50">{copy.builtIn}</span>}
                  <code className="mt-1 block truncate !text-[10.5px] !leading-4 text-[#E8F4FF]/60">{e.query}</code>
                  <span className="lk-tb-result block text-[12px] leading-4 font-medium text-[#E8F4FF] tabular-nums">{e.result}</span>
                  <span className="lk-tb-stem" />
                </li>
              );
            })}
          </ul>
        </div>

        {/* 2. The table. */}
        <div className="lk-tb-tier lk-tb-table" style={{ "--d": "0.25s" } as React.CSSProperties}>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <Table2 className="size-4 text-[#8fcbff]" strokeWidth={2} />
            <code className="!text-[13px] !leading-5 font-medium text-[#E8F4FF]">{copy.table}</code>
            <span className="rounded-full bg-[#44A6FA]/15 px-2 py-0.5 text-[11px] leading-4 font-medium text-[#8fcbff]">{copy.format}</span>
          </div>
          <ul className="mt-2.5 flex flex-wrap gap-1.5">
            {COLUMNS.map(([name, type]) => (
              <li key={name} className="rounded-md bg-white/[0.07] px-2 py-1 text-[11px] leading-4 text-[#E8F4FF]/85">
                <code className="!text-[11px] !leading-4">{name}</code> <code className="!text-[11px] !leading-4 text-[#E8F4FF]/50">{type}</code>
              </li>
            ))}
          </ul>
          <div className="mt-3 flex items-center gap-2.5">
            <span className="lk-tb-label">{copy.snapshots}</span>
            <ol className="lk-tb-snaps">
              {SNAPSHOTS.map((n, i) => (
                <li key={n} className="lk-tb-snap" data-current={i === SNAPSHOTS.length - 1 ? "" : undefined} style={{ "--i": i } as React.CSSProperties}>
                  <span className="lk-tb-dot" />
                  <span className="lk-tb-snapname">
                    s{i + 1}
                    {i === SNAPSHOTS.length - 1 && <span className="text-[#E8F4FF]/45"> · {copy.current}</span>}
                  </span>
                  <span className="lk-tb-snaptip">{copy.rows.replace("{n}", String(n))}</span>
                </li>
              ))}
            </ol>
          </div>
        </div>

        {/* 3. The files. */}
        <div className="lk-tb-tier" style={{ "--d": "0.45s" } as React.CSSProperties}>
          <p className="lk-tb-label flex items-center gap-1.5">
            <HardDriveDownload className="size-3.5" strokeWidth={2} />
            {copy.storage}
          </p>
          <ul className="mt-2 grid grid-cols-1 gap-1.5 sm:grid-cols-2">
            {FILES.map((f, i) => (
              <li key={f} className="lk-tb-file" style={{ "--i": i } as React.CSSProperties}>
                <code className="!text-[11px] !leading-4">{f}</code>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
