"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { Database } from "lucide-react"

import { Button } from "@/components/ui/button"
import { canOpen } from "@/components/app-shell/nav-config"
import { useAuth } from "@/features/auth/auth-provider"
import { ChatComposer, type ChatComposerHandle } from "@/features/copilot/chat-composer"
import { useCopilot, type Mode } from "@/features/copilot/use-copilot"
import { readLastBoard } from "@/features/dashboards/last-board"
import { useService } from "@/hooks/use-service"
import { cardRead } from "@/lib/home-cards"
import { focalBoard } from "@/lib/home-focal"
import {
  CARD_IDS,
  DEFAULT_SHORTCUTS,
  hiddenIds,
  SHORTCUT_IDS,
  WIDE_CARD,
  type CardId,
} from "@/lib/home-layout"
import { pipelineRows } from "@/lib/home-pipelines"
import { homePrompts } from "@/lib/home-prompts"
import { homeStatus, type CheckState } from "@/lib/home-status"
import { cn } from "@/lib/utils"
import {
  connectorService,
  dashboardService,
  goldService,
  overviewService,
  pipelineService,
  queryService,
} from "@/services"
import type { OverviewSummary } from "@/services/contracts/overview"
import {
  CustomizeButton,
  EditableCards,
  EditableShortcuts,
  EditBar,
  EmptyHome,
  PreviewPicker,
} from "./home-customize"
import { SHORTCUTS } from "./home-shortcuts"
import { EASE, FOCUS } from "./home-ui"
import {
  FocalDashboard,
  NeedsAttention,
  PipelinesWidget,
  RecentChats,
  type AttentionItem,
  type FocalState,
} from "./home-widgets"
import { LakeBand } from "./lake-scene"
import { OpenAlertsCard } from "./open-alerts-card"
import { SavedQueriesCard } from "./saved-queries-card"
import { SourcesCard } from "./sources-card"
import { useHomeLayout } from "./use-home-layout"

/**
 * Home (`WEB-17`, mockup B): where a conversation with AI Copilot starts,
 * over the lake, with the day's work right below it.
 *
 * 1. **The hero**: the greeting and the AI Copilot composer (Ask / Build,
 *    Tools) over the lake scene (`lake-scene.tsx`), which fades into the
 *    page. The composer is the only place a conversation starts: sending
 *    opens it at `/copilot?id=…`. A one-line strip in the composer says
 *    what Ask can read (the real count of marts this person can read, from
 *    `/api/dashboard/fields`; without it the count is dropped, the
 *    sentence stays) or what Build may change. Under it, suggestion chips
 *    from `homePrompts()`; the composer's placeholder cycles through the
 *    rest so nothing is on screen twice.
 * 2. **One quiet row**: the create shortcuts the person keeps on Home
 *    (Customize decides which, none to three), compact, and Customize.
 *    They used to be three bordered buttons each with an AI button; the
 *    composer is where AI starts now.
 * 3. **The bento**, in one laptop screen: the focal dashboard drawn live
 *    (`lib/home-focal` picks it), and beside it Needs attention, Recent AI
 *    chats, Pipelines and any optional card the person added (Sources,
 *    Open alerts, Saved queries), in their saved order. It follows the
 *    space Home really has (a container query): with the AI Copilot panel
 *    open beside it, it folds to two columns rather than squeeze.
 *
 * The background is the person's: Dither (default), ASCII or Off, from
 * Customize, kept in this browser (`lib/home-background`). Off draws no band
 * and the page closes up.
 *
 * Status is not a floating pill beside the greeting any more: it is Needs
 * attention (`lib/home-status`, same semantics: a check the person may not
 * see is not named, a read that failed is never "all clear").
 *
 * **The layout is the person's.** Which cards and which shortcuts show, and
 * in what order, is saved per user on the server (`/api/home/layout`) and
 * edited in place from Customize (`home-customize.tsx`, `use-home-layout`).
 * Until it has loaded, the shortcut row is reserved but empty and the bento
 * is its loading look, so nothing is drawn and then rearranged; if it cannot
 * be loaded, Home shows the default and offers no layout editing.
 *
 * Everything is read from real services, and a read that fails says so in
 * its own widget while the others render. Whether an LLM is configured is
 * not known up front (no endpoint reports it); when it is not,
 * `POST /api/ai/chat` answers 503 and the conversation page shows that.
 */
export function HomePage() {
  const router = useRouter()
  const { user, hasPermission } = useAuth()
  const copilot = useCopilot()
  const composer = React.useRef<ChatComposerHandle>(null)
  const [pending, setPending] = React.useState<{
    text: string
    mode: Mode
  } | null>(null)

  // ADM-13 review round 1, B3 (D10.3): Home widgets whose page the person cannot open
  // are not rendered and send no request.
  const canPipelines = canOpen("/pipelines", hasPermission)
  const canAlerts = canOpen("/alerts", hasPermission)
  const canConnectors = canOpen("/connectors", hasPermission)
  const canBoards = canOpen("/dashboards", hasPermission)
  const canSaved = canOpen("/query-studio", hasPermission)
  const canCatalog = canOpen("/catalog", hasPermission)
  const canCopilot = canOpen("/copilot", hasPermission)

  // `send` closes over the current mode, so a prompt that needs Build mode
  // waits one render for `setMode` to land before it is sent.
  const { mode, send } = copilot
  React.useEffect(() => {
    if (!pending || mode !== pending.mode) return
    setPending(null)
    void send(pending.text, [])
    // Navigate only now. `send` has put the question and `busy` in place
    // (both before its first await), and the conversation page sends an
    // empty conversation straight back here.
    router.push("/copilot")
  }, [pending, mode, send, router])

  const ask = React.useCallback(
    (text: string, wanted: Mode = "ask") => {
      const q = text.trim()
      if (!q) return
      copilot.newChat()
      copilot.setMode(wanted)
      setPending({ text: q, mode: wanted })
    },
    [copilot],
  )

  // WEB-17 T6: hand a question to the composer without sending it, so the
  // person reads it (and can change it) first.
  const { setMode } = copilot
  const fill = React.useCallback(
    (text: string, wanted: Mode = "ask") => {
      setMode(wanted)
      composer.current?.fill(text)
    },
    [setMode],
  )

  const pipelines = useService(
    (signal) =>
      canPipelines
        ? pipelineService.listPipelines(signal)
        : Promise.resolve({ pipelines: [] }),
    [canPipelines],
  )
  const alerts = useService(
    (signal) =>
      canAlerts ? overviewService.listAlerts(signal) : Promise.resolve([]),
    [canAlerts],
  )
  const connectors = useService(
    (signal) =>
      canConnectors
        ? connectorService.listConnectors(signal)
        : Promise.resolve([]),
    [canConnectors],
  )
  const boards = useService(
    (signal) =>
      canBoards ? dashboardService.listBoards(signal) : Promise.resolve([]),
    [canBoards],
  )
  // The favourites, for the focal board. A failed read only drops that rule.
  const favourites = useService(
    (signal) =>
      canBoards ? dashboardService.shortcuts(signal) : Promise.resolve({ favourites: [], recent: [] }),
    [canBoards],
  )
  const saved = useService(
    (signal) =>
      canSaved ? queryService.listSaved(signal) : Promise.resolve([]),
    [canSaved],
  )
  const summary = useService<OverviewSummary | null>(
    (signal) =>
      canCatalog ? overviewService.getSummary(signal) : Promise.resolve(null),
    [canCatalog],
  )
  // The marts this person can read (the server leaves out the ones their
  // roles hide, GOV-7), for "Reads the N tables you can see". Refused or
  // failed: the strip drops the number and keeps the sentence.
  const marts = useService((signal) => goldService.listMarts(signal), [])
  // What Home shows and in what order: the saved layout, or the draft while
  // customising. Undecided until it has loaded (`phase`).
  const layout = useHomeLayout()
  const shown = layout.shown

  // Read after mount: localStorage is not there during the server render.
  const [lastBoard, setLastBoard] = React.useState<string | null>(null)
  React.useEffect(() => setLastBoard(readLastBoard()), [])

  // The whole display name: the first word alone read "Good morning,
  // Bootstrap" for the "Bootstrap Admin" account.
  const displayName = user?.name?.trim()

  const failing = canPipelines
    ? (pipelines.data?.pipelines ?? []).filter(
        (p) => p.status === "failed" || p.status === "degraded",
      )
    : []
  const open = canAlerts
    ? (alerts.data ?? []).filter((a) => a.status === "open")
    : []
  const unhealthy = canConnectors
    ? (connectors.data ?? []).filter(
        (c) => c.health === "unhealthy" || c.health === "degraded",
      )
    : []

  // Catalog datasets with no synced rows (`staleAssets`): missing data,
  // not old data. See `lib/home-status`.
  const noData = canCatalog ? (summary.data?.staleAssets ?? 0) : 0

  // Failing pipelines are counted by the status but listed in Pipelines,
  // beside their "Ask why", so they are not listed twice.
  const attention: AttentionItem[] = [
    ...(canAlerts
      ? open.map((a) => ({
          key: `a:${a.id}`,
          kind: "Alert",
          label: a.title,
          state: "open",
          href: "/alerts",
          prompt: `Explain the open alert "${a.title}" (${a.affected}) and what I should check.`,
        }))
      : []),
    ...(canConnectors
      ? unhealthy.map((c) => ({
          key: `c:${c.id}`,
          kind: "Source",
          label: c.name,
          state: c.health,
          href: "/connectors",
          prompt: `Source "${c.name}" is ${c.health}. Test it and tell me what is wrong.`,
        }))
      : []),
    // One row for all of them: the overview reports a count, not which.
    ...(canCatalog && noData > 0
      ? [
          {
            key: "datasets",
            kind: "Catalog",
            label: `${noData} ${noData === 1 ? "dataset" : "datasets"}`,
            state: "without synced data",
            href: "/catalog",
            prompt:
              "Which catalog datasets have no synced rows yet, and what feeds them?",
          },
        ]
      : []),
  ]

  // ADM-13 review round 2, R2-B1: a check is passed only when the person may
  // open its page. A check that was not made is not passed at all, so the
  // widget cannot say "all clear" about something never read; with none,
  // there is no widget.
  const status = homeStatus({
    pipelines: canPipelines
      ? { state: checkState(pipelines.status), count: failing.length }
      : undefined,
    alerts: canAlerts
      ? { state: checkState(alerts.status), count: open.length }
      : undefined,
    sources: canConnectors
      ? { state: checkState(connectors.status), count: unhealthy.length }
      : undefined,
    datasets: canCatalog
      ? { state: checkState(summary.status), count: noData }
      : undefined,
  })
  // "All clear · checked 14:20": the moment the checks last finished.
  const statusTone = status?.tone ?? null
  const [checkedAt, setCheckedAt] = React.useState<Date | null>(null)
  React.useEffect(() => {
    if (statusTone && statusTone !== "loading") setCheckedAt(new Date())
  }, [statusTone])

  const boardList = boards.data ?? []
  const pick = focalBoard({
    boards: boardList,
    chosenId: shown.previewBoardId,
    favouriteIds: (favourites.data?.favourites ?? []).filter((f) => f.kind === "board").map((f) => f.id),
    lastOpenedId: lastBoard,
  })
  const focal: FocalState =
    boards.status === "loading"
      ? { kind: "loading" }
      : boards.status === "error"
        ? { kind: "error" }
        : pick
          ? { kind: "board", id: pick.board.id, name: pick.board.name, updatedAt: pick.board.updatedAt, reason: pick.reason }
          : { kind: "empty", hasBoards: boardList.length > 0 }

  // Chips and the composer's cycling examples come from one list, so what
  // the placeholder suggests is also one click away.
  const prompts = homePrompts({
    mode: copilot.mode,
    failedPipeline: failing.find((p) => p.status === "failed")?.name,
    unhealthySource: unhealthy[0]?.name,
    lastDashboard: boardList.find((b) => b.id === lastBoard)?.name,
  })
  const askStarters = homePrompts({ mode: "ask" })

  const starts = shown.shortcuts.filter((id) => canOpen(SHORTCUTS[id].href, hasPermission))

  const pipelineList = pipelines.data?.pipelines ?? []

  const cardAllowed = React.useCallback(
    (id: CardId): boolean => {
      switch (id) {
        case "dashboard-preview":
          return canBoards
        case "pipeline-runs":
          return canPipelines
        case "sources":
          return canConnectors
        case "open-alerts":
          return canAlerts
        case "saved-queries":
          return canSaved
        case "recent":
          return canCopilot
        default:
          return false
      }
    },
    [canBoards, canPipelines, canConnectors, canAlerts, canSaved, canCopilot],
  )

  // Every card the catalogue knows, by id. The layout decides which of
  // these are shown and where; building them all is cheap (they are
  // elements, not renders).
  const cardNodes: Record<CardId, React.ReactNode> = {
    "dashboard-preview": (
      <FocalDashboard
        state={focal}
        onBuild={() => fill("Suggest a dashboard for the data we have", "build")}
        className="h-full min-h-[420px]"
      />
    ),
    recent: (
      <RecentChats
        sessions={copilot.sessions}
        read={copilot.sessionsRead}
        examples={askStarters}
        onTry={(p) => fill(p, "ask")}
      />
    ),
    "pipeline-runs": (
      <PipelinesWidget
        state={checkState(pipelines.status)}
        rows={pipelineRows(pipelineList, 4)}
        hasPipelines={pipelineList.length > 0}
        canConnect={canOpen("/connectors/create", hasPermission)}
        onAskWhy={(name) => fill(`Why did ${name} fail?`, "ask")}
      />
    ),
    sources: (
      <SourcesCard
        read={cardRead(connectors.status, connectors.error)}
        sources={connectors.data ?? []}
      />
    ),
    "open-alerts": (
      <OpenAlertsCard
        read={cardRead(alerts.status, alerts.error)}
        alerts={alerts.data ?? []}
      />
    ),
    "saved-queries": (
      <SavedQueriesCard
        read={cardRead(saved.status, saved.error)}
        queries={saved.data ?? []}
      />
    ),
  }
  const listCards = shown.cards.filter((c) => c !== WIDE_CARD && cardAllowed(c))
  const focalShown = shown.cards.includes(WIDE_CARD) && cardAllowed(WIDE_CARD)
  const customize = layout.phase === "ready" ? layout.startEditing : undefined

  const attentionNode = status ? (
    <NeedsAttention
      status={status}
      items={attention}
      checkedAt={checkedAt}
      onAsk={(p) => fill(p, "ask")}
      className={focalShown ? "@[560px]/bento:col-span-2 @[1000px]/bento:col-span-1" : "col-span-full"}
    />
  ) : null

  const background = "dither"
  const scene = true
  const martCount = marts.status === "success" && Array.isArray(marts.data) ? marts.data.length : null

  // The page is its own scroller, exactly the viewport under the navbar
  // (`--app-navbar-h`): the negative margins undo `AppFrame`'s padding so
  // the lake reaches the edges, and the content puts the padding back.
  // `relative`: without a positioned scroller, absolutely positioned
  // descendants (the band, `sr-only` labels) escape its clipping and
  // stretch the document, which then scrolls as well.
  return (
    <div className="relative -m-4 h-[calc(100svh-var(--app-navbar-h))] overflow-y-auto sm:-m-5 lg:-m-6">
      {scene ? (
        <LakeBand variant={background} className="absolute inset-x-0 top-0 h-[300px] sm:h-[400px]" />
      ) : null}
      <div
        className={cn(
          "relative mx-auto flex max-w-[1180px] flex-col px-4 pb-6 sm:px-6",
          scene ? "pt-[128px] sm:pt-[172px]" : "pt-8",
        )}
      >
        <section aria-label="AI Copilot" className="mx-auto w-full max-w-[680px]">
          <h1 className="text-center text-2xl leading-8 font-semibold tracking-[-0.02em] text-balance text-foreground">
            {greeting()}
            {displayName ? `, ${displayName}` : ""}
          </h1>
          <div className="mt-4">
            <ChatComposer
              ref={composer}
              solid
              autoFocus
              rows={1}
              mode={copilot.mode}
              setMode={copilot.setMode}
              onSend={(text) => ask(text, copilot.mode)}
              busy={false}
              enabledCaps={copilot.enabledCaps}
              toggleCap={copilot.toggleCap}
              // The chips show the first three; the placeholder cycles
              // through the rest so nothing is on screen twice.
              examples={prompts.length > 3 ? prompts.slice(3) : prompts}
              footer={<ContextStrip mode={copilot.mode} marts={martCount} />}
            />
          </div>
          <div className="mt-3 flex flex-wrap items-center justify-center gap-1.5">
            {prompts.slice(0, 3).map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => ask(p, copilot.mode)}
                className={cn(
                  "inline-flex h-7 max-w-full items-center rounded-full bg-card/90 px-3 text-[13px] leading-none text-muted-foreground shadow-xs hover:bg-card hover:text-foreground active:scale-[0.97] dark:bg-muted/80 dark:hover:bg-muted",
                  EASE,
                  FOCUS,
                )}
              >
                <span className="truncate">{p}</span>
              </button>
            ))}
          </div>
        </section>

        {/* The quiet row between the hero and the bento: the create
            shortcuts the person keeps, and Customize. Reserved while the
            layout loads, so the bento does not move when it lands. */}
        <div className="mt-6 flex min-h-8 flex-wrap items-center gap-1">
          {layout.phase === "loading" ? (
            <div aria-hidden inert className="invisible flex gap-1">
              {DEFAULT_SHORTCUTS.map((id) => (
                <ShortcutLink key={id} id={id} />
              ))}
            </div>
          ) : layout.editing ? (
            <div className="w-full">
              <EditableShortcuts
                ids={starts}
                hidden={hiddenIds(
                  SHORTCUT_IDS.filter((id) => canOpen(SHORTCUTS[id].href, hasPermission)),
                  starts,
                )}
                onReorder={layout.reorderShortcuts}
                onHide={layout.hideShortcut}
                onAdd={layout.addShortcut}
              />
            </div>
          ) : (
            <>
              {starts.map((id) => (
                <ShortcutLink key={id} id={id} />
              ))}
              {customize ? (
                <div className="ml-auto">
                  <CustomizeButton onClick={customize} />
                </div>
              ) : null}
            </>
          )}
        </div>

        <div className="mt-3 @container/bento">
          {layout.phase === "loading" ? (
            <BentoFrame focal={<FocalDashboard state={{ kind: "loading" }} className="h-full min-h-[420px]" />}>
              {attentionNode}
            </BentoFrame>
          ) : layout.editing ? (
            <>
              <EditableCards
                cards={shown.cards.filter(cardAllowed)}
                nodes={cardNodes}
                previewPicker={
                  <PreviewPicker
                    boards={canBoards && boards.status === "success" ? boardList : null}
                    value={shown.previewBoardId}
                    onChange={layout.setPreviewBoard}
                  />
                }
                onReorder={layout.reorderCards}
                onHide={layout.hideCard}
              />
              <EditBar
                hiddenCards={hiddenIds(CARD_IDS.filter(cardAllowed), shown.cards.filter(cardAllowed))}
                busy={layout.busy}
                error={layout.error}
                onAddCard={layout.addCard}
                onReset={layout.resetToDefault}
                onCancel={layout.cancel}
                onDone={layout.done}
              />
            </>
          ) : shown.cards.length === 0 && !attentionNode ? (
            <EmptyHome onCustomize={customize} />
          ) : (
            <BentoFrame focal={focalShown ? cardNodes[WIDE_CARD] : null}>
              {attentionNode}
              {listCards.map((id) => (
                <React.Fragment key={id}>{cardNodes[id]}</React.Fragment>
              ))}
            </BentoFrame>
          )}
        </div>
      </div>
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
function BentoFrame({ focal, children }: { focal: React.ReactNode; children: React.ReactNode }) {
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

/** A create shortcut, compact: the manual flow, its description as the tooltip. */
function ShortcutLink({ id }: { id: keyof typeof SHORTCUTS }) {
  const s = SHORTCUTS[id]
  const Icon = s.icon
  return (
    <Button
      variant="ghost"
      render={<Link href={s.href} />}
      title={s.description}
      className="text-muted-foreground hover:text-foreground active:scale-[0.97]"
    >
      <Icon data-icon="inline-start" />
      {s.title}
    </Button>
  )
}

/**
 * The composer's foot: what Ask reads and that it changes nothing, or that
 * Build can change things and asks first. One line, no wrap.
 */
function ContextStrip({ mode, marts }: { mode: Mode; marts: number | null }) {
  const reads =
    marts === null ? "Reads the tables you can see" : `Reads the ${marts.toLocaleString()} ${marts === 1 ? "table" : "tables"} you can see`
  return (
    <div className="flex h-8 min-w-0 items-center gap-2 bg-muted/50 px-3.5 text-xs text-muted-foreground dark:bg-white/[0.03]">
      <Database aria-hidden className="size-3.5 shrink-0" />
      <span className="min-w-0 truncate">
        {mode === "build" ? "Can build dashboards, pipelines and more" : reads}
        <span aria-hidden> · </span>
        {mode === "build" ? "Asks before each change" : "Answers only, changes nothing"}
      </span>
      <span className="ml-auto hidden shrink-0 text-[11px] text-muted-foreground/80 sm:inline">Enter to send</span>
    </div>
  )
}

/** `useService`'s status as the three states the status checks know. */
function checkState(status: string): CheckState {
  if (status === "success") return "ok"
  return status === "error" ? "error" : "loading"
}

function greeting(): string {
  const h = new Date().getHours()
  if (h < 12) return "Good morning"
  if (h < 18) return "Good afternoon"
  return "Good evening"
}
