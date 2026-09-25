"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import {
  AlertTriangle,
  ArrowUp,
  BarChart3,
  GitBranch,
  MessageSquare,
  Plug,
  SearchCode,
  Sparkles,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot, type Mode } from "@/features/copilot/use-copilot"
import { useService } from "@/hooks/use-service"
import { connectorService, overviewService, pipelineService } from "@/services"

/**
 * AI-first landing page. The console is agentic-first, so the first thing
 * on screen is "ask or instruct", not a wall of platform metrics — those
 * moved to Monitoring → Health (`/health`).
 *
 * Everything under the prompt is read from real services: "Needs
 * attention" lists failed pipelines, open alerts and unhealthy sources as
 * the backend reports them, and each item hands the investigation to
 * Copilot. A section whose service fails says so instead of rendering as
 * "all clear" — an empty list is only shown for a successful empty read.
 *
 * Whether an LLM is configured is not known up front (no endpoint reports
 * it); when it is not, `POST /api/ai/chat` answers 503 and the Copilot
 * page shows that error. The manual links stay on this page for that case.
 */
export function HomePage() {
  const router = useRouter()
  const { user } = useAuth()
  const copilot = useCopilot()
  const [draft, setDraft] = React.useState("")
  const [pending, setPending] = React.useState<{ text: string; mode: Mode } | null>(null)
  const inputRef = React.useRef<HTMLTextAreaElement>(null)

  React.useEffect(() => {
    inputRef.current?.focus()
  }, [])

  // `send` closes over the current mode, so a prompt that needs Build mode
  // waits one render for `setMode` to land before it is sent.
  const { mode, send } = copilot
  React.useEffect(() => {
    if (!pending || mode !== pending.mode) return
    setPending(null)
    void send(pending.text, [])
  }, [pending, mode, send])

  const ask = React.useCallback(
    (text: string, wanted: Mode = "ask") => {
      const q = text.trim()
      if (!q) return
      copilot.newChat()
      copilot.setMode(wanted)
      setPending({ text: q, mode: wanted })
      router.push("/copilot")
    },
    [copilot, router]
  )

  const pipelines = useService((signal) => pipelineService.listPipelines(signal), [])
  const alerts = useService((signal) => overviewService.listAlerts(signal), [])
  const connectors = useService((signal) => connectorService.listConnectors(signal), [])

  const firstName = user?.name?.split(" ")[0]

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 py-4">
      <div className="flex flex-col gap-1">
        <h1 className="text-2xl font-semibold tracking-[-0.02em] text-foreground">
          {greeting()}
          {firstName ? `, ${firstName}` : ""}
        </h1>
        <p className="text-sm text-muted-foreground">
          Ask a question about your data, or tell the AI what to build.
        </p>
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault()
          ask(draft)
        }}
        className="relative rounded-xl border border-border bg-card shadow-sm focus-within:ring-2 focus-within:ring-ring/40"
      >
        <Textarea
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault()
              ask(draft)
            }
          }}
          placeholder="Ask or instruct your data…"
          aria-label="Ask AI"
          rows={3}
          className="min-h-24 resize-none border-0 bg-transparent pr-14 text-base shadow-none focus-visible:ring-0"
        />
        <Button
          type="submit"
          size="icon"
          aria-label="Send to AI"
          disabled={!draft.trim()}
          className="absolute right-3 bottom-3"
        >
          <ArrowUp className="size-4" />
        </Button>
      </form>

      <div className="flex flex-wrap gap-2">
        {suggestions(pipelines.data?.pipelines ?? []).map((s) => (
          <Button key={s} variant="outline" size="sm" onClick={() => ask(s)}>
            <Sparkles className="size-3.5 text-primary" />
            {s}
          </Button>
        ))}
      </div>

      <section className="grid gap-3 sm:grid-cols-3">
        <AiAction
          icon={GitBranch}
          title="Create a pipeline"
          description="Describe the source and target; the AI drafts it for review."
          onClick={() => ask("Help me create a new pipeline. Ask me what to ingest and where it should land.", "build")}
        />
        <AiAction
          icon={BarChart3}
          title="Dashboard from a question"
          description="Ask for a chart in plain words; it is added after you confirm."
          onClick={() => ask("Suggest a dashboard for the data we have.", "build")}
        />
        <AiAction
          icon={Plug}
          title="Connect a source"
          description="Guided setup; credentials stay as secret references."
          onClick={() => ask("Guide me through connecting a new data source.", "build")}
        />
      </section>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
        <span>Or do it yourself:</span>
        <Link href="/query-studio" className="inline-flex items-center gap-1 hover:text-foreground">
          <SearchCode className="size-3.5" /> New query
        </Link>
        <Link href="/pipelines" className="inline-flex items-center gap-1 hover:text-foreground">
          <GitBranch className="size-3.5" /> Pipelines
        </Link>
        <Link href="/connectors" className="inline-flex items-center gap-1 hover:text-foreground">
          <Plug className="size-3.5" /> Sources
        </Link>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <AlertTriangle className="size-4 text-amber-500" /> Needs attention
            </CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <AttentionList
              loading={
                pipelines.status === "loading" ||
                alerts.status === "loading" ||
                connectors.status === "loading"
              }
              failedReads={[
                pipelines.status === "error" ? "pipelines" : null,
                alerts.status === "error" ? "alerts" : null,
                connectors.status === "error" ? "sources" : null,
              ].filter((x): x is string => x !== null)}
              items={[
                ...(pipelines.data?.pipelines ?? [])
                  .filter((p) => p.status === "failed" || p.status === "degraded")
                  .map((p) => ({
                    key: `p:${p.id}`,
                    label: `Pipeline ${p.name} is ${p.status}`,
                    href: `/pipelines/${encodeURIComponent(p.id)}`,
                    prompt: `Why is pipeline "${p.name}" ${p.status}? Look at its recent runs and explain.`,
                  })),
                ...(alerts.data ?? [])
                  .filter((a) => a.status === "open")
                  .map((a) => ({
                    key: `a:${a.id}`,
                    label: `Alert: ${a.title}`,
                    href: "/alerts",
                    prompt: `Explain the open alert "${a.title}" (${a.affected}) and what I should check.`,
                  })),
                ...(connectors.data ?? [])
                  .filter((c) => c.health === "unhealthy" || c.health === "degraded")
                  .map((c) => ({
                    key: `c:${c.id}`,
                    label: `Source ${c.name} is ${c.health}`,
                    href: "/connectors",
                    prompt: `Source "${c.name}" is ${c.health}. Test it and tell me what is wrong.`,
                  })),
              ]}
              onAsk={(prompt) => ask(prompt)}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <MessageSquare className="size-4" /> Recent conversations
            </CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-1">
            {copilot.sessions.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No conversations yet. Ask something above to start one.
              </p>
            ) : (
              copilot.sessions.slice(0, 5).map((s) => (
                <Link
                  key={s.id}
                  href={`/copilot?id=${encodeURIComponent(s.id)}`}
                  className="truncate rounded-md px-2 py-1.5 text-sm hover:bg-muted"
                >
                  {s.title || "Untitled conversation"}
                </Link>
              ))
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

function greeting(): string {
  const h = new Date().getHours()
  if (h < 12) return "Good morning"
  if (h < 18) return "Good afternoon"
  return "Good evening"
}

/**
 * Prompt chips. A failed pipeline, when there is one, leads — otherwise
 * generic starters that every workspace can answer from its own catalog.
 */
function suggestions(pipelines: { name: string; status: string }[]): string[] {
  const failed = pipelines.find((p) => p.status === "failed")
  const base = [
    "What data do we have?",
    "Which tables changed in the last day?",
    "Show the top rows of the busiest mart",
  ]
  return failed ? [`Why did ${failed.name} fail?`, ...base.slice(0, 2)] : base
}

function AiAction({
  icon: Icon,
  title,
  description,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-col gap-1 rounded-xl border border-border bg-card p-4 text-left transition-colors hover:border-primary/50 hover:bg-muted/50"
    >
      <span className="flex items-center gap-2 text-sm font-medium">
        <Icon className="size-4 text-primary" />
        {title}
      </span>
      <span className="text-xs text-muted-foreground">{description}</span>
      <span className="mt-1 inline-flex items-center gap-1 text-xs text-primary">
        <Sparkles className="size-3" /> Let AI do it
      </span>
    </button>
  )
}

type AttentionItem = { key: string; label: string; href: string; prompt: string }

function AttentionList({
  loading,
  failedReads,
  items,
  onAsk,
}: {
  loading: boolean
  failedReads: string[]
  items: AttentionItem[]
  onAsk: (prompt: string) => void
}) {
  if (loading) {
    return (
      <>
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-3/4" />
      </>
    )
  }
  return (
    <>
      {items.slice(0, 6).map((it) => (
        <div key={it.key} className="flex items-center justify-between gap-2">
          <Link href={it.href} className="min-w-0 truncate text-sm hover:underline">
            {it.label}
          </Link>
          <Button variant="ghost" size="xs" onClick={() => onAsk(it.prompt)}>
            <Sparkles className="size-3 text-primary" /> Ask AI
          </Button>
        </div>
      ))}
      {items.length === 0 && failedReads.length === 0 ? (
        <p className="text-sm text-muted-foreground">Nothing needs you right now.</p>
      ) : null}
      {failedReads.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          Couldn&apos;t load {failedReads.join(", ")} — this list may be incomplete.
        </p>
      ) : null}
    </>
  )
}
