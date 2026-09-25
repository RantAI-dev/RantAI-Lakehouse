"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import {
  AnimatePresence,
  motion,
  useReducedMotion,
  type Variants,
} from "motion/react"
import {
  ArrowRight,
  ArrowUp,
  BarChart3,
  CheckCircle2,
  GitBranch,
  MessageSquare,
  Plug,
  Sparkles,
} from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot, type Mode } from "@/features/copilot/use-copilot"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { connectorService, overviewService, pipelineService } from "@/services"

/**
 * AI-first landing page. The console is agentic-first, so the first thing
 * on screen is "ask or instruct", not a wall of platform metrics — those
 * moved to Monitoring → Health (`/health`).
 *
 * AI is a copilot here, not the default path for changes: the "Start
 * something" cards open the manual flow (create pipeline, add source,
 * dashboards), and handing the task to Copilot is a separate, explicit
 * "Let AI do it" button on the card. Copilot can do these things, but a
 * person should see the form before a source or pipeline exists.
 *
 * Everything under the prompt is read from real services: "Needs
 * attention" lists failed pipelines, open alerts and unhealthy sources as
 * the backend reports them. A section whose service fails says so instead
 * of rendering as "all clear".
 *
 * Whether an LLM is configured is not known up front (no endpoint reports
 * it); when it is not, `POST /api/ai/chat` answers 503 and the Copilot
 * page shows that error.
 *
 * Visuals follow `/login`'s "light through water" brand treatment
 * (`--brand-1`, `--brand-canvas-*`), done with CSS gradients and `motion`
 * instead of WebGL so the landing page stays cheap. All motion is dropped
 * under `prefers-reduced-motion`.
 */
export function HomePage() {
  const router = useRouter()
  const { user } = useAuth()
  const copilot = useCopilot()
  const reduce = useReducedMotion() ?? false
  const [pending, setPending] = React.useState<{ text: string; mode: Mode } | null>(null)

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
  const list = reduce ? undefined : STAGGER
  const item = reduce ? undefined : RISE

  const attention: AttentionItem[] = [
    ...(pipelines.data?.pipelines ?? [])
      .filter((p) => p.status === "failed" || p.status === "degraded")
      .map((p) => ({
        key: `p:${p.id}`,
        kind: "Pipeline",
        label: p.name,
        state: p.status,
        href: `/pipelines/${encodeURIComponent(p.id)}`,
        prompt: `Why is pipeline "${p.name}" ${p.status}? Look at its recent runs and explain.`,
      })),
    ...(alerts.data ?? [])
      .filter((a) => a.status === "open")
      .map((a) => ({
        key: `a:${a.id}`,
        kind: "Alert",
        label: a.title,
        state: "open",
        href: "/alerts",
        prompt: `Explain the open alert "${a.title}" (${a.affected}) and what I should check.`,
      })),
    ...(connectors.data ?? [])
      .filter((c) => c.health === "unhealthy" || c.health === "degraded")
      .map((c) => ({
        key: `c:${c.id}`,
        kind: "Source",
        label: c.name,
        state: c.health,
        href: "/connectors",
        prompt: `Source "${c.name}" is ${c.health}. Test it and tell me what is wrong.`,
      })),
  ]

  return (
    <motion.div
      variants={list}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-5xl flex-col gap-8 py-2 sm:py-6"
    >
      {/* Hero: greeting + prompt over the brand "lit water" backdrop. */}
      <motion.section
        variants={item}
        className="relative isolate overflow-hidden rounded-3xl border border-border/60 bg-card/40 px-5 pt-10 pb-6 sm:px-10 sm:pt-14 sm:pb-8"
      >
        <HeroBackdrop reduce={reduce} />
        <div className="mx-auto flex max-w-2xl flex-col items-center gap-2 text-center">
          <span className="inline-flex items-center gap-1.5 rounded-full border border-[color-mix(in_oklch,var(--brand-1),transparent_60%)] bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] px-3 py-1 text-xs font-medium text-[var(--brand-1)]">
            <Sparkles className="size-3" /> Copilot
          </span>
          <h1 className="text-3xl font-semibold tracking-[-0.03em] text-balance text-foreground sm:text-4xl">
            {greeting()}
            {firstName ? `, ${firstName}` : ""}
          </h1>
          <p className="text-sm text-muted-foreground sm:text-base">
            Ask about your data, or get help with the next step.
          </p>
        </div>

        <div className="mx-auto mt-7 max-w-2xl">
          <PromptBox onAsk={(q) => ask(q)} reduce={reduce} />
          <div className="mt-4 flex flex-wrap justify-center gap-2">
            {suggestions(pipelines.data?.pipelines ?? []).map((s) => (
              <button
                key={s}
                type="button"
                onClick={() => ask(s)}
                className="group/chip inline-flex items-center gap-1.5 rounded-full border border-border/70 bg-background/60 px-3 py-1.5 text-xs text-muted-foreground backdrop-blur transition-all duration-200 hover:-translate-y-0.5 hover:border-[color-mix(in_oklch,var(--brand-1),transparent_40%)] hover:text-foreground hover:shadow-[0_6px_20px_-8px_var(--brand-1)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50 active:translate-y-0 motion-reduce:transition-none motion-reduce:hover:translate-y-0"
              >
                <Sparkles className="size-3 text-[var(--brand-1)] transition-transform duration-300 group-hover/chip:rotate-12 group-hover/chip:scale-110" />
                {s}
              </button>
            ))}
          </div>
        </div>
      </motion.section>

      {/* Start something: manual first, Copilot on request. */}
      <motion.section variants={item} className="flex flex-col gap-3">
        <SectionTitle>Start something</SectionTitle>
        <div className="grid gap-3 sm:grid-cols-3">
          <StartCard
            icon={Plug}
            title="Connect a source"
            description="Register a database, stream or bucket. Credentials stay as secret references."
            href="/connectors/create"
            onAi={() => ask("Guide me through connecting a new data source.", "build")}
          />
          <StartCard
            icon={GitBranch}
            title="Create a pipeline"
            description="Choose a source and a target layer, then review before it runs."
            href="/pipelines/create"
            onAi={() => ask("Help me create a new pipeline. Ask me what to ingest and where it should land.", "build")}
          />
          <StartCard
            icon={BarChart3}
            title="Build a dashboard"
            description="Pick a mart and chart it. Every chart is added only after you confirm."
            href="/dashboards"
            onAi={() => ask("Suggest a dashboard for the data we have.", "build")}
          />
        </div>
      </motion.section>

      <motion.div variants={item} className="grid min-w-0 grid-cols-1 gap-4 lg:grid-cols-5">
        <Panel className="lg:col-span-3" title="Needs attention" count={attention.length}>
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
            items={attention}
            onAsk={(prompt) => ask(prompt)}
          />
        </Panel>

        <Panel className="lg:col-span-2" title="Recent conversations">
          {copilot.sessions.length === 0 ? (
            <p className="px-2 py-6 text-center text-sm text-muted-foreground">
              No conversations yet. Ask something above to start one.
            </p>
          ) : (
            <ul className="flex flex-col">
              {copilot.sessions.slice(0, 6).map((s) => (
                <li key={s.id}>
                  <Link
                    href={`/copilot?id=${encodeURIComponent(s.id)}`}
                    className="group/row flex items-center gap-3 rounded-lg px-2 py-2 text-sm transition-colors hover:bg-muted/60"
                  >
                    <MessageSquare className="size-4 shrink-0 text-muted-foreground transition-colors group-hover/row:text-[var(--brand-1)]" />
                    <span className="min-w-0 flex-1 truncate">
                      {s.title || "Untitled conversation"}
                    </span>
                    {s.updatedAt ? (
                      <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                        {formatRelativeTime(s.updatedAt)}
                      </span>
                    ) : null}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </motion.div>
    </motion.div>
  )
}

const STAGGER: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.08, delayChildren: 0.05 } },
}

const RISE: Variants = {
  hidden: { opacity: 0, y: 14 },
  show: {
    opacity: 1,
    y: 0,
    transition: { type: "spring", stiffness: 170, damping: 26 },
  },
}

/** Two slow-drifting brand glows and a faint grid — the `/login` water, calmed down. */
function HeroBackdrop({ reduce }: { reduce: boolean }) {
  const drift = (x: number[], y: number[], duration: number) =>
    reduce
      ? {}
      : {
          animate: { x, y },
          transition: { duration, repeat: Infinity, repeatType: "mirror" as const, ease: "easeInOut" as const },
        }
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 -z-10">
      <motion.div
        {...drift([0, 60, -20], [0, 20, -10], 18)}
        className="absolute -top-24 left-[8%] size-72 rounded-full bg-[var(--brand-canvas-light)] opacity-25 blur-[90px] dark:opacity-20"
      />
      <motion.div
        {...drift([0, -50, 30], [0, -15, 25], 22)}
        className="absolute -right-10 top-10 size-80 rounded-full bg-[var(--brand-canvas-dark)] opacity-30 blur-[100px] dark:opacity-40"
      />
      <div
        className="absolute inset-0 opacity-[0.35] [mask-image:radial-gradient(ellipse_at_center,black_20%,transparent_70%)] dark:opacity-[0.18]"
        style={{
          backgroundImage:
            "linear-gradient(to right, var(--border) 1px, transparent 1px), linear-gradient(to bottom, var(--border) 1px, transparent 1px)",
          backgroundSize: "32px 32px",
        }}
      />
    </div>
  )
}

const EXAMPLES = [
  "Which tables changed in the last day?",
  "Summarize yesterday's pipeline runs",
  "Row counts for every gold mart",
  "Is any source unhealthy right now?",
]

/**
 * The main input. A rotating conic ring lights up while it has focus, and
 * the placeholder cycles through example asks until the user types.
 */
function PromptBox({ onAsk, reduce }: { onAsk: (q: string) => void; reduce: boolean }) {
  const [draft, setDraft] = React.useState("")
  const [focused, setFocused] = React.useState(false)
  const [hint, setHint] = React.useState(0)
  const ref = React.useRef<HTMLTextAreaElement>(null)

  React.useEffect(() => {
    ref.current?.focus()
  }, [])

  React.useEffect(() => {
    if (reduce || draft) return
    const t = window.setInterval(() => setHint((h) => (h + 1) % EXAMPLES.length), 3500)
    return () => window.clearInterval(t)
  }, [reduce, draft])

  const submit = () => {
    if (!draft.trim()) return
    onAsk(draft)
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        submit()
      }}
      className="relative rounded-2xl p-px"
    >
      {/* Focus ring: a conic gradient spun behind a 1px gap. */}
      <div
        aria-hidden
        className={cn(
          "absolute inset-0 overflow-hidden rounded-2xl bg-border transition-opacity duration-300",
          focused ? "opacity-100" : "opacity-60"
        )}
      >
        <motion.div
          className={cn(
            "absolute top-1/2 left-1/2 aspect-square w-[200%] -translate-x-1/2 -translate-y-1/2 transition-opacity duration-500",
            focused ? "opacity-100" : "opacity-0"
          )}
          style={{
            background:
              "conic-gradient(from 0deg, transparent 0deg, var(--brand-1) 60deg, var(--brand-canvas-dark) 120deg, transparent 180deg, transparent 360deg)",
          }}
          animate={reduce ? undefined : { rotate: 360 }}
          transition={{ duration: 6, repeat: Infinity, ease: "linear" }}
        />
      </div>

      <div className="relative rounded-[15px] bg-background/90 shadow-[0_20px_60px_-30px_var(--brand-canvas-dark)] backdrop-blur-xl">
        <textarea
          ref={ref}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault()
              submit()
            }
          }}
          aria-label="Ask AI"
          rows={3}
          className="peer block min-h-28 w-full resize-none bg-transparent px-5 pt-4 pb-14 text-base text-foreground outline-none"
        />
        {/* Animated placeholder: a native one can't cross-fade. */}
        {!draft ? (
          <div aria-hidden className="pointer-events-none absolute top-4 left-5 right-16 text-base text-muted-foreground">
            <AnimatePresence mode="wait" initial={false}>
              <motion.span
                key={hint}
                initial={reduce ? false : { opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={reduce ? undefined : { opacity: 0, y: -6 }}
                transition={{ duration: 0.25 }}
                className="block truncate"
              >
                {EXAMPLES[hint]}
              </motion.span>
            </AnimatePresence>
          </div>
        ) : null}
        <div className="absolute inset-x-3 bottom-3 flex items-center justify-between">
          <span className="hidden pl-2 text-xs text-muted-foreground sm:inline">
            <kbd className="rounded border border-border bg-muted px-1 font-mono text-[10px]">Enter</kbd> to ask ·{" "}
            <kbd className="rounded border border-border bg-muted px-1 font-mono text-[10px]">Shift + Enter</kbd> new line
          </span>
          <motion.button
            type="submit"
            aria-label="Send to AI"
            disabled={!draft.trim()}
            whileHover={reduce || !draft.trim() ? undefined : { scale: 1.08 }}
            whileTap={reduce || !draft.trim() ? undefined : { scale: 0.94 }}
            className="ml-auto inline-flex size-9 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-[0_8px_24px_-10px_var(--brand-1)] transition-[opacity,box-shadow] hover:shadow-[0_10px_30px_-8px_var(--brand-1)] disabled:opacity-35 disabled:shadow-none"
          >
            <ArrowUp className="size-4" />
          </motion.button>
        </div>
      </div>
    </form>
  )
}

/**
 * A start card: the card itself opens the manual flow; "Let AI do it" is
 * a separate button that hands the same task to Copilot in Build mode. A
 * spotlight follows the cursor, driven by CSS variables so hover costs no
 * React renders.
 */
function StartCard({
  icon: Icon,
  title,
  description,
  href,
  onAi,
}: {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description: string
  href: string
  onAi: () => void
}) {
  const ref = React.useRef<HTMLDivElement>(null)
  return (
    <div
      ref={ref}
      onPointerMove={(e) => {
        const el = ref.current
        if (!el) return
        const r = el.getBoundingClientRect()
        el.style.setProperty("--mx", `${e.clientX - r.left}px`)
        el.style.setProperty("--my", `${e.clientY - r.top}px`)
      }}
      className="group/card relative flex flex-col overflow-hidden rounded-2xl border border-border bg-card transition-all duration-300 hover:-translate-y-1 hover:border-[color-mix(in_oklch,var(--brand-1),transparent_55%)] hover:shadow-[0_18px_40px_-24px_var(--brand-canvas-dark)] motion-reduce:transition-none motion-reduce:hover:translate-y-0"
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 opacity-0 transition-opacity duration-300 group-hover/card:opacity-100"
        style={{
          background:
            "radial-gradient(260px circle at var(--mx, 50%) var(--my, 0%), color-mix(in oklch, var(--brand-1), transparent 86%), transparent 70%)",
        }}
      />
      <Link
        href={href}
        className="relative flex flex-1 flex-col gap-3 p-5 outline-none after:absolute after:inset-0 after:rounded-2xl focus-visible:after:ring-2 focus-visible:after:ring-ring/50"
      >
        <span className="inline-flex size-10 items-center justify-center rounded-xl border border-border bg-muted/60 text-foreground transition-all duration-300 group-hover/card:scale-105 group-hover/card:border-[color-mix(in_oklch,var(--brand-1),transparent_50%)] group-hover/card:text-[var(--brand-1)]">
          <Icon className="size-5" />
        </span>
        <span className="flex items-center gap-1 text-sm font-semibold">
          {title}
          <ArrowRight className="size-4 -translate-x-1 opacity-0 transition-all duration-300 group-hover/card:translate-x-0 group-hover/card:opacity-100" />
        </span>
        <span className="text-xs leading-relaxed text-muted-foreground">{description}</span>
      </Link>
      <div className="relative flex items-center justify-between border-t border-border/70 px-5 py-2.5">
        <span className="text-[11px] text-muted-foreground">Or hand it to Copilot</span>
        <button
          type="button"
          onClick={onAi}
          className="group/ai inline-flex items-center gap-1 rounded-full px-2 py-1 text-xs font-medium text-[var(--brand-1)] transition-colors hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <Sparkles className="size-3 transition-transform duration-300 group-hover/ai:rotate-12" />
          Let AI do it
        </button>
      </div>
    </div>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="px-1 text-xs font-semibold tracking-[0.08em] text-muted-foreground uppercase">
      {children}
    </h2>
  )
}

function Panel({
  title,
  count,
  className,
  children,
}: {
  title: string
  count?: number
  className?: string
  children: React.ReactNode
}) {
  return (
    <section className={cn("flex min-w-0 flex-col gap-2 rounded-2xl border border-border bg-card p-3", className)}>
      <div className="flex items-center justify-between px-2 pt-1">
        <h2 className="text-sm font-semibold">{title}</h2>
        {count ? (
          <span className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive tabular-nums">
            {count}
          </span>
        ) : null}
      </div>
      {children}
    </section>
  )
}

type AttentionItem = {
  key: string
  kind: string
  label: string
  state: string
  href: string
  prompt: string
}

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
      <div className="flex flex-col gap-2 p-2">
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-4/5" />
      </div>
    )
  }
  return (
    <div className="flex flex-col">
      {items.length === 0 && failedReads.length === 0 ? (
        <div className="flex flex-col items-center gap-2 px-2 py-6 text-center">
          <CheckCircle2 className="size-6 text-emerald-500" />
          <p className="text-sm text-muted-foreground">Nothing needs you right now.</p>
        </div>
      ) : null}
      <ul className="flex flex-col">
        {items.slice(0, 6).map((it) => (
          <li
            key={it.key}
            className="group/row flex items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-muted/60"
          >
            <span className="relative flex size-2.5 shrink-0">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-destructive/60 motion-reduce:animate-none" />
              <span className="relative inline-flex size-2.5 rounded-full bg-destructive" />
            </span>
            <Link href={it.href} className="min-w-0 flex-1 text-sm hover:underline">
              <span className="text-muted-foreground">{it.kind} · </span>
              <span className="font-medium">{it.label}</span>
              <span className="text-muted-foreground"> is {it.state}</span>
            </Link>
            <button
              type="button"
              onClick={() => onAsk(it.prompt)}
              className="inline-flex shrink-0 items-center gap-1 rounded-full border border-transparent px-2 py-1 text-xs font-medium text-[var(--brand-1)] transition-all hover:border-[color-mix(in_oklch,var(--brand-1),transparent_60%)] hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50 sm:opacity-60 sm:group-hover/row:opacity-100"
            >
              <Sparkles className="size-3" /> Ask AI
            </button>
          </li>
        ))}
      </ul>
      {failedReads.length > 0 ? (
        <p className="px-2 pt-2 text-xs text-muted-foreground">
          Couldn&apos;t load {failedReads.join(", ")}. This list may be incomplete.
        </p>
      ) : null}
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
