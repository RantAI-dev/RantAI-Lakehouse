/**
 * Makes a half-written markdown answer safe to render while it streams.
 *
 * Copilot's answer arrives in pieces (`delta` events from `/api/ai/chat`),
 * so the text on screen is often cut mid-token. Rendered as-is, an open
 * `**` turns the rest of the answer bold, an open backtick turns it into
 * code, a table without its `|---|` row shows as pipes, and `text\n-`
 * reads as a heading. This closes or holds back exactly those cases; the
 * finished answer (the `done` body) is rendered without it.
 *
 * Adapted from the ParaGPT answer-rendering proposal's `settle()` and
 * Streamdown's incomplete-markdown handling.
 */
export function settleStreamingMarkdown(text: string): string {
  let t = text

  // Hold back a trailing table until its delimiter row has arrived.
  const lines = t.replace(/\s+$/, "").split("\n")
  let i = lines.length - 1
  while (i >= 0 && lines[i].trim().startsWith("|")) i--
  const tail = lines.slice(i + 1)
  if (tail.length && !tail.some((l) => /^\|?\s*:?-{3,}/.test(l.trim()))) {
    t = lines.slice(0, i + 1).join("\n")
  }

  // A lone `-`/`=` after a paragraph would render as a setext heading.
  t = t.replace(/\n\s*[-=*+]\s*$/, "\n")

  // A half-typed link shows its text only.
  t = t.replace(/\[([^\]]*)\]\([^)]*$/, "$1").replace(/\[([^\]\n]*)$/, "$1")

  // Close emphasis and inline code the model has opened but not closed.
  // Fenced blocks are left alone: an open fence already renders as code.
  const outsideFences = t.replace(/```[\s\S]*?(```|$)/g, "")
  if ((outsideFences.match(/`/g) ?? []).length % 2) t += "`"
  if ((outsideFences.match(/\*\*/g) ?? []).length % 2) t += "**"

  return t
}
