"use client"

/**
 * The pictures of the Dashboards page (`BI-59` T3, decision P3): a small
 * drawing of a dashboard's first tiles, each tile a silhouette of its chart
 * kind in its place and width, in the console's chart colours, with sample
 * shapes and no numbers.
 *
 * Drawn from chart definitions only: the page reads `GET /api/dashboard/specs`
 * once (`listChartOutlines`) and each board's saved layout from the boards
 * list it already has, so a picture runs no query. Pictures with the real
 * numbers need an API that reads only remembered results (`BI-60`).
 *
 * The built-in board (its tiles are not stored charts), a board with no
 * charts, a template (the template list carries no definition) and a report
 * (the report list carries no blocks) get a neutral picture: a glyph, or for
 * a report a page with its real title, never invented chart shapes.
 */
import { LayoutDashboard, LayoutTemplate, Package } from "lucide-react"

import { DEFAULT_PALETTE } from "@/lib/chart-palettes"
import { placeUnboxed, tabOf } from "@/lib/dashboard-tabs"
import { cn } from "@/lib/utils"
import type { Board, ChartOutline } from "@/services"
import { countLabel, type LibraryItem } from "./library-items"

/**
 * The chart colours of a dashboard: the console palette without its first
 * colour, indigo, which sits too close to the console's own blue primary in a
 * picture this small (the mockup's choice, kept).
 */
const INK = DEFAULT_PALETTE.slice(1)

export type PictureSize = "sm" | "md" | "lg"

/** Grid units of the drawing: a column is 10 wide, a row 7 high (a 44 px row beside an ~80 px column on a laptop). */
const COL = 10
const ROW = 7
const WIDTH = 12 * COL
/** Width over height of each size's box, so the drawing fills it and only its bottom is cut. */
const ASPECT: Record<Exclude<PictureSize, "sm">, number> = { md: 2.4, lg: 2.25 }

/** The charts of each board, by board id. */
export function outlinesByBoard(outlines: readonly ChartOutline[]): Map<string, ChartOutline[]> {
  const by = new Map<string, ChartOutline[]>()
  for (const o of outlines) by.set(o.board, [...(by.get(o.board) ?? []), o])
  return by
}

export type PictureTile = { id: string; kind: string; x: number; y: number; w: number; h: number }

/**
 * The tiles of a board as its page lays them out: the saved boxes, and a box
 * for each chart without one by the one shared rule (`placeUnboxed`). On a
 * tabbed board only the first tab's tiles, the ones the page opens on. Read
 * top to bottom, left to right, and moved up so the first row is at the top.
 */
export function boardTiles(board: Pick<Board, "layout" | "tabs">, charts: readonly ChartOutline[]): PictureTile[] {
  if (charts.length === 0) return []
  const tabs = board.tabs ?? []
  const layout = placeUnboxed(charts.map((c) => ({ id: c.id, span: c.span, kind: c.kind })), board.layout ?? {}, tabs)
  const first = tabs[0]?.id
  const tiles = charts
    .filter((c) => layout[c.id] && (first === undefined || tabOf(tabs, layout, c.id) === first))
    .map((c) => {
      const box = layout[c.id]
      return { id: c.id, kind: c.kind, x: box.x, y: box.y, w: box.w, h: box.h }
    })
    .sort((a, b) => a.y - b.y || a.x - b.x)
  const top = tiles.reduce((m, t) => Math.min(m, t.y), Number.POSITIVE_INFINITY)
  return tiles.map((t) => ({ ...t, y: t.y - (Number.isFinite(top) ? top : 0) }))
}

/** A repeatable sequence of numbers in [0, 1) from a chart id, so each picture is stable and differs from its neighbours. */
function seeded(id: string): () => number {
  let h = 2166136261
  for (let i = 0; i < id.length; i++) h = Math.imul(h ^ id.charCodeAt(i), 16777619)
  return () => {
    h = Math.imul(h ^ (h >>> 15), 2246822507)
    h = Math.imul(h ^ (h >>> 13), 3266489909)
    h ^= h >>> 16
    return (h >>> 0) / 4294967296
  }
}

type Area = { x: number; y: number; w: number; h: number }

/** A wandering series of `count` values in [0.08, 0.95], for a line or an area. */
function walk(rnd: () => number, count: number): number[] {
  const out: number[] = []
  let v = 0.25 + rnd() * 0.3
  for (let i = 0; i < count; i++) {
    v = Math.min(0.95, Math.max(0.08, v + (rnd() - 0.38) * 0.3))
    out.push(v)
  }
  return out
}

/** Where each slice of a ring starts, as a length along the circle. */
function starts(lengths: readonly number[]): number[] {
  const out: number[] = []
  let at = 0
  for (const len of lengths) {
    out.push(at)
    at += len
  }
  return out
}

const MUTED = "var(--muted-foreground)"

/** The silhouette of one chart kind inside `a`. Sample shapes only: no axis, no label, no number. */
function Silhouette({ kind, a, seed, stroke }: { kind: string; a: Area; seed: string; stroke: number }) {
  const rnd = seeded(seed)
  const n = (min: number, span: number) => min + rnd() * span
  switch (kind) {
    case "bar":
    case "histogram":
    case "waterfall":
    case "boxplot":
    case "stacked":
    case "combo": {
      const count = 6 + Math.floor(rnd() * 3)
      const slot = a.w / count
      const values = Array.from({ length: count }, (_, i) => Math.min(1, n(0.3, 0.45) + (i / count) * 0.3))
      const pts = values.map((v, i) => `${a.x + slot * (i + 0.5)},${a.y + a.h * (1 - v * 0.8)}`).join(" ")
      return (
        <g>
          {values.map((v, i) => {
            const h = a.h * v
            const x = a.x + slot * i + slot * 0.18
            const w = slot * 0.64
            if (kind === "stacked") {
              const lower = h * n(0.45, 0.25)
              return (
                <g key={i}>
                  <rect x={x} y={a.y + a.h - lower} width={w} height={lower} rx={0.6} fill={INK[0]} />
                  <rect x={x} y={a.y + a.h - h} width={w} height={h - lower} rx={0.6} fill={INK[1]} />
                </g>
              )
            }
            return <rect key={i} x={x} y={a.y + a.h - h} width={w} height={h} rx={0.6} fill={INK[0]} fillOpacity={kind === "combo" ? 0.45 : 0.9} />
          })}
          {kind === "combo" ? <polyline points={pts} fill="none" stroke={INK[2]} strokeWidth={stroke} strokeLinejoin="round" strokeLinecap="round" /> : null}
        </g>
      )
    }
    case "hbar":
    case "funnel":
    case "gantt": {
      const count = 4 + Math.floor(rnd() * 2)
      const slot = a.h / count
      return (
        <g>
          {Array.from({ length: count }, (_, i) => {
            const v = kind === "gantt" ? n(0.3, 0.3) : 1 - i * (0.6 / count) - rnd() * 0.08
            const w = a.w * v
            const x = kind === "funnel" ? a.x + (a.w - w) / 2 : kind === "gantt" ? a.x + n(0, a.w - w) : a.x
            return <rect key={i} x={x} y={a.y + slot * i + slot * 0.18} width={w} height={slot * 0.64} rx={0.6} fill={INK[kind === "gantt" ? i % 3 : 1]} fillOpacity={kind === "hbar" ? 0.9 - i * 0.1 : 0.9} />
          })}
        </g>
      )
    }
    case "line":
    case "area": {
      const count = 9
      const pts = walk(rnd, count).map((v, i) => [a.x + (a.w * i) / (count - 1), a.y + a.h * (1 - v)] as const)
      const line = pts.map(([x, y]) => `${x},${y}`).join(" ")
      return (
        <g>
          {kind === "area" ? <polygon points={`${a.x},${a.y + a.h} ${line} ${a.x + a.w},${a.y + a.h}`} fill={INK[0]} fillOpacity={0.18} /> : null}
          <polyline points={line} fill="none" stroke={INK[0]} strokeWidth={stroke} strokeLinejoin="round" strokeLinecap="round" />
        </g>
      )
    }
    case "pie":
    case "rose":
    case "sunburst":
    case "gauge": {
      const r = Math.min(a.w, a.h) * 0.36
      const cx = a.x + a.w / 2
      const cy = a.y + a.h / 2 + (kind === "gauge" ? r * 0.4 : 0)
      const circ = 2 * Math.PI * r
      const parts = [n(0.35, 0.2), n(0.2, 0.1), n(0.12, 0.08)]
      const rest = Math.max(0.05, 1 - parts.reduce((s, p) => s + p, 0))
      const slices = (kind === "gauge" ? [n(0.25, 0.2)] : [...parts, rest]).map((p) => p * circ)
      const offsets = starts(slices)
      return (
        <g transform={`rotate(${kind === "gauge" ? 180 : -90} ${cx} ${cy})`}>
          {kind === "gauge" ? <circle cx={cx} cy={cy} r={r} fill="none" stroke={MUTED} strokeOpacity={0.18} strokeWidth={r * 0.45} strokeDasharray={`${circ / 2} ${circ}`} /> : null}
          {slices.map((len, i) => (
            <circle key={i} cx={cx} cy={cy} r={r} fill="none" stroke={INK[i % INK.length]} strokeWidth={r * 0.45} strokeDasharray={`${len} ${circ}`} strokeDashoffset={-offsets[i]} />
          ))}
        </g>
      )
    }
    case "kpi":
    case "progress":
    case "bullet": {
      if (kind === "kpi") {
        return (
          <g>
            <rect x={a.x} y={a.y + a.h * 0.22} width={Math.min(a.w * 0.55, 30)} height={Math.max(2.4, a.h * 0.3)} rx={0.8} fill="var(--foreground)" fillOpacity={0.72} />
            <rect x={a.x} y={a.y + a.h * 0.68} width={Math.min(a.w * 0.22, 12)} height={Math.max(1.2, a.h * 0.12)} rx={0.6} fill={INK[1]} />
          </g>
        )
      }
      const y = a.y + a.h * 0.4
      const h = Math.max(1.6, a.h * 0.22)
      return (
        <g>
          <rect x={a.x} y={y} width={a.w} height={h} rx={h / 2} fill={MUTED} fillOpacity={0.18} />
          <rect x={a.x} y={y} width={a.w * n(0.4, 0.45)} height={h} rx={h / 2} fill={INK[0]} />
        </g>
      )
    }
    case "table":
    case "pivot": {
      const rows = 5
      const slot = a.h / rows
      return (
        <g>
          {Array.from({ length: rows }, (_, i) => (
            <g key={i}>
              <rect x={a.x} y={a.y + slot * i + slot * 0.3} width={a.w * 0.42} height={slot * 0.36} rx={0.4} fill={MUTED} fillOpacity={i === 0 ? 0.45 : 0.22} />
              <rect x={a.x + a.w * (0.78 - rnd() * 0.12)} y={a.y + slot * i + slot * 0.3} width={a.w * 0.22} height={slot * 0.36} rx={0.4} fill={MUTED} fillOpacity={i === 0 ? 0.45 : 0.22} />
            </g>
          ))}
        </g>
      )
    }
    case "heatmap":
    case "calendar":
    case "treemap": {
      if (kind === "treemap") {
        const split = n(0.45, 0.2)
        const split2 = n(0.4, 0.3)
        const g = 0.6
        return (
          <g>
            <rect x={a.x} y={a.y} width={a.w * split - g} height={a.h} rx={0.6} fill={INK[0]} />
            <rect x={a.x + a.w * split} y={a.y} width={a.w * (1 - split)} height={a.h * split2 - g} rx={0.6} fill={INK[1]} />
            <rect x={a.x + a.w * split} y={a.y + a.h * split2} width={a.w * (1 - split) * 0.55 - g} height={a.h * (1 - split2)} rx={0.6} fill={INK[2]} />
            <rect x={a.x + a.w * split + a.w * (1 - split) * 0.55} y={a.y + a.h * split2} width={a.w * (1 - split) * 0.45} height={a.h * (1 - split2)} rx={0.6} fill={INK[3]} />
          </g>
        )
      }
      const cols = kind === "calendar" ? 10 : 7
      const rows = kind === "calendar" ? 4 : 4
      const cw = a.w / cols
      const ch = a.h / rows
      return (
        <g>
          {Array.from({ length: cols * rows }, (_, i) => (
            <rect key={i} x={a.x + (i % cols) * cw + 0.3} y={a.y + Math.floor(i / cols) * ch + 0.3} width={cw - 0.6} height={ch - 0.6} rx={0.4} fill={INK[0]} fillOpacity={0.15 + rnd() * 0.75} />
          ))}
        </g>
      )
    }
    case "scatter":
    case "bubble":
    case "pointmap":
    case "geoheat":
    case "geomap": {
      if (kind === "geomap") {
        return (
          <g>
            {Array.from({ length: 5 }, (_, i) => (
              <ellipse key={i} cx={a.x + a.w * n(0.15, 0.7)} cy={a.y + a.h * n(0.25, 0.5)} rx={a.w * n(0.08, 0.1)} ry={a.h * n(0.12, 0.12)} fill={INK[0]} fillOpacity={0.25 + rnd() * 0.6} />
            ))}
          </g>
        )
      }
      return (
        <g>
          {Array.from({ length: 12 }, (_, i) => (
            <circle key={i} cx={a.x + a.w * n(0.04, 0.92)} cy={a.y + a.h * n(0.08, 0.84)} r={kind === "bubble" ? n(0.8, 2) : 0.9} fill={INK[i % 2]} fillOpacity={kind === "geoheat" ? 0.35 : 0.8} />
          ))}
        </g>
      )
    }
    case "radar": {
      const cx = a.x + a.w / 2
      const cy = a.y + a.h / 2
      const r = Math.min(a.w, a.h) * 0.45
      const pts = Array.from({ length: 6 }, (_, i) => {
        const ang = (Math.PI * 2 * i) / 6 - Math.PI / 2
        const v = n(0.45, 0.55)
        return `${cx + Math.cos(ang) * r * v},${cy + Math.sin(ang) * r * v}`
      }).join(" ")
      return <polygon points={pts} fill={INK[0]} fillOpacity={0.3} stroke={INK[0]} strokeWidth={stroke * 0.8} strokeLinejoin="round" />
    }
    case "sankey": {
      const bands = [0, 1, 2].map((i) => {
        const y1 = a.y + a.h * (0.12 + i * 0.3)
        const y2 = a.y + a.h * n(0.1, 0.75)
        const t = a.h * 0.16
        const mx = a.x + a.w / 2
        return (
          <path key={i} d={`M${a.x},${y1} C${mx},${y1} ${mx},${y2} ${a.x + a.w},${y2} L${a.x + a.w},${y2 + t} C${mx},${y2 + t} ${mx},${y1 + t} ${a.x},${y1 + t} Z`} fill={INK[i]} fillOpacity={0.5} />
        )
      })
      return <g>{bands}</g>
    }
    case "text":
    case "heading":
    case "section":
    case "divider":
    case "image":
    case "web":
    default: {
      if (kind === "divider") return <rect x={a.x} y={a.y + a.h / 2} width={a.w} height={0.5} fill={MUTED} fillOpacity={0.4} />
      if (kind === "image" || kind === "web") {
        return (
          <g>
            <rect x={a.x} y={a.y} width={a.w} height={a.h} rx={0.8} fill={MUTED} fillOpacity={0.12} />
            <polygon points={`${a.x + a.w * 0.2},${a.y + a.h * 0.8} ${a.x + a.w * 0.42},${a.y + a.h * 0.42} ${a.x + a.w * 0.6},${a.y + a.h * 0.8}`} fill={MUTED} fillOpacity={0.3} />
          </g>
        )
      }
      const lines = kind === "heading" || kind === "section" ? 1 : 3
      const slot = Math.min(a.h / lines, 4)
      return (
        <g>
          {Array.from({ length: lines }, (_, i) => (
            <rect key={i} x={a.x} y={a.y + slot * i + slot * 0.25} width={a.w * (i === lines - 1 && lines > 1 ? 0.6 : 0.92)} height={Math.min(slot * 0.5, lines === 1 ? 2.4 : 1.4)} rx={0.5} fill={MUTED} fillOpacity={lines === 1 ? 0.5 : 0.3} />
          ))}
        </g>
      )
    }
  }
}

/** Kinds drawn without the tile's card behind them: a heading, a section header and a rule sit on the canvas. */
const BARE = new Set(["heading", "section", "divider"])

/** One tile: its card, a title placeholder, and the silhouette of its kind. */
function TileShape({ tile, x, y, w, h, small }: { tile: Pick<PictureTile, "id" | "kind">; x: number; y: number; w: number; h: number; small?: boolean }) {
  const bare = BARE.has(tile.kind)
  const pad = small ? 2.2 : Math.min(3, Math.max(1.6, h * 0.1))
  const title = !bare && !small && h > 14
  const top = y + pad + (title ? 4 : 0)
  const area = { x: x + pad, y: top, w: Math.max(1, w - pad * 2), h: Math.max(1, y + h - pad - top) }
  return (
    <g>
      {bare ? null : <rect x={x} y={y} width={w} height={h} rx={small ? 1.5 : 2} fill="var(--card)" />}
      {title ? <rect x={x + pad} y={y + pad} width={Math.min(w * 0.4, 22)} height={1.6} rx={0.6} fill={MUTED} fillOpacity={0.35} /> : null}
      <Silhouette kind={tile.kind} a={area} seed={tile.id} stroke={small ? 1.1 : 0.9} />
    </g>
  )
}

/** The box every picture sits in: muted, rounded, cut at the edges. */
const BOX = "relative overflow-hidden rounded-lg bg-muted/70 dark:bg-muted/40"

/** A neutral picture: a glyph, and in the larger sizes a few words of what it is. */
function Neutral({ icon: Icon, words, size, className }: {
  icon: typeof Package
  words?: string
  size: PictureSize
  className?: string
}) {
  return (
    <div aria-hidden className={cn(BOX, "grid place-content-center justify-items-center gap-1 text-muted-foreground", className)}>
      <Icon className={size === "sm" ? "size-3.5" : "size-5"} />
      {size !== "sm" && words ? <span className="text-[11px]">{words}</span> : null}
    </div>
  )
}

/** A dashboard's first tiles, or for `sm` its first chart alone (a 44 px picture holds one shape that reads). */
export function BoardPicture({ board, charts, size, className }: {
  board: Pick<Board, "layout" | "tabs" | "builtin">
  /** `null` when the charts could not be read: the picture is then a glyph, not a guess. */
  charts: readonly ChartOutline[] | null
  size: PictureSize
  className?: string
}) {
  if (board.builtin) return <Neutral icon={Package} words="Built-in dashboard" size={size} className={className} />
  if (charts === null) return <Neutral icon={LayoutDashboard} size={size} className={className} />
  const tiles = boardTiles(board, charts)
  if (tiles.length === 0) return <Neutral icon={LayoutDashboard} words="No tiles yet" size={size} className={className} />
  if (size === "sm") {
    const tile = tiles.find((t) => t.kind !== "kpi" && !BARE.has(t.kind)) ?? tiles[0]
    return (
      <div aria-hidden className={cn(BOX, className)}>
        <svg viewBox="0 0 44 30" preserveAspectRatio="xMidYMid slice" className="block size-full" role="presentation">
          <TileShape tile={tile} x={3} y={3} w={38} h={24} small />
        </svg>
      </div>
    )
  }
  const height = WIDTH / ASPECT[size]
  const pad = 2
  const colW = (WIDTH - pad * 2) / 12
  const gap = 2
  return (
    <div aria-hidden className={cn(BOX, className)}>
      <svg viewBox={`0 0 ${WIDTH} ${height}`} preserveAspectRatio="xMidYMin slice" className="block size-full" role="presentation">
        {tiles
          .filter((t) => pad + t.y * ROW < height)
          .map((t) => (
            <TileShape
              key={t.id}
              tile={t}
              x={pad + t.x * colW + gap / 2}
              y={pad + t.y * ROW + gap / 2}
              w={t.w * colW - gap}
              h={t.h * ROW - gap}
            />
          ))}
      </svg>
    </div>
  )
}

/** One chart on its own (a recent chart): its silhouette in a card. */
export function ChartPicture({ chart, size, className }: { chart: ChartOutline | null; size: PictureSize; className?: string }) {
  if (!chart) return <Neutral icon={LayoutDashboard} size={size} className={className} />
  const h = size === "sm" ? 30 : WIDTH / ASPECT[size]
  const w = size === "sm" ? 44 : WIDTH
  const inset = size === "sm" ? 3 : 4
  return (
    <div aria-hidden className={cn(BOX, className)}>
      <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="xMidYMid slice" className="block size-full" role="presentation">
        <TileShape tile={chart} x={inset} y={inset} w={w - inset * 2} h={h - inset * 2} small={size === "sm"} />
      </svg>
    </div>
  )
}

/**
 * A report as a page: its real title and grey lines of text, and one grey
 * block per chart it holds (at most two). The list does not say which kinds
 * those charts are, so they are drawn without colour or shape.
 */
function ReportPicture({ item, size, className }: { item: LibraryItem; size: PictureSize; className?: string }) {
  const blocks = Math.min(item.count, 2)
  if (size === "sm") {
    return (
      <div aria-hidden className={cn(BOX, "p-1", className)}>
        <div className="flex h-full flex-col gap-[2px] rounded-[3px] bg-card p-1">
          <span className="h-[3px] w-3/4 rounded-full bg-foreground/60" />
          <span className="h-[2px] w-full rounded-full bg-muted-foreground/30" />
          {blocks > 0 ? <span className="min-h-0 flex-1 rounded-[2px] bg-muted-foreground/15" /> : <span className="h-[2px] w-2/3 rounded-full bg-muted-foreground/30" />}
        </div>
      </div>
    )
  }
  return (
    <div aria-hidden className={cn(BOX, "flex justify-center px-4 pt-3", className)}>
      <div className={cn("flex w-full max-w-[78%] flex-col gap-1 rounded-t-[5px] bg-card shadow-[0_1px_3px_rgba(0,0,0,0.08)]", size === "lg" ? "px-3 pt-2.5" : "px-2 pt-1.5")}>
        <span className={cn("truncate font-semibold text-foreground", size === "lg" ? "text-[10px] leading-3" : "text-[8px] leading-[10px]")}>{item.name}</span>
        <span className="h-[3px] w-full rounded-full bg-muted-foreground/25" />
        <span className="h-[3px] w-4/5 rounded-full bg-muted-foreground/25" />
        {Array.from({ length: blocks }, (_, i) => (
          <span key={i} className={cn("shrink-0 rounded-[3px] bg-muted-foreground/12", size === "lg" ? "h-9" : "h-6")} />
        ))}
        <span className="h-[3px] w-3/5 rounded-full bg-muted-foreground/25" />
      </div>
    </div>
  )
}

/** The picture of any row of the list. `charts` is the board's outlines, or `null` when they could not be read. */
export function ItemPicture({ item, charts, size, className }: {
  item: LibraryItem
  charts: readonly ChartOutline[] | null
  size: PictureSize
  className?: string
}) {
  if (item.kind === "report") return <ReportPicture item={item} size={size} className={className} />
  if (item.kind === "template") return <Neutral icon={LayoutTemplate} words={countLabel(item)} size={size} className={className} />
  return <BoardPicture board={item.board ?? { builtin: item.builtin }} charts={charts} size={size} className={className} />
}

/** The pictures' data, read once for the page: the outlines grouped by board, or `null` when the read failed. */
export type PictureSource = {
  readonly byBoard: ReadonlyMap<string, ChartOutline[]> | null
  readonly byId: ReadonlyMap<string, ChartOutline> | null
}

export function pictureSource(outlines: readonly ChartOutline[] | null): PictureSource {
  if (outlines === null) return { byBoard: null, byId: null }
  return { byBoard: outlinesByBoard(outlines), byId: new Map(outlines.map((o) => [o.id, o])) }
}

/** The outlines of one board from the page's source: `[]` for a board with none, `null` when unknown. */
export function chartsOf(source: PictureSource, boardId: string): ChartOutline[] | null {
  if (source.byBoard === null) return null
  return source.byBoard.get(boardId) ?? []
}
