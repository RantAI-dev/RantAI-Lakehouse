/**
 * Prompts offered on Home: the suggestion chips under the composer and the
 * examples its placeholder cycles through.
 *
 * They lead with what is true of this workspace right now (a failed
 * pipeline, the dashboard last opened), then fall back to starters every
 * deployment can answer from its own catalog. A name appears only when the
 * caller read it from a service; nothing here invents one, and no tenant's
 * subject matter is baked in (same rule as `features/copilot/page-context`).
 */
export type HomePromptContext = {
  mode: "ask" | "build"
  /** A pipeline whose status is `failed`, when there is one. */
  failedPipeline?: string
  /** The dashboard last opened in this browser, when it still exists. */
  lastDashboard?: string
  /** A source whose health is unhealthy or degraded, when there is one. */
  unhealthySource?: string
}

const ASK = [
  "What data do we have?",
  "Which tables changed in the last day?",
  "Row counts for every gold mart",
  "Summarize yesterday's pipeline runs",
]

const BUILD = [
  "Suggest a dashboard for the data we have",
  "Add an alert when a mart measure crosses a threshold",
  "Refresh the lakehouse from Bronze to Gold",
  "Guide me through connecting a new data source",
]

export function homePrompts({ mode, failedPipeline, lastDashboard, unhealthySource }: HomePromptContext): string[] {
  if (mode === "build") {
    return [
      ...(lastDashboard ? [`Add a chart to "${lastDashboard}"`] : []),
      ...(failedPipeline ? [`Retry the failed pipeline "${failedPipeline}"`] : []),
      ...BUILD,
    ]
  }
  return [
    ...(failedPipeline ? [`Why did ${failedPipeline} fail?`] : []),
    ...(unhealthySource ? [`What is wrong with the source "${unhealthySource}"?`] : []),
    ...(lastDashboard ? [`Explain the charts on "${lastDashboard}"`] : []),
    ...ASK,
  ]
}
