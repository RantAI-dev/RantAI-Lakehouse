"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import Image from "next/image"
import {
  AlertCircle,
  ArrowUp,
  ChevronLeft,
  ChevronRight,
  CircleArrowUp,
  Loader2,
  MessageSquarePlus,
  Mic,
  RefreshCw,
  Sparkles,
  Timer,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { PromptActionsMenu } from "@/components/query-studio/prompt-actions-menu"
import { SqlCodeBlock } from "@/components/query-studio/sql-code-block"
import {
  AGENT_MODEL_OPTIONS,
  type AgentModelId,
} from "@/lib/knowledge-library-fixtures"
import {
  CHAT_RUN_MS,
  NL_GENERATE_MS,
  TOPIC_PAGES,
  type ChatMessage,
  formatDurationMs,
  generateSqlFromPrompt,
  newNlId,
  nlGuardrailError,
} from "@/lib/query-studio-nl"
import { cn } from "@/lib/utils"

type WebSpeechRec = {
  lang: string
  interimResults: boolean
  maxAlternatives: number
  onresult: ((ev: { results: { 0: { 0: { transcript: string } } } }) => void) | null
  onerror: (() => void) | null
  onend: (() => void) | null
  start: () => void
  stop: () => void
}

function getSpeechRecognitionCtor(): (new () => WebSpeechRec) | null {
  if (typeof window === "undefined") return null
  const w = window as unknown as {
    SpeechRecognition?: new () => WebSpeechRec
    webkitSpeechRecognition?: new () => WebSpeechRec
  }
  return w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null
}

function useSpeechRecognition(
  onTranscript: (text: string) => void,
  onEnd: () => void
) {
  const [supported, setSupported] = useState(false)
  const [listening, setListening] = useState(false)
  const recRef = useRef<WebSpeechRec | null>(null)

  useEffect(() => {
    setSupported(!!getSpeechRecognitionCtor())
  }, [])

  const stop = useCallback(() => {
    try {
      recRef.current?.stop()
    } catch {
      /* ignore */
    }
    recRef.current = null
    setListening(false)
  }, [])

  const start = useCallback(() => {
    const SR = getSpeechRecognitionCtor()
    if (!SR) return

    if (listening) {
      stop()
      return
    }

    const rec = new SR()
    rec.lang = "en-US"
    rec.interimResults = false
    rec.maxAlternatives = 1
    rec.onresult = (event) => {
      const text = event.results[0]?.[0]?.transcript?.trim()
      if (text) onTranscript(text)
    }
    rec.onerror = () => {
      setListening(false)
      recRef.current = null
    }
    rec.onend = () => {
      setListening(false)
      recRef.current = null
      onEnd()
    }
    recRef.current = rec
    setListening(true)
    rec.start()
  }, [listening, onEnd, onTranscript, stop])

  return { supported, listening, start, stop }
}

export type NaturalLanguageChatProps = {
  /** Moves generated SQL into the SQL editor tab without running it. */
  onOpenInEditor: (sql: string) => void
  /**
   * Fired when an assistant reply is ready (success or error). Used by the
   * parent to append entries to the History sheet.
   */
  onExchange?: (userPrompt: string, assistantContent: string) => void
  /** Syncs the draft prompt so the parent can offer “Save current question”. */
  onPromptChange?: (prompt: string) => void
  /** Optional empty-state supporting copy (defaults to the Query Studio blurb). */
  emptyDescription?: string
  className?: string
}

/**
 * Chat-style Natural Language → SQL surface shared by Query Studio and
 * Collaborative Query Studio. Owns conversation state, loading / error
 * bubbles, SqlCodeBlock actions, suggested topics, and the sticky prompt bar.
 */
export function NaturalLanguageChat({
  onOpenInEditor,
  onExchange,
  onPromptChange,
  emptyDescription = "Describe the data you need in plain language. The AI drafts the SQL for you — run it here, or open it in the SQL editor to refine and save it.",
  className,
}: NaturalLanguageChatProps) {
  const [topicPage, setTopicPage] = useState(0)
  const [nlPrompt, setNlPrompt] = useState("")
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([])
  const [nlGenerating, setNlGenerating] = useState(false)

  const nlGenerateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )
  const chatTimeoutsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set())
  const chatEndRef = useRef<HTMLDivElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const [nlAgentModel, setNlAgentModel] = useState<AgentModelId>("default")
  const [nlUseIntelligenceKnowledge, setNlUseIntelligenceKnowledge] =
    useState(false)
  const [nlKnowledgeSourceIds, setNlKnowledgeSourceIds] = useState<string[]>(
    []
  )

  const setPrompt = useCallback(
    (value: string | ((prev: string) => string)) => {
      setNlPrompt((prev) => {
        const next = typeof value === "function" ? value(prev) : value
        onPromptChange?.(next)
        return next
      })
    },
    [onPromptChange]
  )

  const appendPrompt = useCallback(
    (text: string) => {
      setPrompt((prev) => (prev ? `${prev.trimEnd()} ${text}` : text))
    },
    [setPrompt]
  )

  const { supported: micSupported, listening, start: toggleMic } =
    useSpeechRecognition(appendPrompt, () => {})

  const handleUseIntelligenceKnowledge = useCallback((v: boolean) => {
    setNlUseIntelligenceKnowledge(v)
    if (!v) setNlKnowledgeSourceIds([])
  }, [])

  const toggleNlKnowledgeId = useCallback((id: string, selected: boolean) => {
    setNlKnowledgeSourceIds((prev) =>
      selected
        ? prev.includes(id)
          ? prev
          : [...prev, id]
        : prev.filter((x) => x !== id)
    )
  }, [])

  const appendSchemaMention = useCallback(
    (token: string) => {
      appendPrompt(` @${token}`)
    },
    [appendPrompt]
  )

  useEffect(() => {
    const chatTimeouts = chatTimeoutsRef.current
    return () => {
      if (nlGenerateTimeoutRef.current)
        clearTimeout(nlGenerateTimeoutRef.current)
      chatTimeouts.forEach((t) => clearTimeout(t))
      chatTimeouts.clear()
    }
  }, [])

  useEffect(() => {
    if (chatMessages.length === 0) return
    chatEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" })
  }, [chatMessages])

  const runNlWith = useCallback(
    (raw: string) => {
      const q = raw.trim()
      if (!q || nlGenerating) return

      const assistantId = newNlId()
      setChatMessages((prev) => [
        ...prev,
        { id: newNlId(), role: "user", status: "done", content: q },
        {
          id: assistantId,
          role: "assistant",
          status: "loading",
          content: "",
          sourcePrompt: q,
          variant: 0,
        },
      ])
      setPrompt("")
      setNlGenerating(true)

      nlGenerateTimeoutRef.current = setTimeout(() => {
        nlGenerateTimeoutRef.current = null
        const guard = nlGuardrailError(q)
        if (guard) {
          setChatMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, status: "error", content: guard }
                : m
            )
          )
          onExchange?.(q, guard)
        } else {
          const generated = generateSqlFromPrompt(q, 0)
          const assistantBody = `${generated.explanation}\n\n\`\`\`sql\n${generated.sql}\n\`\`\``
          setChatMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? {
                    ...m,
                    status: "done",
                    content: generated.explanation,
                    sql: generated.sql,
                    resultSet: generated.resultSet,
                    runState: "idle",
                    runResult: undefined,
                  }
                : m
            )
          )
          onExchange?.(q, assistantBody)
        }
        setNlGenerating(false)
      }, NL_GENERATE_MS)
    },
    [nlGenerating, onExchange, setPrompt]
  )

  const regenerateAssistant = useCallback(
    (messageId: string) => {
      if (nlGenerating) return
      const target = chatMessages.find((m) => m.id === messageId)
      const prompt = target?.sourcePrompt
      if (!prompt) return
      const nextVariant = (target.variant ?? 0) + 1

      setChatMessages((prev) =>
        prev.map((m) =>
          m.id === messageId
            ? {
                ...m,
                status: "loading",
                content: "",
                sql: undefined,
                resultSet: undefined,
                runState: undefined,
                runResult: undefined,
                variant: nextVariant,
              }
            : m
        )
      )
      setNlGenerating(true)

      const t = setTimeout(() => {
        chatTimeoutsRef.current.delete(t)
        const guard = nlGuardrailError(prompt)
        setChatMessages((prev) =>
          prev.map((m) => {
            if (m.id !== messageId) return m
            if (guard) return { ...m, status: "error", content: guard }
            const generated = generateSqlFromPrompt(prompt, nextVariant)
            return {
              ...m,
              status: "done",
              content: generated.explanation,
              sql: generated.sql,
              resultSet: generated.resultSet,
              runState: "idle",
              runResult: undefined,
            }
          })
        )
        setNlGenerating(false)
      }, NL_GENERATE_MS)
      chatTimeoutsRef.current.add(t)
    },
    [chatMessages, nlGenerating]
  )

  const runChatSql = useCallback(
    (messageId: string) => {
      const target = chatMessages.find((m) => m.id === messageId)
      if (!target?.sql || target.runState === "running") return
      const startedAt = Date.now()
      setChatMessages((prev) =>
        prev.map((m) =>
          m.id === messageId ? { ...m, runState: "running" } : m
        )
      )
      const t = setTimeout(() => {
        chatTimeoutsRef.current.delete(t)
        setChatMessages((prev) =>
          prev.map((m) => {
            if (m.id !== messageId) return m
            const rs = m.resultSet ?? { columns: ["label", "value"], rows: [] }
            return {
              ...m,
              runState: "done",
              runResult: { ...rs, durationMs: Date.now() - startedAt },
            }
          })
        )
      }, CHAT_RUN_MS)
      chatTimeoutsRef.current.add(t)
    },
    [chatMessages]
  )

  const startNewChat = useCallback(() => {
    if (nlGenerating) return
    setChatMessages([])
  }, [nlGenerating])

  const topics = TOPIC_PAGES[topicPage] ?? TOPIC_PAGES[0]!

  return (
    <div
      className={cn("mx-auto flex w-full max-w-4xl flex-1 flex-col", className)}
      data-name="naturalLanguage"
    >
      {chatMessages.length === 0 ? (
        <div className="flex flex-1 flex-col justify-center gap-7 py-6">
          <div className="flex flex-col items-center gap-3 text-center">
            <div className="relative size-14 shrink-0 overflow-hidden rounded-xl sm:size-16">
              <Image
                src="/rantai.png"
                alt="Rantai"
                fill
                sizes="64px"
                className="object-cover"
                priority
              />
            </div>
            <div className="space-y-1.5">
              <h2 className="font-[family-name:var(--font-montserrat)] text-xl font-semibold tracking-[-0.1px] text-primary sm:text-2xl">
                Ask your data anything
              </h2>
              <p className="mx-auto max-w-xl text-sm leading-relaxed text-muted-foreground">
                {emptyDescription}
              </p>
            </div>
          </div>

          <div className="flex flex-col gap-3.5">
            <div className="flex w-full items-start justify-between gap-3">
              <p className="text-sm font-medium text-foreground">
                Start with a topic
              </p>
              <div className="flex shrink-0 gap-2">
                <button
                  type="button"
                  aria-label="Previous topics"
                  className="flex size-5 items-center justify-center rounded-full border border-[#a8b0b8] text-primary disabled:opacity-40"
                  disabled={topicPage <= 0}
                  onClick={() => setTopicPage((p) => Math.max(0, p - 1))}
                >
                  <ChevronLeft className="size-4" />
                </button>
                <button
                  type="button"
                  aria-label="Next topics"
                  className="flex size-5 items-center justify-center rounded-full border border-[#a8b0b8] text-primary disabled:opacity-40"
                  disabled={topicPage >= TOPIC_PAGES.length - 1}
                  onClick={() =>
                    setTopicPage((p) =>
                      Math.min(TOPIC_PAGES.length - 1, p + 1)
                    )
                  }
                >
                  <ChevronRight className="size-4" />
                </button>
              </div>
            </div>

            <div className="grid min-h-[152px] gap-3.5 sm:grid-cols-2 xl:grid-cols-4">
              {topics.map((topic) => (
                <div
                  key={topic.id}
                  className="flex min-h-[152px] flex-col justify-between overflow-hidden rounded-lg bg-card shadow-[0px_4px_6px_-1px_rgba(0,0,0,0.1),0px_2px_4px_-1px_rgba(0,0,0,0.06)]"
                >
                  <div className="flex flex-col gap-2 p-3">
                    <p className="font-[family-name:var(--font-montserrat)] text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
                      {topic.title}
                    </p>
                    <p className="font-[family-name:var(--font-montserrat)] text-xs leading-4 tracking-[-0.072px] text-[#5d5d5d] dark:text-muted-foreground">
                      {topic.description}
                    </p>
                  </div>
                  <div className="px-3 pb-3">
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      className="size-10 rounded-md border-border"
                      aria-label={`Ask: ${topic.title}`}
                      disabled={nlGenerating}
                      onClick={() => runNlWith(topic.description)}
                    >
                      <ArrowUp className="size-4" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
      ) : (
        <div className="flex flex-1 flex-col gap-5 pb-6 pt-1">
          <div className="flex items-center justify-between gap-2">
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Conversation
            </p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={startNewChat}
              disabled={nlGenerating}
            >
              <MessageSquarePlus aria-hidden />
              New chat
            </Button>
          </div>

          {chatMessages.map((m) =>
            m.role === "user" ? (
              <div key={m.id} className="flex justify-end">
                <div className="max-w-[min(90%,36rem)] whitespace-pre-wrap break-words rounded-2xl rounded-br-md bg-primary px-4 py-2.5 text-sm leading-relaxed text-primary-foreground">
                  {m.content}
                </div>
              </div>
            ) : (
              <div key={m.id} className="flex items-start gap-3">
                <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border border-border bg-card">
                  <Sparkles className="size-3.5 text-primary" aria-hidden />
                </div>
                <div className="flex min-w-0 flex-1 flex-col gap-2.5">
                  {m.status === "loading" ? (
                    <div className="flex max-w-[min(100%,44rem)] items-center gap-3 rounded-xl rounded-tl-sm border border-border bg-card px-4 py-3">
                      <Loader2
                        className="size-4 shrink-0 animate-spin text-primary"
                        aria-hidden
                      />
                      <div>
                        <p className="text-sm font-medium text-primary">
                          Generating SQL…
                        </p>
                        <p className="text-xs text-muted-foreground">
                          Translating your question into a query against the
                          lakehouse.
                        </p>
                      </div>
                    </div>
                  ) : m.status === "error" ? (
                    <div className="max-w-[min(100%,44rem)] rounded-xl rounded-tl-sm border border-destructive/40 bg-destructive/5 px-4 py-3">
                      <div className="flex items-start gap-2.5">
                        <AlertCircle
                          className="mt-0.5 size-4 shrink-0 text-destructive"
                          aria-hidden
                        />
                        <div className="min-w-0 space-y-2">
                          <p className="text-sm font-medium text-destructive">
                            Couldn&apos;t generate a query
                          </p>
                          <p className="text-sm leading-relaxed text-muted-foreground">
                            {m.content}
                          </p>
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            onClick={() => regenerateAssistant(m.id)}
                            disabled={nlGenerating}
                          >
                            <RefreshCw aria-hidden />
                            Try again
                          </Button>
                        </div>
                      </div>
                    </div>
                  ) : (
                    <>
                      {m.content && (
                        <div className="max-w-[min(100%,44rem)] rounded-xl rounded-tl-sm border border-border bg-card px-3.5 py-2.5 text-sm leading-relaxed text-foreground">
                          {m.content}
                        </div>
                      )}
                      {m.sql && (
                        <SqlCodeBlock
                          sql={m.sql}
                          className="max-w-[min(100%,44rem)]"
                          running={m.runState === "running"}
                          disabled={nlGenerating}
                          onRun={() => runChatSql(m.id)}
                          onOpenInEditor={() => onOpenInEditor(m.sql!)}
                          onRegenerate={() => regenerateAssistant(m.id)}
                        />
                      )}
                      {m.runState === "running" && (
                        <div className="flex items-center gap-2 text-xs text-muted-foreground">
                          <Loader2
                            className="size-3.5 animate-spin text-primary"
                            aria-hidden
                          />
                          Running query against the lakehouse…
                        </div>
                      )}
                      {m.runResult && (
                        <div className="max-w-[min(100%,44rem)] overflow-hidden rounded-lg border border-border bg-card">
                          <div className="overflow-x-auto">
                            <Table>
                              <TableHeader>
                                <TableRow className="hover:bg-transparent">
                                  {m.runResult.columns.map((col) => (
                                    <TableHead
                                      key={col}
                                      className="font-mono text-xs first:pl-3 last:pr-3"
                                    >
                                      {col}
                                    </TableHead>
                                  ))}
                                </TableRow>
                              </TableHeader>
                              <TableBody>
                                {m.runResult.rows.map((row, ri) => (
                                  <TableRow key={ri}>
                                    {row.map((cell, ci) => (
                                      <TableCell
                                        key={ci}
                                        className={cn(
                                          "text-muted-foreground first:pl-3 last:pr-3",
                                          ci === 0 &&
                                            "font-medium text-primary"
                                        )}
                                      >
                                        {cell}
                                      </TableCell>
                                    ))}
                                  </TableRow>
                                ))}
                              </TableBody>
                            </Table>
                          </div>
                          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border bg-muted/40 px-3 py-1.5 text-[11px] text-muted-foreground">
                            <span className="font-mono tabular-nums text-foreground">
                              {m.runResult.rows.length} rows
                            </span>
                            <span aria-hidden className="text-border">
                              ·
                            </span>
                            <span className="inline-flex items-center gap-1 font-mono tabular-nums text-foreground">
                              <Timer
                                className="size-3 opacity-60"
                                aria-hidden
                              />
                              {formatDurationMs(m.runResult.durationMs)}
                            </span>
                            <span aria-hidden className="text-border">
                              ·
                            </span>
                            <span>
                              preview — open in the SQL editor for the full
                              result set
                            </span>
                          </div>
                        </div>
                      )}
                    </>
                  )}
                </div>
              </div>
            )
          )}
          <div ref={chatEndRef} aria-hidden />
        </div>
      )}

      <div
        className="sticky bottom-0 z-30 mt-auto pt-2"
        data-name="promptSection"
      >
        <div className="flex flex-col gap-2.5 rounded-xl border border-border bg-popover/95 p-3 shadow-[0_-4px_24px_rgba(0,0,0,0.10)] backdrop-blur-md sm:p-4">
          <input
            ref={fileRef}
            type="file"
            className="hidden"
            accept=".csv,.txt,.md,.json"
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (f) appendPrompt(` [Attached: ${f.name}]`)
              e.target.value = ""
            }}
          />
          {(nlAgentModel !== "default" ||
            nlUseIntelligenceKnowledge ||
            nlKnowledgeSourceIds.length > 0) && (
            <div className="flex flex-wrap items-center gap-1.5 px-1">
              {nlAgentModel !== "default" && (
                <Badge
                  variant="outline"
                  className="max-w-full truncate text-[11px] font-normal text-primary"
                >
                  Model:{" "}
                  {AGENT_MODEL_OPTIONS.find((o) => o.id === nlAgentModel)
                    ?.label ?? nlAgentModel}
                </Badge>
              )}
              {nlUseIntelligenceKnowledge && (
                <Badge
                  variant="secondary"
                  className="text-[11px] font-normal"
                >
                  Intelligence Knowledge
                  {nlKnowledgeSourceIds.length > 0
                    ? ` · ${nlKnowledgeSourceIds.length} source${
                        nlKnowledgeSourceIds.length === 1 ? "" : "s"
                      }`
                    : ""}
                </Badge>
              )}
            </div>
          )}
          <div className="flex w-full min-h-9 items-center gap-2 rounded-full border border-[#a8b0b8] bg-background px-3 py-2.5">
            <PromptActionsMenu
              disabled={nlGenerating}
              onUploadDocument={() => fileRef.current?.click()}
              agentModel={nlAgentModel}
              onAgentModelChange={setNlAgentModel}
              useIntelligenceKnowledge={nlUseIntelligenceKnowledge}
              onUseIntelligenceKnowledgeChange={
                handleUseIntelligenceKnowledge
              }
              selectedKnowledgeIds={nlKnowledgeSourceIds}
              onToggleKnowledgeId={toggleNlKnowledgeId}
              onMentionToken={appendSchemaMention}
            />
            <input
              type="text"
              value={nlPrompt}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !nlGenerating) {
                  e.preventDefault()
                  runNlWith(nlPrompt)
                }
              }}
              placeholder="Ask your data question..."
              disabled={nlGenerating}
              className="min-w-0 flex-1 bg-transparent font-[family-name:var(--font-montserrat)] text-base leading-6 text-foreground outline-none placeholder:text-[#5e6975] disabled:opacity-60 dark:placeholder:text-muted-foreground"
              aria-label="Natural language prompt"
            />
            <button
              type="button"
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted",
                listening && "bg-primary/15",
                !micSupported && "opacity-40"
              )}
              aria-label={
                micSupported
                  ? listening
                    ? "Stop dictation"
                    : "Dictate with microphone"
                  : "Microphone not supported in this browser"
              }
              disabled={!micSupported || nlGenerating}
              onClick={() => toggleMic()}
            >
              <Mic className="size-5" />
            </button>
            <button
              type="button"
              className="flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted disabled:opacity-40"
              aria-label="Submit question"
              disabled={!nlPrompt.trim() || nlGenerating}
              onClick={() => runNlWith(nlPrompt)}
            >
              {nlGenerating ? (
                <Loader2 className="size-5 animate-spin" aria-hidden />
              ) : (
                <CircleArrowUp className="size-5" />
              )}
            </button>
          </div>
          <p className="text-center font-[family-name:var(--font-montserrat)] text-xs leading-5 text-[#5d5d5d] dark:text-muted-foreground">
            Text2SQL can generate responses in Markdown, charts, CSV files, and
            execute Python code.
          </p>
        </div>
      </div>
    </div>
  )
}
