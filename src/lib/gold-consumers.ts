/**
 * Shared view of the outside-reader count for a published Gold mart
 * (DATA-5 D10). The Gold Exports table and the open-format card show
 * the same three honest states, so the states and the per-tool line
 * live here, once, instead of being written twice in two components.
 *
 * The three states are the only ones the console may show:
 * - `not_measured`: outside reading is not set up. The route says why
 *   (DATA-5 D4/D7) and the console shows that reason. It never shows a
 *   zero here — "not measured" and "nobody read it" are different
 *   claims.
 * - `no_reads`: the door is set up and nobody read the table in the
 *   window. A measured zero.
 * - `readers`: the tools that read it, most-read first.
 *
 * `unknown` covers the one shape the API does not emit — `supported:
 * true` with a null list. It renders as "not known", never as zero,
 * because a broken contract must not read as "no readers".
 */
import { formatRelativeTime } from "@/lib/format"
import type { GoldConsumer, GoldConsumers } from "@/services/contracts/gold"

export type ConsumerState =
  | { kind: "not_measured"; reason: string }
  | { kind: "unknown" }
  | { kind: "no_reads" }
  | { kind: "readers"; readers: GoldConsumer[] }

/** How many tools the open-format card names before it counts the rest. */
export const READERS_SHOWN = 5

/**
 * The reason shown when the route refuses but sends no reason of its
 * own. It names the setting an operator has to publish, so the console
 * never says "not measured" without saying why.
 */
export const NOT_MEASURED_REASON = "Outside reading is not set up."

/**
 * Classify the route's answer into the three states the console shows
 * (plus the one shape the API does not emit).
 */
export function consumerState(data: GoldConsumers): ConsumerState {
  if (!data.supported) {
    return { kind: "not_measured", reason: data.reason ?? NOT_MEASURED_REASON }
  }
  if (data.consumers === null || data.consumers === undefined) {
    return { kind: "unknown" }
  }
  if (data.consumers.length === 0) {
    return { kind: "no_reads" }
  }
  return { kind: "readers", readers: data.consumers }
}

/**
 * One tool's line, in the shape the Gold Exports tooltip and the
 * open-format card both use: `name: N reads, last <relative time>`.
 * The relative time is what an admin acts on — a tool that read the
 * table three months ago is not the same as one that read it an hour
 * ago.
 */
export function consumerLine(consumer: GoldConsumer, now = Date.now()): string {
  return `${consumer.name}: ${consumer.reads} reads, last ${formatRelativeTime(
    consumer.lastReadAt,
    now,
  )}`
}

/** The tooltip of the Gold Exports cell: one line per tool. */
export function consumerTooltip(readers: GoldConsumer[], now = Date.now()): string {
  return readers.map((reader) => consumerLine(reader, now)).join("\n")
}
