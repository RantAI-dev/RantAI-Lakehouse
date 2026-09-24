/**
 * Pure tokenizer for MiniMarkdown's inline citation markers (WS7 item F6).
 *
 * Recognizes the two literal shapes the backend's `annotate_answer`
 * (`rust/crates/lakehouse-api/src/routes/ai/citations.rs`) writes into the
 * answer text, alongside the bold/code/italic inline tokens
 * `mini-markdown.tsx` already rendered before this change:
 *
 * - `<span data-unverified="true">…</span>` wraps a printed number that
 *   matched no `ok: true` tool-trace result — see `annotate_line` and
 *   `annotate_table_block` in `citations.rs`. A verified number is left
 *   byte-for-byte unchanged, so there is no positive-state marker to
 *   tokenize here at all (WS7 item F1's own doc comment: any UI label for a
 *   match reads "matches a tool result", never bare "verified").
 * - the literal line `[table omitted: not backed by a tool result]`,
 *   substituted by `annotate_table_block` for a whole table with zero
 *   verified cells.
 *
 * Both strings arrive as plain text inside the answer string — MiniMarkdown
 * never runs the text through an HTML parser or `dangerouslySetInnerHTML`,
 * so there is no sanitizer step that could strip the `data-unverified`
 * marker: the regex below matches the literal text and rebuilds a real
 * React `<span>` from it, once, at render time.
 *
 * Kept dependency- and DOM-free so the matching rules are provable without
 * mounting a component (matches `src/lib/*.test.ts`'s no-DOM convention).
 */

export type InlineToken =
  | { kind: "text"; content: string }
  | { kind: "bold"; content: string }
  | { kind: "code"; content: string }
  | { kind: "italic"; content: string }
  | { kind: "link"; content: string; href: string }
  | { kind: "unverified"; content: string }
  | { kind: "omitted-table" };

/** The exact literal `annotate_table_block` (citations.rs) emits. */
export const OMITTED_TABLE_TEXT = "[table omitted: not backed by a tool result]";

/** The tooltip text a reader sees on an unverified-number marker (WS7 item
 * F1's own wording: a fact about the CHECK, not a claim the number is
 * wrong). */
export const UNVERIFIED_NUMBER_LABEL = "Could not be matched to any tool result";

/**
 * Splits `text` into inline tokens in order. Unmatched text runs come back
 * as `{ kind: "text" }`. The unverified-number and omitted-table cases are
 * matched as whole literal tokens — nothing about their inner text is
 * reinterpreted as Markdown.
 */
export function tokenizeInline(text: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  const re =
    /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\(https?:\/\/[^)\s]+\)|\*[^*\s][^*]*\*|<span data-unverified="true">[^<]+<\/span>|\[table omitted: not backed by a tool result\])/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) tokens.push({ kind: "text", content: text.slice(last, m.index) });
    const tok = m[0];
    if (tok.startsWith("**")) {
      tokens.push({ kind: "bold", content: tok.slice(2, -2) });
    } else if (tok.startsWith("`")) {
      tokens.push({ kind: "code", content: tok.slice(1, -1) });
    } else if (tok.startsWith("<span data-unverified")) {
      const inner = tok.replace(/^<span data-unverified="true">/, "").replace(/<\/span>$/, "");
      tokens.push({ kind: "unverified", content: inner });
    } else if (tok.startsWith("[table omitted")) {
      tokens.push({ kind: "omitted-table" });
    } else if (tok.startsWith("[")) {
      const link = /^\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)$/.exec(tok);
      tokens.push({ kind: "link", content: link?.[1] ?? tok, href: link?.[2] ?? "" });
    } else {
      tokens.push({ kind: "italic", content: tok.slice(1, -1) });
    }
    last = m.index + tok.length;
  }
  if (last < text.length) tokens.push({ kind: "text", content: text.slice(last) });
  return tokens;
}
