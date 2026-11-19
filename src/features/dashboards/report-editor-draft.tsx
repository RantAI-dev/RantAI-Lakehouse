"use client"

/**
 * The assistant inside the report editor (`BI-62` PR B, decision D8; mockup
 * D's `draft.tsx` and `thread.ts`): "Ask AI to write…" (the first item of
 * `/`) and "Describe" (a chart's toolbar) put a draft in the document, the
 * answer streams into it, and Accept (Enter) turns it into blocks while
 * Discard (Esc) takes it out.
 *
 * - **One conversation.** A draft is a `send` on the page's one assistant
 *   (`useCopilot`), so the exchange shows in the AI Copilot tab by
 *   construction; the draft reads the text streaming into that answer
 *   (`useCopilotStream`). No second stream, no direct call to the chat route.
 * - **Ask mode, for that one message** (`send`'s `mode` option): the panel's
 *   own mode is left as it is.
 * - **Page context.** Each request first sets the page context with the
 *   report as Markdown and, for Describe, the chart and its rows
 *   (`buildReportEditorContext`, built by the page); the request is sent from
 *   an effect after that render, when the provider holds the new context.
 *   The context also follows the document a second after it stops changing,
 *   and is cleared when the page goes.
 * - **One draft at a time.** While a draft is open, or the assistant is
 *   answering anything, `/` → Ask AI and Describe stay visible but disabled,
 *   with the reason in their tooltip (`blocked`). An empty prompt left open
 *   does not count: the next draft replaces it.
 * - **Never saved before Accept.** The answer lives in `DraftStore`, never
 *   in the document; the node holds only its id, and `docToBlocks` skips it,
 *   so autosave sees nothing until Accept (`report-editor-extensions.ts`).
 *   Accept is one undo step; Discard (Stop first while it writes) leaves the
 *   document as it was.
 * - **Stop** ends the answer; what had streamed is not offered for Accept,
 *   because the server checks an answer's figures only when it completes
 *   (`done`), so a cut-off text is unchecked. The draft says "Stopped" with
 *   Retry. An error or a refusal is one line with Retry; nothing is inserted.
 *
 * The mark is the app's `Bot`, never a sparkle (DESIGN.md §3). With reduced
 * motion the draft does not rise or blink; the text appears as it arrives.
 */
import * as React from "react"
import { Node, type Editor } from "@tiptap/core"
import { TextSelection } from "@tiptap/pm/state"
import { NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react"
import { ArrowUp, Bot, Check, CornerDownLeft, RotateCcw, Square, X } from "lucide-react"

import type { PageContext } from "@/features/copilot/page-context"
import { NO_ANSWER_TEXT, useCopilot, useCopilotStream, type Msg } from "@/features/copilot/use-copilot"
import { describeRequest, writeRequest } from "@/lib/report-editor-context"
import { markdownToBlocks } from "@/lib/report-markdown"
import { cn } from "@/lib/utils"
import { ReportBlocks } from "./report-blocks"
import { DRAFT_NODE_SPEC, acceptDraft, draftIds, insertDraft, removeDraft } from "./report-editor-extensions"
import { EASE, FOCUS, IconTip } from "./report-editor-ui"

/* ------------------------------------------------------------------ */
/* The drafts' answers, outside the document                           */
/* ------------------------------------------------------------------ */

export type DraftPhase = "prompt" | "writing" | "done" | "error"

export type DraftRun = {
  id: string
  /** The chart a Describe is about. */
  chart: string | null
  /** The line above the text: what was asked, in the person's words. */
  label: string
  /** The message sent to the conversation; empty while the prompt is open. */
  ask: string
  phase: DraftPhase
  /** The answer once it arrived whole (Markdown). */
  text: string
  /** One line for an error, a refusal, or a stopped answer. */
  error: string | null
  /** Where the attempt's question sits in the conversation (Retry replaces a failed attempt there). */
  base: number | null
}

/** A small external store: the controller writes it, each draft's view reads its own run. */
export class DraftStore {
  private runs = new Map<string, DraftRun>()
  private list: readonly DraftRun[] = []
  private readonly listeners = new Set<() => void>()

  subscribe = (l: () => void) => {
    this.listeners.add(l)
    return () => {
      this.listeners.delete(l)
    }
  }
  get = (id: string): DraftRun | undefined => this.runs.get(id)
  all = (): readonly DraftRun[] => this.list
  put(run: DraftRun) {
    this.runs.set(run.id, run)
    this.emit()
  }
  patch(id: string, patch: Partial<Omit<DraftRun, "id">>) {
    const run = this.runs.get(id)
    if (!run) return
    this.runs.set(id, { ...run, ...patch })
    this.emit()
  }
  delete(id: string) {
    if (this.runs.delete(id)) this.emit()
  }
  private emit() {
    this.list = [...this.runs.values()]
    for (const l of this.listeners) l()
  }
}

/* ------------------------------------------------------------------ */
/* The controller                                                      */
/* ------------------------------------------------------------------ */

/** The one-line reasons `/` → Ask AI and Describe give when they cannot start. */
export const DRAFT_OPEN_REASON = "Accept or discard the open draft first"
export const DRAFT_BUSY_REASON = "Wait for AI Copilot to finish its answer"
const FAILED = "AI Copilot couldn't answer. Try again."
const STOPPED = "Stopped"
const NOTHING = "AI Copilot wrote nothing to add. Try again."

/** The three ideas under the prompt line (mockup D). */
export const PROMPT_IDEAS = ["Summarise this report", "Write an introduction", "Explain the biggest change"] as const

export type ReportDrafts = {
  store: DraftStore
  /** Why a new draft cannot start now (one line), or null. */
  blocked: string | null
  /** Whether the conversation is answering something (a prompt's Write waits). */
  busy: boolean
  /** "/" → Ask AI to write…: a prompt line where the cursor is. */
  openPrompt: () => void
  /** A chart's Describe: a draft under the chart at `pos`, written at once. */
  describe: (chartId: string, pos: number) => void
  write: (id: string, prompt: string) => void
  stop: (id: string) => void
  retry: (id: string) => void
  accept: (id: string) => void
  discard: (id: string) => void
}

let draftSeq = 0
function newDraftId(): string {
  draftSeq += 1
  return `d${Date.now().toString(36)}_${draftSeq}`
}

/** Put the cursor at the text position nearest `pos` and focus the editor. */
function focusNear(editor: Editor, pos: number) {
  editor
    .chain()
    .command(({ tr }) => {
      const at = Math.max(0, Math.min(pos, tr.doc.content.size))
      tr.setSelection(TextSelection.near(tr.doc.resolve(at)))
      return true
    })
    .focus()
    .run()
}

type Outbox = { id: string; ask: string; history?: Msg[] }
type Flight = { id: string; ask: string; base: number; done: boolean }

/**
 * The page's drafts. `context` is the page context a request is sent with,
 * for the chart being described (or none); `chartTitle` names a chart.
 */
export function useReportDrafts({
  editor,
  context,
  chartTitle,
}: {
  readonly editor: Editor | null
  readonly context: (chart: string | null) => PageContext
  readonly chartTitle: (id: string) => string
}): ReportDrafts {
  const { send, stop: stopAnswer, busy, messages, error, setPageContext } = useCopilot()
  const [store] = React.useState(() => new DraftStore())
  const runs = React.useSyncExternalStore(store.subscribe, store.all, store.all)
  // The chart the last Describe was about: it stays in the page context, so a follow-up in the panel still has its rows.
  const [chart, setChart] = React.useState<string | null>(null)
  const outbox = React.useRef<Outbox | null>(null)
  const flight = React.useRef<Flight | null>(null)
  const [outTick, setOutTick] = React.useState(0)
  const [settled, setSettled] = React.useState(0)
  const live = React.useRef({ context, chart, busy, messages, blocked: null as string | null })

  const open = runs.some((r) => r.phase !== "prompt")
  const blocked = open ? DRAFT_OPEN_REASON : busy ? DRAFT_BUSY_REASON : null
  React.useEffect(() => {
    live.current = { context, chart, busy, messages, blocked }
  })

  // The page context: now and whenever what it reads changes, and a second after the document stops changing.
  React.useEffect(() => {
    setPageContext(context(chart))
  }, [context, chart, setPageContext])
  React.useEffect(() => {
    if (!editor) return
    let timer: ReturnType<typeof setTimeout> | undefined
    const onUpdate = () => {
      clearTimeout(timer)
      timer = setTimeout(() => setPageContext(live.current.context(live.current.chart)), 1000)
    }
    editor.on("update", onUpdate)
    return () => {
      clearTimeout(timer)
      editor.off("update", onUpdate)
    }
  }, [editor, setPageContext])
  React.useEffect(() => () => setPageContext(null), [setPageContext])

  // A draft taken out some other way (selected and deleted with text around it) ends its answer.
  React.useEffect(() => {
    if (!editor) return
    const onUpdate = () => {
      const ids = draftIds(editor.state.doc)
      for (const run of store.all()) {
        if (ids.has(run.id)) continue
        if (flight.current?.id === run.id) {
          flight.current = null
          stopAnswer()
        }
        store.delete(run.id)
      }
    }
    editor.on("update", onUpdate)
    return () => {
      editor.off("update", onUpdate)
    }
  }, [editor, store, stopAnswer])

  /** An empty prompt left open is replaced by the next draft. */
  const dropPrompts = React.useCallback(
    (keep: string) => {
      if (!editor) return
      for (const run of store.all()) {
        if (run.phase !== "prompt" || run.id === keep) continue
        store.delete(run.id)
        removeDraft(editor, run.id)
      }
    },
    [editor, store],
  )

  const start = React.useCallback(
    (id: string, ask: string, chartId: string | null, history?: Msg[]) => {
      setChart(chartId)
      // The provider holds this context after the next render; the send waits for it (the effect below).
      setPageContext(live.current.context(chartId))
      store.patch(id, { phase: "writing", ask, error: null, text: "" })
      outbox.current = { id, ask, history }
      setOutTick((n) => n + 1)
    },
    [setPageContext, store],
  )

  React.useEffect(() => {
    const out = outbox.current
    if (!out) return
    outbox.current = null
    if (!store.get(out.id)) return
    if (busy) {
      store.patch(out.id, { phase: "error", error: DRAFT_BUSY_REASON })
      return
    }
    const base = (out.history ?? messages).length
    flight.current = { id: out.id, ask: out.ask, base, done: false }
    store.patch(out.id, { base })
    // BI-62 D8: Ask mode for this message only; the conversation keeps its mode.
    void send(out.ask, out.history, { mode: "ask" }).finally(() => {
      if (flight.current?.id === out.id) flight.current.done = true
      setSettled((n) => n + 1)
    })
  }, [outTick, send, busy, messages, store])

  // The answer arrived, failed, or was stopped: read it from the conversation.
  React.useEffect(() => {
    const f = flight.current
    if (!f?.done || busy) return
    flight.current = null
    if (!store.get(f.id)) return
    const asked = messages[f.base]
    const reply = messages[f.base + 1]
    if (asked?.role !== "user" || asked.content !== f.ask || reply?.role !== "assistant") {
      store.patch(f.id, { phase: "error", error: error ?? FAILED })
      return
    }
    if (reply.stopped) {
      store.patch(f.id, { phase: "error", error: STOPPED })
      return
    }
    const text = reply.content.trim()
    if (!text || text === NO_ANSWER_TEXT || markdownToBlocks(text).length === 0) {
      store.patch(f.id, { phase: "error", error: NOTHING })
      return
    }
    store.patch(f.id, { phase: "done", text })
  }, [settled, busy, messages, error, store])

  const openPrompt = React.useCallback(() => {
    if (!editor || live.current.blocked) return
    const id = newDraftId()
    store.put({ id, chart: null, label: "", ask: "", phase: "prompt", text: "", error: null, base: null })
    insertDraft(editor, { draft: id })
    dropPrompts(id)
  }, [editor, store, dropPrompts])

  const describe = React.useCallback(
    (chartId: string, pos: number) => {
      if (!editor || live.current.blocked) return
      const node = editor.state.doc.nodeAt(pos)
      if (node?.type.name !== "chart") return
      const title = chartTitle(chartId)
      const id = newDraftId()
      const after = pos + node.nodeSize
      store.put({ id, chart: chartId, label: `about ${title}`, ask: "", phase: "writing", text: "", error: null, base: null })
      insertDraft(editor, { draft: id, chart: chartId }, after)
      // The cursor moves past the draft, so the chart is not left selected (mockup D).
      focusNear(editor, after + 1)
      dropPrompts(id)
      start(id, describeRequest(title), chartId)
    },
    [editor, chartTitle, store, dropPrompts, start],
  )

  const write = React.useCallback(
    (id: string, prompt: string) => {
      const run = store.get(id)
      const words = prompt.replace(/\s+/g, " ").trim()
      if (!run || run.phase !== "prompt" || !words || live.current.blocked) return
      store.patch(id, { label: words })
      start(id, writeRequest(words), null)
    },
    [store, start],
  )

  const stop = React.useCallback(
    (id: string) => {
      if (outbox.current?.id === id) {
        outbox.current = null
        store.patch(id, { phase: "error", error: STOPPED })
      }
      if (flight.current?.id === id) stopAnswer()
    },
    [store, stopAnswer],
  )

  const retry = React.useCallback(
    (id: string) => {
      const run = store.get(id)
      if (!run || run.phase !== "error" || !run.ask || live.current.busy) return
      // The failed attempt is dropped when it is still the end of the conversation, as the panel's Retry does.
      const msgs = live.current.messages
      const tail = run.base !== null && msgs.length <= run.base + 2 && msgs[run.base]?.content === run.ask
      start(id, run.ask, run.chart, tail && run.base !== null ? msgs.slice(0, run.base) : undefined)
    },
    [store, start],
  )

  const accept = React.useCallback(
    (id: string) => {
      const run = store.get(id)
      if (!editor || run?.phase !== "done") return
      store.delete(id)
      acceptDraft(editor, id, markdownToBlocks(run.text))
      editor.commands.focus()
    },
    [editor, store],
  )

  const discard = React.useCallback(
    (id: string) => {
      if (!editor) return
      if (outbox.current?.id === id) outbox.current = null
      if (flight.current?.id === id) {
        flight.current = null
        stopAnswer()
      }
      store.delete(id)
      const removed = removeDraft(editor, id)
      if (removed) focusNear(editor, removed.pos + (removed.line ? 1 : 0))
    },
    [editor, store, stopAnswer],
  )

  return React.useMemo(
    () => ({ store, blocked, busy, openPrompt, describe, write, stop, retry, accept, discard }),
    [store, blocked, busy, openPrompt, describe, write, stop, retry, accept, discard],
  )
}

export const ReportDraftsContext = React.createContext<ReportDrafts | null>(null)

/* ------------------------------------------------------------------ */
/* The draft in the document                                           */
/* ------------------------------------------------------------------ */

const NO_CHARTS = {}
const subscribeNothing = () => () => {}

const SOFT_BTN = cn(
  "inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted-foreground hover:bg-background/80 hover:text-foreground active:scale-[0.97] dark:hover:bg-background/40",
  EASE,
  FOCUS,
)

function DraftView({ node }: ReactNodeViewProps) {
  const drafts = React.useContext(ReportDraftsContext)
  const id = typeof node.attrs.draft === "string" ? node.attrs.draft : ""
  const store = drafts?.store
  const run = React.useSyncExternalStore(
    store?.subscribe ?? subscribeNothing,
    () => store?.get(id),
    () => store?.get(id),
  )
  if (!drafts || !run) return <NodeViewWrapper data-ai-draft="" />

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== "Escape") return
    e.preventDefault()
    e.stopPropagation()
    drafts.discard(id)
  }

  return (
    <NodeViewWrapper data-ai-draft="" data-draft={id} data-phase={run.phase}>
      <div
        contentEditable={false}
        onKeyDown={onKey}
        role="group"
        aria-label="AI Copilot draft"
        className="rd-rise my-2 rounded-2xl bg-sky-500/[0.07] px-4 py-3 dark:bg-sky-400/[0.08]"
      >
        {run.phase === "prompt" ? <DraftPrompt drafts={drafts} id={id} /> : <DraftAnswer drafts={drafts} run={run} />}
      </div>
    </NodeViewWrapper>
  )
}

function DraftPrompt({ drafts, id }: { readonly drafts: ReportDrafts; readonly id: string }) {
  const [prompt, setPrompt] = React.useState("")
  const input = React.useRef<HTMLInputElement>(null)
  const inputId = React.useId()
  React.useEffect(() => {
    input.current?.focus({ preventScroll: true })
  }, [])
  const waiting = drafts.busy ? DRAFT_BUSY_REASON : null
  const submit = (
    <button
      type="submit"
      aria-label="Write"
      aria-disabled={!prompt.trim() || waiting !== null}
      className={cn(
        "grid size-7 shrink-0 place-items-center rounded-lg bg-primary text-primary-foreground hover:bg-primary/85 active:scale-[0.96] aria-disabled:bg-muted aria-disabled:text-muted-foreground aria-disabled:active:scale-100",
        EASE,
        FOCUS,
      )}
    >
      <ArrowUp className="size-4" aria-hidden />
    </button>
  )
  return (
    <form
      className="grid gap-2.5"
      onSubmit={(e) => {
        e.preventDefault()
        drafts.write(id, prompt)
      }}
    >
      <div className="flex items-center gap-2.5">
        <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-sky-500/15 text-sky-700 dark:text-sky-300">
          <Bot className="size-4" aria-hidden />
        </span>
        <label htmlFor={inputId} className="sr-only">
          Tell AI Copilot what to write
        </label>
        <input
          id={inputId}
          ref={input}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Tell AI Copilot what to write…"
          autoComplete="off"
          className="min-w-0 flex-1 bg-transparent text-[15px] leading-7 outline-none placeholder:text-muted-foreground"
        />
        {waiting ? <IconTip label={waiting}>{submit}</IconTip> : <IconTip label="Write · Enter">{submit}</IconTip>}
        <IconTip label="Discard · Esc">
          <button type="button" aria-label="Discard" onClick={() => drafts.discard(id)} className={cn(SOFT_BTN, "size-7 justify-center px-0")}>
            <X className="size-4" aria-hidden />
          </button>
        </IconTip>
      </div>
      <div className="flex flex-wrap gap-1.5 pl-9.5">
        {PROMPT_IDEAS.map((idea) => (
          <button
            key={idea}
            type="button"
            aria-disabled={waiting !== null || undefined}
            title={waiting ?? undefined}
            onClick={() => {
              if (waiting) return
              setPrompt(idea)
              drafts.write(id, idea)
            }}
            className={cn(
              "h-7 rounded-full bg-background/80 px-2.5 text-xs text-muted-foreground hover:bg-background hover:text-foreground active:scale-[0.97] aria-disabled:opacity-60 dark:bg-background/40",
              EASE,
              FOCUS,
            )}
          >
            {idea}
          </button>
        ))}
      </div>
    </form>
  )
}

function DraftAnswer({ drafts, run }: { readonly drafts: ReportDrafts; readonly run: DraftRun }) {
  const stream = useCopilotStream()
  const accept = React.useRef<HTMLButtonElement>(null)
  const writing = run.phase === "writing"
  const text = writing ? stream.draft : run.text
  // The draft shows what Accept will insert: the same parser, the same renderer (D7).
  const blocks = React.useMemo(() => (text ? markdownToBlocks(text) : []), [text])

  React.useEffect(() => {
    if (run.phase === "done") accept.current?.focus({ preventScroll: true })
  }, [run.phase])

  const status = writing ? (stream.progress?.phase === "tool" ? "Reading the data…" : "Writing…") : run.phase === "done" ? "Draft ready" : ""
  return (
    <div className="grid gap-2">
      <p className="flex min-w-0 items-center gap-1.5 text-xs leading-4 text-muted-foreground">
        <Bot className="size-3.5 shrink-0 text-sky-700 dark:text-sky-300" aria-hidden />
        <span className="truncate" title={run.label}>
          AI Copilot · {run.label}
        </span>
      </p>
      <span className="sr-only" aria-live="polite">
        {status}
      </span>
      {blocks.length ? (
        <div className="rd-draft-text" data-writing={writing || undefined} aria-busy={writing || undefined}>
          <ReportBlocks blocks={blocks} charts={NO_CHARTS} dark={false} />
        </div>
      ) : writing ? (
        <p className="text-sm leading-6 text-muted-foreground">{status}</p>
      ) : null}
      {run.phase === "error" && run.error ? (
        <p role="alert" className="truncate text-sm leading-6 text-destructive" title={run.error}>
          {run.error}
        </p>
      ) : null}
      <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
        {writing ? (
          <button type="button" onClick={() => drafts.stop(run.id)} className={SOFT_BTN}>
            <Square className="size-3" aria-hidden /> Stop
          </button>
        ) : run.phase === "done" ? (
          <>
            <button
              ref={accept}
              type="button"
              onClick={() => drafts.accept(run.id)}
              className={cn(
                "inline-flex h-7 items-center gap-1.5 rounded-lg bg-background px-2.5 text-xs font-medium text-foreground shadow-xs hover:bg-background/70 active:scale-[0.97] dark:bg-muted dark:hover:bg-muted/70",
                EASE,
                FOCUS,
              )}
            >
              <Check className="size-3.5" aria-hidden /> Accept <CornerDownLeft className="size-3 text-muted-foreground" aria-hidden />
            </button>
            <DiscardButton drafts={drafts} id={run.id} />
            <span className="ml-auto hidden text-[11px] text-muted-foreground sm:inline">Check the numbers before you keep it</span>
          </>
        ) : (
          <>
            <RetryButton drafts={drafts} id={run.id} />
            <DiscardButton drafts={drafts} id={run.id} />
          </>
        )}
      </div>
    </div>
  )
}

function DiscardButton({ drafts, id }: { readonly drafts: ReportDrafts; readonly id: string }) {
  return (
    <button type="button" onClick={() => drafts.discard(id)} className={SOFT_BTN}>
      <X className="size-3.5" aria-hidden /> Discard <span className="text-[11px] text-muted-foreground/80">Esc</span>
    </button>
  )
}

function RetryButton({ drafts, id }: { readonly drafts: ReportDrafts; readonly id: string }) {
  const button = (
    <button
      type="button"
      aria-disabled={drafts.busy || undefined}
      onClick={() => drafts.retry(id)}
      className={cn(SOFT_BTN, "text-foreground aria-disabled:opacity-60")}
    >
      <RotateCcw className="size-3.5" aria-hidden /> Retry
    </button>
  )
  return drafts.busy ? <IconTip label={DRAFT_BUSY_REASON}>{button}</IconTip> : button
}

export const DraftNode = Node.create({
  ...DRAFT_NODE_SPEC,
  addNodeView() {
    return ReactNodeViewRenderer(DraftView)
  },
})
