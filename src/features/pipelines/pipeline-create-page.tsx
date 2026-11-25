"use client"

import { PipelineEditor } from "./pipeline-editor"

/**
 * `/pipelines/create`: the pipeline editor with nothing stored yet
 * (BLD-1 part 1 D17). `?connectorId=` opens it from a connector's page
 * (`BLD-7`).
 */
export function PipelineCreatePage() {
  return <PipelineEditor />
}
