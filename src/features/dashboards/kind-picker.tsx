"use client";

/**
 * `BI-58` T3: the chart type above the canvas. The eight most used types are
 * icons in a row; "More" opens a gallery of every type, grouped as the
 * builder groups them (`KIND_GROUPS`), with the search the old type list had.
 *
 * Each gallery card is a small picture of the chosen data in that type,
 * drawn by the dashboard's own `TileBody` from the rows the live preview
 * already fetched (decision L2: no request of its own, at most
 * `GALLERY_ROW_CAP` rows). A type that cannot be drawn from those rows shows
 * its icon until it is picked; one the chosen source cannot make at all is
 * greyed out with one line saying why (`lib/kind-gallery`).
 */
import * as React from "react";
import { ChevronDown, MoreHorizontal, Search } from "lucide-react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import { filterKindGroups } from "@/lib/chart-kind-search";
import { GALLERY_ROW_CAP, MAIN_KINDS, drawsFrom, kindNeed, previewShape, type SourceFacts } from "@/lib/kind-gallery";
import { cn } from "@/lib/utils";
import { KIND_DESCRIPTIONS, KIND_GROUPS, KIND_KEYWORDS, KIND_LABELS, kindIcon, kindShort } from "./chart-kinds";
import { kindSample } from "@/lib/kind-sample";
import { TileBody } from "./tile-body";

const EASE = "transition-[background-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]";
const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

/** The preview the gallery draws from: its spec (with the payload as `def`) and its rows. */
export type GalleryPreview = {
  spec: ChartRenderSpec & { def?: unknown };
  columns: string[];
  rows: Record<string, unknown>[];
};

export function KindPicker({ kind, onPick, facts, preview, dark }: {
  readonly kind: ChartKind;
  readonly onPick: (next: ChartKind) => void;
  /** The chosen source's columns as the pickers count them; null before a source is chosen. */
  readonly facts: SourceFacts | null;
  /** The live preview's result, when it is rows; null otherwise. */
  readonly preview: GalleryPreview | null;
  readonly dark: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const inMain = MAIN_KINDS.includes(kind);
  const choose = (next: ChartKind) => { onPick(next); setOpen(false); };
  return (
    // `BI-58` T5: the icons wrap to a second row on a phone rather than hide "More" past the edge.
    <div role="group" aria-label="Chart type" className="flex min-w-0 flex-1 flex-wrap items-center gap-0.5">
      {MAIN_KINDS.map((k) => {
        const on = k === kind;
        const need = kindNeed(k, facts);
        return (
          <Tooltip key={k}>
            <TooltipTrigger
              render={
                <button
                  type="button" aria-pressed={on} aria-label={KIND_LABELS[k]} onClick={() => onPick(k)}
                  className={cn(
                    "grid size-8 shrink-0 place-items-center rounded-md", EASE, FOCUS,
                    on ? "bg-primary/12 text-primary dark:text-sky-300" : "text-muted-foreground hover:bg-muted hover:text-foreground",
                    need && !on && "opacity-50",
                  )}
                />
              }
            >
              <KindIcon kind={k} className="size-4" />
            </TooltipTrigger>
            <TooltipContent>{KIND_LABELS[k]} · {need ?? KIND_DESCRIPTIONS[k]}</TooltipContent>
          </Tooltip>
        );
      })}
      <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
        <PopoverPrimitive.Trigger
          aria-label={inMain ? "More chart types" : `More chart types, now ${KIND_LABELS[kind]}`}
          className={cn(
            "inline-flex h-8 min-w-0 shrink-0 items-center gap-1.5 rounded-md px-2 text-sm", EASE, FOCUS,
            !inMain ? "bg-primary/12 text-primary dark:text-sky-300" : "text-muted-foreground hover:bg-muted hover:text-foreground data-popup-open:bg-muted",
          )}
        >
          {inMain ? <MoreHorizontal aria-hidden className="size-4 shrink-0" /> : <KindIcon kind={kind} className="size-4 shrink-0" />}
          <span className="truncate">{inMain ? "More" : KIND_LABELS[kind]}</span>
          <ChevronDown aria-hidden className={cn("size-3.5 shrink-0", EASE, open && "rotate-180")} />
        </PopoverPrimitive.Trigger>
        <PopoverPrimitive.Portal>
          <PopoverPrimitive.Positioner side="bottom" align="start" sideOffset={6} collisionPadding={16} className="z-[60]">
            <PopoverPrimitive.Popup
              aria-label="Chart types"
              className={cn(
                "flex max-h-[min(40rem,75vh)] w-[min(64rem,calc(100vw-2rem))] flex-col rounded-xl border border-border bg-popover text-popover-foreground shadow-lg outline-hidden",
                "origin-(--transform-origin) transition duration-200 ease-out data-ending-style:scale-[0.98] data-ending-style:opacity-0 data-starting-style:scale-[0.98] data-starting-style:opacity-0",
              )}
            >
              <KindGallery kind={kind} onPick={choose} facts={facts} preview={preview} dark={dark} />
            </PopoverPrimitive.Popup>
          </PopoverPrimitive.Positioner>
        </PopoverPrimitive.Portal>
      </PopoverPrimitive.Root>
    </div>
  );
}

function KindGallery({ kind, onPick, facts, preview, dark }: {
  readonly kind: ChartKind;
  readonly onPick: (next: ChartKind) => void;
  readonly facts: SourceFacts | null;
  readonly preview: GalleryPreview | null;
  readonly dark: boolean;
}) {
  const [query, setQuery] = React.useState("");
  const groups = React.useMemo(() => filterKindGroups(KIND_GROUPS, KIND_DESCRIPTIONS, query, KIND_KEYWORDS), [query]);
  const def = (preview?.spec.def ?? undefined) as Record<string, unknown> | undefined;
  const shape = preview ? previewShape(preview.spec, Array.isArray(def?.layers) && def.layers.length > 0) : null;
  // Drawn at most once per open gallery: the rows are cut here, not per card.
  const rows = React.useMemo(() => preview?.rows.slice(0, GALLERY_ROW_CAP) ?? [], [preview]);
  return (
    <>
      <div className="grid gap-2 border-b border-border/60 p-3">
        <div className="relative">
          <Search aria-hidden className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query} onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              // Enter picks the first match the source can make, so "sank⏎" is enough.
              const first = groups.flatMap((g) => g.items).find((i) => !kindNeed(i.value, facts));
              if (e.key === "Enter" && first) { e.preventDefault(); onPick(first.value); }
            }}
            placeholder="Search chart types" aria-label="Search chart types" className="h-8 bg-background pl-8"
          />
        </div>
        {/* BI-58 review round 3, SHOULD-FIX 8 (owner): before any data, every picture is the sample. */}
        {preview ? null : <p className="text-xs text-muted-foreground">Pick a source and columns to see your own numbers.</p>}
      </div>
      <div className="grid min-h-0 gap-5 overflow-y-auto p-3">
        {groups.length === 0 ? <p className="px-1 py-2 text-sm text-muted-foreground">No chart type matches “{query.trim()}”.</p> : null}
        {groups.map((g) => (
          <section key={g.group} aria-label={g.group} className="grid gap-2">
            <h3 className="px-1 text-xs font-medium text-muted-foreground">{g.group}</h3>
            <ul className="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
              {g.items.map((item) => (
                <li key={item.value}>
                  <KindCard
                    kind={item.value} on={item.value === kind} need={kindNeed(item.value, facts)}
                    picture={preview && drawsFrom(item.value, shape) ? { ...preview, rows, def } : null}
                    hasData={!!preview} dark={dark} onPick={onPick}
                  />
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </>
  );
}

/** A render spec for a gallery picture: the kind, with no axes, labels or lines, which crowd it at this size. */
function pictureSpec(base: Record<string, unknown>, kind: ChartKind, def: Record<string, unknown> | undefined) {
  return {
    id: "gallery", title: "", mart: "", sql: "", source: "ui", ...base, kind,
    def: { ...def, kind, axes: { x: { show: false }, y: { show: false }, y2: { show: false } }, labels: { show: false }, referenceLines: undefined, annotations: undefined },
  } as unknown as ChartRenderSpec & { def?: unknown; text?: string };
}

function KindCard({ kind, on, need, picture, hasData, dark, onPick }: {
  readonly kind: ChartKind;
  readonly on: boolean;
  readonly need: string | null;
  /** The preview's own rows, when they can be drawn as this kind. */
  readonly picture: (GalleryPreview & { def: Record<string, unknown> | undefined }) | null;
  /** Whether the preview has rows at all: a kind drawn from the sample then is shown quieter. */
  readonly hasData: boolean;
  readonly dark: boolean;
  readonly onPick: (next: ChartKind) => void;
}) {
  const id = React.useId();
  const disabled = !!need && !on;
  const name = kindShort(kind);
  // BI-58 review round 3, SHOULD-FIX 8 (owner): always a picture. The person's rows when they can be
  // drawn as this kind; otherwise the kind's sample (lib/kind-sample), quieter when there are rows of
  // the person's own; a drawing for maps, a picture and a web page, which need a file to draw.
  const sample = React.useMemo(() => kindSample(kind), [kind]);
  const drawn = React.useMemo(() => {
    if (picture) return { spec: pictureSpec(picture.spec as unknown as Record<string, unknown>, kind, picture.def), cell: { columns: picture.columns, rows: picture.rows }, from: "data" as const };
    if (sample.kind === "illustration") return null;
    return { spec: pictureSpec(sample.spec, kind, sample.def), cell: { columns: sample.columns, rows: sample.rows }, from: "sample" as const };
  }, [picture, sample, kind]);
  return (
    <button
      type="button" aria-pressed={on} aria-label={name} aria-describedby={id} disabled={disabled} title={KIND_LABELS[kind]}
      onClick={() => onPick(kind)}
      className={cn(
        "grid w-full gap-2 overflow-hidden rounded-xl pb-2.5 text-left", EASE, FOCUS,
        on ? "bg-primary/8 ring-2 ring-primary dark:ring-sky-400" : "bg-muted/50 hover:-translate-y-0.5 hover:bg-muted hover:shadow-sm",
        "disabled:pointer-events-none disabled:opacity-50",
      )}
    >
      {/* The picture sits flush in the card's top, on the card's own fill: no box inside the card. */}
      <span
        aria-hidden data-picture={drawn?.from ?? "drawing"}
        className={cn("pointer-events-none relative block h-28 overflow-hidden", drawn?.from === "sample" && hasData && "opacity-70")}
      >
        {drawn ? (
          // Drawn at twice the size and scaled down: the chart's own margins are fixed in pixels, so at
          // card size they left the shape a sliver; scaled, it keeps the proportions of the real chart.
          <span className="absolute top-0 left-0 block h-[200%] w-[200%] origin-top-left scale-50 p-2">
            <TileBody spec={drawn.spec} cell={drawn.cell} dark={dark} loading={false} hideLegend />
          </span>
        ) : sample.kind === "illustration" ? (
          <KindDrawing picture={sample.picture} />
        ) : null}
      </span>
      <span className="grid min-w-0 gap-0.5 px-3">
        <span className="text-sm font-medium">{name}</span>
        <span id={id} className="text-xs text-muted-foreground">{need ?? KIND_DESCRIPTIONS[kind]}</span>
      </span>
    </button>
  );
}

/**
 * The pictures of the kinds that need a file or an address to draw (a map's boundaries, an image,
 * a web page): simple drawings in the theme's colours, no request.
 */
function KindDrawing({ picture }: { readonly picture: "regions" | "points" | "density" | "image" | "web" }) {
  const box = "absolute inset-0 m-auto h-[78%] w-[86%] text-primary";
  if (picture === "regions") {
    return (
      <svg viewBox="0 0 120 64" className={box}>
        {[[4, 10, 28, 20, 0.85], [34, 6, 30, 24, 0.55], [66, 12, 26, 18, 0.35], [94, 8, 22, 22, 0.7], [14, 34, 34, 22, 0.45], [52, 34, 28, 24, 0.9], [84, 34, 30, 22, 0.25]].map(([x, y, w, h, o], i) => (
          <rect key={i} x={x} y={y} width={w} height={h} rx={6} fill="currentColor" opacity={o} />
        ))}
      </svg>
    );
  }
  if (picture === "points" || picture === "density") {
    const dots = [[18, 20, 4], [30, 34, 7], [44, 22, 3], [58, 40, 9], [70, 18, 5], [84, 32, 6], [98, 44, 4], [106, 22, 3], [38, 48, 5]];
    return (
      <svg viewBox="0 0 120 64" className={box}>
        <rect x="2" y="4" width="116" height="56" rx="8" fill="currentColor" opacity="0.08" />
        {dots.map(([x, y, r], i) => (
          <circle key={i} cx={x} cy={y} r={picture === "density" ? r! * 2.4 : r} fill="currentColor" opacity={picture === "density" ? 0.22 : 0.75} />
        ))}
      </svg>
    );
  }
  if (picture === "image") {
    return (
      <svg viewBox="0 0 120 64" className={box}>
        <rect x="10" y="4" width="100" height="56" rx="6" fill="currentColor" opacity="0.12" />
        <circle cx="86" cy="20" r="7" fill="currentColor" opacity="0.6" />
        <path d="M14 56 L44 26 L66 46 L78 36 L106 56 Z" fill="currentColor" opacity="0.7" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 120 64" className={box}>
      <rect x="6" y="4" width="108" height="56" rx="6" fill="currentColor" opacity="0.12" />
      <rect x="6" y="4" width="108" height="10" rx="5" fill="currentColor" opacity="0.3" />
      {[12, 18, 24].map((x) => <circle key={x} cx={x} cy="9" r="1.8" fill="currentColor" opacity="0.8" />)}
      <rect x="14" y="22" width="56" height="6" rx="3" fill="currentColor" opacity="0.6" />
      <rect x="14" y="34" width="92" height="4" rx="2" fill="currentColor" opacity="0.3" />
      <rect x="14" y="43" width="80" height="4" rx="2" fill="currentColor" opacity="0.3" />
    </svg>
  );
}

/** A kind's icon; the icon is looked up, not created, so it is the same component on every render. */
export function KindIcon({ kind, className }: { readonly kind: ChartKind; readonly className?: string }) {
  return React.createElement(kindIcon(kind), { className, "aria-hidden": true });
}
