"use client";

/**
 * `BI-58` T3: the chart type above the canvas. The eight most used types are
 * icons in a row; "More" opens a gallery of every type, grouped as the
 * builder groups them (`KIND_GROUPS`), with the search the old type list had.
 *
 * The gallery is a dialog (review round 4) with two views, cards and a
 * compact list. Every type is a small picture drawn by the dashboard's own
 * `TileBody`: from the rows the live preview already fetched when they fit
 * the type (decision L2: no request of its own, at most `GALLERY_ROW_CAP`
 * rows), else from a fixed sample (`lib/kind-sample`, round 3). A type the
 * chosen source cannot make at all is greyed out with one line saying why
 * (`lib/kind-gallery`).
 */
import * as React from "react";
import { ChevronDown, LayoutGrid, List, MoreHorizontal, Search } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import { filterKindGroups } from "@/lib/chart-kind-search";
import { GALLERY_ROW_CAP, MAIN_KINDS, drawsFrom, kindNeed, previewShape, type SourceFacts } from "@/lib/kind-gallery";
import { cn } from "@/lib/utils";
import { KIND_DESCRIPTIONS, KIND_GROUPS, KIND_KEYWORDS, KIND_LABELS, kindBlurb, kindIcon, kindShort } from "./chart-kinds";
import { kindSample } from "@/lib/kind-sample";
import { ThumbnailContext, TileBody } from "./tile-body";

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
      {MAIN_KINDS.map((k, index) => {
        const on = k === kind;
        const need = kindNeed(k, facts);
        // BI-58 review round 2, SHOULD-FIX 7 follow-up: the toolbar is one 44 px row, so in a narrow
        // canvas (the Copilot panel open beside it, a phone) the last toolbar types step back into
        // "More", which lists every type. The type in use always stays.
        const fold = on ? "grid" : index >= 6 ? "hidden @[36rem]/canvas:grid" : index >= 4 ? "hidden @[26rem]/canvas:grid" : "grid";
        return (
          <Tooltip key={k}>
            <TooltipTrigger
              render={
                <button
                  type="button" aria-pressed={on} aria-label={KIND_LABELS[k]} onClick={() => onPick(k)}
                  className={cn(
                    "size-8 shrink-0 place-items-center rounded-md", fold, EASE, FOCUS,
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
      {/* BI-58 review round 4, SHOULD-FIX 9 (owner): the gallery is a centred dialog over a dim
          backdrop (it covered the inspector as a popover). The console's Dialog traps focus, closes
          on Esc and its close button, and gives focus back to "More". */}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogTrigger
          aria-label={inMain ? "More chart types" : `More chart types, now ${KIND_LABELS[kind]}`}
          className={cn(
            "inline-flex h-8 min-w-0 shrink-0 items-center gap-1.5 rounded-md px-2 text-sm", EASE, FOCUS,
            !inMain ? "bg-primary/12 text-primary dark:text-sky-300" : "text-muted-foreground hover:bg-muted hover:text-foreground data-popup-open:bg-muted",
          )}
        >
          {inMain ? <MoreHorizontal aria-hidden className="size-4 shrink-0" /> : <KindIcon kind={kind} className="size-4 shrink-0" />}
          <span className="truncate">{inMain ? "More" : KIND_LABELS[kind]}</span>
          <ChevronDown aria-hidden className="size-3.5 shrink-0" />
        </DialogTrigger>
        <DialogContent className="flex max-h-[80vh] w-[calc(100%-2rem)] max-w-[60rem] flex-col gap-0 overflow-hidden p-0 sm:max-w-[60rem]">
          <KindGallery kind={kind} onPick={choose} facts={facts} preview={preview} dark={dark} />
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** BI-58 review round 4, SHOULD-FIX 9 (owner): the gallery's view, remembered per browser. */
type GalleryView = "cards" | "list";
const VIEW_KEY = "lh.chartBuilder.galleryView";
function readView(): GalleryView {
  try {
    return window.localStorage.getItem(VIEW_KEY) === "list" ? "list" : "cards";
  } catch {
    return "cards";
  }
}
function writeView(view: GalleryView) {
  try {
    window.localStorage.setItem(VIEW_KEY, view);
  } catch {
    // Not remembered; the choice still holds while the dialog is open.
  }
}

function KindGallery({ kind, onPick, facts, preview, dark }: {
  readonly kind: ChartKind;
  readonly onPick: (next: ChartKind) => void;
  readonly facts: SourceFacts | null;
  readonly preview: GalleryPreview | null;
  readonly dark: boolean;
}) {
  const [query, setQuery] = React.useState("");
  const [view, setView] = React.useState<GalleryView>(readView);
  const pickView = (next: GalleryView) => { setView(next); writeView(next); };
  const groups = React.useMemo(() => filterKindGroups(KIND_GROUPS, KIND_DESCRIPTIONS, query, KIND_KEYWORDS), [query]);
  const def = (preview?.spec.def ?? undefined) as Record<string, unknown> | undefined;
  const shape = preview ? previewShape(preview.spec, Array.isArray(def?.layers) && def.layers.length > 0) : null;
  // Drawn at most once per open gallery: the rows are cut here, not per card.
  const rows = React.useMemo(() => preview?.rows.slice(0, GALLERY_ROW_CAP) ?? [], [preview]);
  return (
    <>
      {/* Owner QA 2026-11-14, gallery header: the title row holds only "Chart type" and the close
          button; the search and the Cards / List switch share the next row, the search taking the
          room and the switch the search's own height (32 px). On a phone the switch stays on that
          row as two icons (their names are their accessible names and tooltips). The header stays
          put while the types scroll. */}
      <div className="grid shrink-0 gap-3 border-b border-border/60 px-5 pt-4 pb-3">
        <div className="flex min-h-8 items-center pr-10">
          <DialogTitle className="text-base font-semibold">Chart type</DialogTitle>
        </div>
        <div className="flex items-center gap-2" data-slot="gallery-search-row">
          <div className="relative min-w-0 flex-1">
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
          <div role="radiogroup" aria-label="View" className="inline-flex h-8 shrink-0 items-center rounded-lg bg-muted p-0.5">
            {([["cards", "Cards", LayoutGrid], ["list", "List", List]] as const).map(([value, label, Icon]) => (
              <button
                key={value} type="button" role="radio" aria-checked={view === value} aria-label={label} title={label} onClick={() => pickView(value)}
                className={cn(
                  "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium sm:px-2.5", EASE, FOCUS,
                  view === value ? "bg-background text-foreground shadow-sm dark:bg-input/40" : "text-muted-foreground hover:text-foreground",
                )}
              >
                <Icon aria-hidden className="size-3.5" /> <span className="hidden sm:inline">{label}</span>
              </button>
            ))}
          </div>
        </div>
        {/* BI-58 review round 3, SHOULD-FIX 8 (owner): before any data, every picture is the sample. */}
        {preview ? null : <DialogDescription className="text-xs">Pick a source and columns to see your own numbers.</DialogDescription>}
      </div>
      <div className="grid min-h-0 gap-5 overflow-y-auto px-5 pt-4 pb-5">
        {groups.length === 0 ? <p className="py-2 text-sm text-muted-foreground">No chart type matches “{query.trim()}”.</p> : null}
        {groups.map((g) => (
          <section key={g.group} aria-label={g.group} className="grid gap-2">
            <h3 className="text-xs font-medium text-muted-foreground">{g.group}</h3>
            {/* Cards: four to a row at the dialog's width, all rows the same height and gaps, so a group
                that ends with one card leaves no larger gap. List: one compact row per type. */}
            <ul className={view === "cards" ? "grid grid-cols-2 gap-2.5 sm:grid-cols-3 md:grid-cols-4" : "grid gap-1"}>
              {g.items.map((item) => (
                <li key={item.value}>
                  <KindCard
                    view={view} kind={item.value} on={item.value === kind} need={kindNeed(item.value, facts)}
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

/**
 * One type's picture. BI-58 review round 3, SHOULD-FIX 8 (owner): always a picture. The person's rows
 * when they can be drawn as this kind; otherwise the kind's sample (lib/kind-sample), quieter when
 * there are rows of the person's own; a drawing for maps, a picture and a web page, which need a file
 * to draw. The same picture is the card's and, smaller, the list row's.
 */
function KindPicture({ kind, picture, hasData, dark, className, zoom = 2 }: {
  readonly kind: ChartKind;
  /** How many times larger the chart is drawn before it is scaled down: more for a smaller picture. */
  readonly zoom?: 2 | 4;
  readonly picture: (GalleryPreview & { def: Record<string, unknown> | undefined }) | null;
  readonly hasData: boolean;
  readonly dark: boolean;
  readonly className: string;
}) {
  const sample = React.useMemo(() => kindSample(kind), [kind]);
  const drawn = React.useMemo(() => {
    if (picture) return { spec: pictureSpec(picture.spec as unknown as Record<string, unknown>, kind, picture.def), cell: { columns: picture.columns, rows: picture.rows }, from: "data" as const };
    if (sample.kind === "illustration") return null;
    return { spec: pictureSpec(sample.spec, kind, sample.def), cell: { columns: sample.columns, rows: sample.rows }, from: "sample" as const };
  }, [picture, sample, kind]);
  return (
    <span
      aria-hidden data-picture={drawn?.from ?? "drawing"}
      className={cn("pointer-events-none relative block overflow-hidden", className, drawn?.from === "sample" && hasData && "opacity-70")}
    >
      {drawn ? (
        // Drawn at twice the size and scaled down: the chart's own margins are fixed in pixels, so at
        // card size they left the shape a sliver; scaled, it keeps the proportions of the real chart.
        <span className={cn("absolute top-0 left-0 block origin-top-left p-2", zoom === 4 ? "h-[400%] w-[400%] scale-25" : "h-[200%] w-[200%] scale-50")}>
          <ThumbnailContext.Provider value>
            <TileBody spec={drawn.spec} cell={drawn.cell} dark={dark} loading={false} hideLegend />
          </ThumbnailContext.Provider>
        </span>
      ) : sample.kind === "illustration" ? (
        <KindDrawing picture={sample.picture} />
      ) : null}
    </span>
  );
}

function KindCard({ view, kind, on, need, picture, hasData, dark, onPick }: {
  readonly view: GalleryView;
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
  const words = need ?? kindBlurb(kind);
  const state = cn(
    EASE, FOCUS, "disabled:pointer-events-none disabled:opacity-50",
    on ? "bg-primary/8 ring-2 ring-primary dark:ring-sky-400" : "bg-muted/50 hover:bg-muted",
  );
  if (view === "list") {
    return (
      <button
        type="button" aria-pressed={on} aria-label={name} aria-describedby={id} disabled={disabled} title={KIND_LABELS[kind]}
        onClick={() => onPick(kind)} className={cn("flex w-full items-center gap-3 rounded-lg p-1.5 pr-3 text-left", state)}
      >
        <KindPicture kind={kind} picture={picture} hasData={hasData} dark={dark} zoom={4} className="h-11 w-[4.5rem] shrink-0 rounded-md" />
        <span className="grid min-w-0 flex-1 gap-0.5">
          <span className="text-sm font-medium">{name}</span>
          <span id={id} className={cn("text-xs", need ? "text-muted-foreground/80" : "text-muted-foreground")}>{words}</span>
        </span>
      </button>
    );
  }
  return (
    <button
      type="button" aria-pressed={on} aria-label={name} aria-describedby={id} disabled={disabled} title={KIND_LABELS[kind]}
      onClick={() => onPick(kind)}
      className={cn("grid h-full w-full content-start gap-1.5 overflow-hidden rounded-xl pb-2.5 text-left", state, !on && "hover:-translate-y-0.5 hover:shadow-sm")}
    >
      {/* The picture sits flush in the card's top, on the card's own fill: no box inside the card. */}
      <KindPicture kind={kind} picture={picture} hasData={hasData} dark={dark} className="h-24" />
      <span className="grid min-w-0 gap-0.5 px-3">
        <span className="text-sm font-medium">{name}</span>
        <span id={id} className="text-xs text-muted-foreground">{words}</span>
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
