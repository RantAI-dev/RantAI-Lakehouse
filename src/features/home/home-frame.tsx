"use client"

/**
 * Home's frame and its one loading look (`WEB-17` T5).
 *
 * DESIGN.md "satu halaman, satu tampilan loading": from a refresh of `/`
 * to the data there is one placeholder, in the shape of the page. So the
 * shell's placeholder for `/` (`AppShellSkeleton`, `shell-placeholder.ts`)
 * and Home while its layout loads are built from the same pieces as Home
 * itself: the same scroller and band (`HomeFrame`), the composer's own
 * component (inert), the same row above the bento, and the bento drawn by
 * the real widgets in their loading state. Each piece is then replaced by
 * the real one in place; nothing moves.
 *
 * The Background choice lives in this browser, and the shell's placeholder
 * is rendered on the server, where it is not known. A small script in the
 * root layout (`HOME_BACKGROUND_SCRIPT`) marks `<html data-home-bg="off">`
 * before the first paint, and the placeholder's band follows that mark in
 * CSS, so an Off page never shows the band's colour. The layout is the same
 * for every Background (owner QA 2026-11-14), so nothing else depends on it.
 */
import * as React from "react"
import { Database } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { capsForMode } from "@/features/copilot/capabilities"
import { ChatComposer } from "@/features/copilot/chat-composer"
import type { Mode } from "@/features/copilot/use-copilot"
import { drawsScene, type HomeBackground } from "@/lib/home-background"
import { cn } from "@/lib/utils"
import { FocalSkeleton, NeedsAttention, PipelinesWidget, RecentChats } from "./home-widgets"
import { LakeBand, LakeBandBase } from "./lake-scene"

const BAND = "absolute inset-x-0 top-0 h-[300px] sm:h-[400px]"
/**
 * The hero's top padding, the same for every Background: Off hides the
 * art and keeps the place, so the greeting, composer and chips never move
 * (owner QA 2026-11-14: "kalo pas off jangan naik ke atas gini
 * composernya, tetap aja kek gitu tapi kosong aja backgroundnya").
 */
const TOP = "pt-[128px] sm:pt-[172px]"
/** The art fades in and out in place when the Background changes; no movement. */
const ART_FADE = "motion-safe:transition-opacity motion-safe:duration-300 motion-safe:ease-ui"
/**
 * The page's first band: the server painted its base colour already, so only the art inside fades in
 * (micro-interaction audit 2026-11-14), never the base, which would flash the plain page first.
 */
const ART_FIRST_FADE = "*:transition-opacity *:duration-[400ms] *:ease-ui starting:*:opacity-0"

/** The row between the hero and the bento: shortcuts at the left, Customize at the right. */
export const TOOLBAR_ROW = "mt-6 flex min-h-8 flex-wrap items-center gap-1"
/** The bento's box: a container, so the bento follows the room Home has, not the window. */
export const BENTO_BOX = "mt-3 @container/bento"
/** The right column's first widget spans both columns at the two-column width. */
export const ATTENTION_SPAN = "@[560px]/bento:col-span-2 @[1000px]/bento:col-span-1"

/**
 * The page: its own scroller, exactly the viewport under the navbar
 * (`--app-navbar-h`); the negative margins undo `AppFrame`'s padding so the
 * lake reaches the edges, and the content puts the padding back.
 * `relative`: without a positioned scroller, absolutely positioned
 * descendants (the band, `sr-only` labels) escape its clipping and stretch
 * the document, which then scrolls as well.
 *
 * `background`: the choice, or `null` while it is not known (the server
 * render), when the `data-home-bg` mark decides in CSS.
 */
export function HomeFrame({
  background,
  children,
  busy,
}: {
  background: HomeBackground | null
  children: React.ReactNode
  busy?: boolean
}) {
  return (
    <div
      aria-busy={busy || undefined}
      role={busy ? "status" : undefined}
      aria-label={busy ? "Loading Home" : undefined}
      data-home-frame=""
      className={cn(
        "relative -m-4 h-[calc(100svh-var(--app-navbar-h))] overflow-y-auto sm:-m-5 lg:-m-6",
        // The band's base colour, for the greeting's halo; none when there is no art.
        "[--home-scene-base:#f3eee2] dark:[--home-scene-base:#061230]",
        background === "off" && "[--home-scene-base:transparent] dark:[--home-scene-base:transparent]",
        background === null && "in-data-[home-bg=off]:[--home-scene-base:transparent]",
      )}
    >
      {background === null ? (
        <LakeBandBase className={cn(BAND, "in-data-[home-bg=off]:opacity-0")} />
      ) : (
        <SceneBand background={background} />
      )}
      <div className={cn("relative mx-auto flex max-w-[1180px] flex-col px-4 pb-6 sm:px-6", TOP)}>
        {children}
      </div>
    </div>
  )
}

/**
 * The band for a known Background. Off fades the art out and leaves the
 * band's place empty (the plain page shows through); the last art stays
 * mounted, still, so switching back fades it in again without a redraw
 * flash. A page that opens with Off never mounts the scene at all.
 *
 * Micro-interaction audit 2026-11-14 (emil; owner: switching the Background
 * cross-fades, never jumps): Dither to ASCII (or back) mounts the new art as
 * a second band over the old one and fades it in; the old one, still drawn
 * underneath, is dropped once the new one is opaque. A band that comes after
 * the page opened (from Off, or the switch) fades in whole; the band the page
 * opens with fades only its art in, over the base colour the server already
 * painted, so the first frame never flashes the plain page.
 */
type ArtLayer = { readonly art: Exclude<HomeBackground, "off">; readonly id: number; readonly later: boolean }
/** How long a new band takes to fade in (`ART_FADE`), plus a frame, before the old one goes. */
const CROSSFADE_MS = 340

function SceneBand({ background }: { background: HomeBackground }) {
  const off = !drawsScene(background)
  const [layers, setLayers] = React.useState<readonly ArtLayer[]>(() =>
    drawsScene(background) ? [{ art: background, id: 0, later: false }] : [],
  )
  const top = layers.at(-1)
  if (drawsScene(background) && top?.art !== background) {
    // Derived during render (not in an effect), so the new band is in the same commit as the choice.
    setLayers([...(top ? [top] : []), { art: background, id: (top?.id ?? -1) + 1, later: true }])
  }
  React.useEffect(() => {
    if (layers.length < 2) return
    const t = window.setTimeout(() => setLayers((l) => l.slice(-1)), CROSSFADE_MS)
    return () => window.clearTimeout(t)
  }, [layers])
  if (!top) return null
  return (
    <>
      {layers.map((layer) => {
        const isTop = layer.id === top.id
        return (
          <LakeBand
            key={layer.id}
            variant={layer.art}
            // The band under a cross-fade keeps moving until it goes; only Off holds the picture still.
            still={off}
            data-state={off ? "off" : "on"}
            className={cn(
              BAND,
              ART_FADE,
              off && "opacity-0",
              isTop && (layer.later ? "starting:opacity-0" : ART_FIRST_FADE),
            )}
          />
        )
      })}
    </>
  )
}

/** The hero's box: the greeting, the composer and the chips, centred. */
export const HERO = "mx-auto w-full max-w-[680px]"
export const GREETING = "text-center text-2xl leading-8 font-semibold tracking-[-0.02em] text-balance text-foreground"
/**
 * The greeting's words: marked for the scene's veil (`lake-scene.tsx`), with
 * a soft halo in the band's base colour, so they read over the art without
 * a hole cut in it (owner QA 2026-11-14).
 */
export const GREETING_TEXT =
  "[text-shadow:0_0_1px_var(--home-scene-base),0_0_3px_var(--home-scene-base),0_0_6px_var(--home-scene-base),0_0_12px_var(--home-scene-base)]"

/**
 * The composer's foot: what Ask reads and that it changes nothing, or that
 * Build can change things and asks first. One line, no wrap.
 */
export function ContextStrip({ mode, marts }: { mode: Mode; marts: number | null }) {
  const reads =
    marts === null
      ? "Reads the tables you can see"
      : `Reads the ${marts.toLocaleString()} ${marts === 1 ? "table" : "tables"} you can see`
  return (
    <div className="flex h-8 min-w-0 items-center gap-2 bg-muted/50 px-3.5 text-xs text-muted-foreground dark:bg-white/[0.03]">
      <Database aria-hidden className="size-3.5 shrink-0" />
      {/* Micro-interaction audit 2026-11-14: switching Ask and Build swaps the line with a short fade, so the change of what the composer will do is seen. */}
      <span key={mode} className="min-w-0 truncate transition-opacity duration-200 ease-ui starting:opacity-0">
        {mode === "build" ? "Can build dashboards, pipelines and more" : reads}
        <span aria-hidden> · </span>
        {mode === "build" ? "Asks before each change" : "Answers only, changes nothing"}
      </span>
      <span className="ml-auto hidden shrink-0 text-[11px] text-muted-foreground/80 sm:inline">Enter to send</span>
    </div>
  )
}

/**
 * The bento's two shapes. With the focal dashboard: it on the left (7 of
 * 12 columns) as tall as the column beside it, the other widgets stacked on
 * the right. Without it: the widgets share the width, up to three across.
 * Below 1000 px of room the focal dashboard takes the full width and the
 * rest go two across; below 560 px everything is one column.
 */
export function BentoFrame({ focal, children }: { focal: React.ReactNode; children: React.ReactNode }) {
  if (focal) {
    return (
      <div className="grid grid-cols-1 gap-3 @[1000px]/bento:grid-cols-12">
        <div className="min-w-0 @[1000px]/bento:col-span-7">{focal}</div>
        <div className="grid min-w-0 content-start gap-3 @[560px]/bento:grid-cols-2 @[1000px]/bento:col-span-5 @[1000px]/bento:grid-cols-1">
          {children}
        </div>
      </div>
    )
  }
  return (
    <div className="grid grid-cols-1 content-start gap-3 @[560px]/bento:grid-cols-2 @[1000px]/bento:grid-cols-3">
      {children}
    </div>
  )
}

const noop = () => {}
const ASK_CAPS = new Set(capsForMode("ask").map((c) => c.key))

/**
 * The hero before the person is known: a bar where the greeting goes (the
 * name is not known yet), the composer itself (inert, so nothing typed into
 * a page that is about to mount is lost), and bars where the chips go.
 */
export function HeroSkeleton() {
  return (
    <section aria-hidden className={HERO}>
      <div className="flex h-8 items-center justify-center">
        <Skeleton className="h-5 w-72 max-w-[80%]" />
      </div>
      <div inert className="mt-4">
        <ChatComposer
          solid
          rows={1}
          mode="ask"
          setMode={noop}
          onSend={noop}
          busy={false}
          enabledCaps={ASK_CAPS}
          toggleCap={noop}
          footer={<ContextStrip mode="ask" marts={null} />}
        />
      </div>
      <ChipsSkeleton />
    </section>
  )
}

/** Bars where the suggestion chips go, the chips' own size. */
export function ChipsSkeleton() {
  return (
    <div aria-hidden className="mt-3 flex flex-wrap items-center justify-center gap-1.5">
      {[128, 196, 172].map((w) => (
        <Skeleton key={w} className="h-7 rounded-full" style={{ width: w }} />
      ))}
    </div>
  )
}

/** The row above the bento while the person's layout loads: three shortcuts and Customize, as bars. */
export function ToolbarSkeleton() {
  return (
    <div aria-hidden className={TOOLBAR_ROW}>
      {[112, 116, 124].map((w) => (
        <span key={w} className="flex h-8 items-center gap-1.5 px-2.5">
          <Skeleton className="size-4 rounded" />
          <Skeleton className="h-3" style={{ width: w - 24 }} />
        </span>
      ))}
      <span className="ml-auto flex h-8 items-center gap-1.5 px-2.5">
        <Skeleton className="size-4 rounded" />
        <Skeleton className="h-3 w-16" />
      </span>
    </div>
  )
}

/** The default bento, every widget in its own loading look. */
export function BentoSkeleton() {
  return (
    <div aria-hidden className={BENTO_BOX}>
      <BentoFrame focal={<FocalSkeleton className="h-full min-h-[420px]" />}>
        <NeedsAttention
          status={{ tone: "loading", headline: "", parts: [] }}
          items={[]}
          checkedAt={null}
          onAsk={noop}
          className={ATTENTION_SPAN}
        />
        <RecentChats sessions={[]} read="loading" examples={[]} onTry={noop} />
        <PipelinesWidget state="loading" rows={[]} hasPipelines={false} canConnect={false} onAskWhy={noop} />
      </BentoFrame>
    </div>
  )
}

/**
 * Home while the session is checked: the shell draws this for `/`
 * (`shellPlaceholderFor`), and Home mounts into the same shapes.
 */
export function HomePlaceholder() {
  return (
    <HomeFrame background={null} busy>
      <HeroSkeleton />
      <ToolbarSkeleton />
      <BentoSkeleton />
    </HomeFrame>
  )
}
