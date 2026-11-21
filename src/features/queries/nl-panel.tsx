"use client"

import { FileCode2, Play } from "lucide-react"

import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { MiniMarkdown } from "@/features/copilot/mini-markdown"
import type { useQueryStudio } from "./use-query-studio"
import { AgentResultCard } from "./agent-result-card"
import { Kbd } from "./studio-toolbar"

/**
 * Ask in words: the Ask tab of the studio (`UX-9`: Ask stays a tab beside
 * SQL). Two ways out: let the agent run the whole loop, or just take the SQL
 * and drive it yourself.
 */
export function NaturalLanguagePanel({
  studio,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
}) {
  const { question, setQuestion, generate, generateAct, agent } = studio
  const asking = agent.busy
  const generating = generateAct.status === "pending"
  const canAsk = !asking && question.trim().length > 0

  return (
    <div className="space-y-3">
      {/* UX-4: the question is this panel's one required input, so no star
          and no legend; the textarea keeps `aria-required`. UX-9: the Ask
          tab above it is its visible name. */}
      <label htmlFor="nl-question" className="sr-only">
        Question
      </label>
      <Textarea
        id="nl-question"
        aria-required
        value={question}
        onChange={(e) => setQuestion(e.target.value)}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault()
            if (canAsk) void agent.ask()
          }
        }}
        rows={4}
        placeholder="e.g. What was revenue by region last quarter?"
        className="min-h-24 resize-none rounded-xl border-0 bg-muted/50 px-3.5 py-2.5 dark:bg-input/30"
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={() => void agent.ask()} disabled={!canAsk}>
          <Play className="size-3.5 fill-current" aria-hidden />
          {asking ? "Agent working…" : "Ask"}
          {asking ? null : (
            <Kbd className="ml-0.5 hidden bg-white/15 text-white/85 sm:inline-flex" aria-hidden>
              ⌘↵
            </Kbd>
          )}
        </Button>
        <Button
          variant="ghost"
          onClick={() => void generate()}
          disabled={generating || !question.trim()}
        >
          <FileCode2 aria-hidden />
          {generating ? "Writing…" : "Write SQL only"}
        </Button>
        <p className="text-xs text-muted-foreground">
          Ask runs the query and explains it · Write SQL only fills the editor
        </p>
      </div>

      {agent.error ? (
        <SectionCard title="The agent could not answer">
          <p className="text-sm text-muted-foreground">{agent.error}</p>
        </SectionCard>
      ) : generateAct.status === "error" ? (
        <SectionCard title="Could not generate SQL">
          <p className="text-sm text-muted-foreground">
            {generateAct.error.message}
          </p>
        </SectionCard>
      ) : null}

      {agent.result ? <AgentResultCard result={agent.result} /> : null}

      {generateAct.data ? (
        <SectionCard title="Explanation">
          <div className="text-sm">
            <MiniMarkdown text={generateAct.data.explanation} />
          </div>
        </SectionCard>
      ) : null}
    </div>
  )
}
