"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"

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
import { OpenAlertsCard } from "./open-alerts-card"
import { SavedQueriesCard } from "./saved-queries-card"
import { SourcesCard } from "./sources-card"
import {
  ATTENTION_SPAN,
  BENTO_BOX,
  BentoFrame,
  BentoSkeleton,
  ContextStrip,
  GREETING,
  HERO,
  HomeFrame,
  TOOLBAR_ROW,
  ToolbarSkeleton,
} from "./home-frame"
import { useHomeBackground } from "./use-home-background"
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
  const [background, setBackground] = useHomeBackground()

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
  // The list route answers 200 with `error` set and no pipelines when it
  // could not read them; that is a failed read, not "no pipelines yet".
  const pipelinesRead: CheckState =
    pipelines.status === "success" && pipelines.data.error && pipelineList.length === 0
      ? "error"
      : checkState(pipelines.status)

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
        state={pipelinesRead}
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
      className={focalShown ? ATTENTION_SPAN : "col-span-full"}
    />
  ) : null

  const martCount = marts.status === "success" && Array.isArray(marts.data) ? marts.data.length : null

  return (
    <HomeFrame background={background}>
        <section aria-label="AI Copilot" className={HERO}>
          <h1 className={GREETING}>
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
            shortcuts the person keeps, and Customize. While the layout
            loads it is the same row as bars (one loading look, T5). */}
        {layout.phase === "loading" ? (
          <ToolbarSkeleton />
        ) : (
        <div className={TOOLBAR_ROW}>
          {layout.editing ? (
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
              {/* Customize is always offered: the Background lives in this
                  browser, so it works even where the layout cannot be saved. */}
              <div className="ml-auto">
                <CustomizeButton onEdit={customize} background={background} onBackgroundChange={setBackground} />
              </div>
            </>
          )}
        </div>
        )}

        {layout.phase === "loading" ? (
          <BentoSkeleton />
        ) : (
        <div className={BENTO_BOX}>
          {layout.editing ? (
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
        )}
    </HomeFrame>
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
