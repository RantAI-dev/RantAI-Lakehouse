"use client"

import { Sparkles } from "lucide-react"

import { SectionCard } from "@/components/patterns/section-card"
import { RequiredLegend } from "@/components/patterns/required-legend"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { MiniMarkdown } from "@/features/copilot/mini-markdown"
import type { useQueryStudio } from "./use-query-studio"
import { AgentResultCard } from "./agent-result-card"

/**
 * Ask in words. Two ways out: let the agent run the whole loop, or just
 * take the SQL and drive it yourself.
 */
export function NaturalLanguagePanel({
  studio,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
}) {
  const { question, setQuestion, generate, generateAct, agent } = studio
  const asking = agent.busy
  const generating = generateAct.status === "pending"

  return (
    <div className="space-y-3">
      <RequiredLegend />
      <Label htmlFor="nl-question" required>Question</Label>
      <Textarea
        id="nl-question"
        aria-required
        value={question}
        onChange={(e) => setQuestion(e.target.value)}
        rows={4}
        placeholder="e.g. What was revenue by region last quarter?"
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={() => void agent.ask()}
          disabled={asking || !question.trim()}
        >
          <Sparkles className="size-4" aria-hidden />
          {asking ? "Agent working…" : "Ask (agentic)"}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void generate()}
          disabled={generating || !question.trim()}
        >
          {generating ? "Generating…" : "Generate SQL only"}
        </Button>
        <p className="text-xs text-muted-foreground">
          Ask runs the query and explains the answer. Generate only writes
          the SQL.
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
