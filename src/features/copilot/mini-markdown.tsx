import * as React from "react";
import { OMITTED_TABLE_TEXT, UNVERIFIED_NUMBER_LABEL, tokenizeInline } from "@/lib/citation-markers";

/**
 * A minimal, dependency-free Markdown renderer for AI Copilot answers.
 * Supports: headings, GFM tables, lists (dash/star/number), bold, italic,
 * inline code. Enough for the model's concise output; not full CommonMark
 * (e.g. no blockquotes / nested lists).
 *
 * Also renders the two citation states the copilot backend's
 * `annotate_answer` (WS7 item F6; `rust/crates/lakehouse-api/src/routes/ai/
 * citations.rs`) can write into the answer text before it ever reaches
 * this component: an unverified number wrapped in
 * `<span data-unverified="true">…</span>`, and a whole table with zero
 * verified cells replaced by the literal `[table omitted: not backed by a
 * tool result]`. Both are matched as plain text by `tokenizeInline`
 * (`src/lib/citation-markers.ts`) — this component never runs the answer
 * through an HTML parser, so there is no sanitizer step that could strip
 * either marker.
 */

function renderInline(text: string, keyBase: string): React.ReactNode[] {
  return tokenizeInline(text).map((tok, i) => {
    const key = `${keyBase}-${i}`;
    switch (tok.kind) {
      case "text":
        return tok.content;
      case "bold":
        return <strong key={key}>{tok.content}</strong>;
      case "code":
        return (
          <code key={key} className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
            {tok.content}
          </code>
        );
      case "italic":
        return <em key={key}>{tok.content}</em>;
      case "unverified":
        // Legible, not decorative: the dashed underline alone would teach
        // a reader to ignore it, so the title spells out WHY this number
        // is marked (WS7 item F1's own wording — a fact about the check,
        // never a claim the number is wrong).
        return (
          <span
            key={key}
            data-unverified="true"
            className="underline decoration-dashed decoration-2 decoration-amber-500 underline-offset-2"
            title={UNVERIFIED_NUMBER_LABEL}
          >
            {tok.content}
          </span>
        );
      case "omitted-table":
        // Reads as a deliberate product behaviour, not a crash: same
        // inline styling as the rest of the answer's prose, no error
        // chrome, and the exact literal the backend emits.
        return (
          <span key={key} className="italic text-muted-foreground">
            {OMITTED_TABLE_TEXT}
          </span>
        );
      default:
        return null;
    }
  });
}

function splitRow(line: string): string[] {
  return line
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((c) => c.trim());
}

export function MiniMarkdown({ text }: { text: string }) {
  const lines = text.replace(/\r/g, "").split("\n");
  const blocks: React.ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Empty → skip.
    if (!line.trim()) {
      i++;
      continue;
    }

    // Heading.
    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    if (h) {
      const level = h[1].length;
      const cls = level === 1 ? "text-base font-semibold" : level === 2 ? "text-sm font-semibold" : "text-sm font-medium";
      blocks.push(
        <p key={key++} className={`${cls} mt-1 text-foreground`}>
          {renderInline(h[2], `h${key}`)}
        </p>,
      );
      i++;
      continue;
    }

    // GFM table: a '|...|' row followed by a '|---|' separator row.
    if (line.trim().startsWith("|") && i + 1 < lines.length && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[i + 1])) {
      const header = splitRow(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        rows.push(splitRow(lines[i]));
        i++;
      }
      blocks.push(
        <div key={key++} className="my-1 overflow-x-auto">
          <table className="w-full border-collapse text-xs">
            <thead>
              <tr className="border-b border-border">
                {header.map((c, j) => (
                  <th key={j} className="px-2 py-1 text-left font-medium text-muted-foreground">
                    {renderInline(c, `th${key}-${j}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, ri) => (
                <tr key={ri} className="border-b border-border/50 last:border-0">
                  {r.map((c, ci) => (
                    <td key={ci} className="px-2 py-1 tabular-nums">
                      {renderInline(c, `td${key}-${ri}-${ci}`)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    // List (-, *, or '1.').
    if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
      const items: string[] = [];
      const ordered = /^\s*\d+\.\s+/.test(line);
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*([-*]|\d+\.)\s+/, ""));
        i++;
      }
      const ListTag = ordered ? "ol" : "ul";
      blocks.push(
        <ListTag key={key++} className={`my-1 ${ordered ? "list-decimal" : "list-disc"} space-y-0.5 pl-5 text-sm`}>
          {items.map((it, j) => (
            <li key={j}>{renderInline(it, `li${key}-${j}`)}</li>
          ))}
        </ListTag>,
      );
      continue;
    }

    // Paragraph: join lines until a blank line.
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,3})\s/.test(lines[i]) && !lines[i].trim().startsWith("|") && !/^\s*([-*]|\d+\.)\s+/.test(lines[i])) {
      para.push(lines[i]);
      i++;
    }
    blocks.push(
      <p key={key++} className="text-sm leading-relaxed">
        {renderInline(para.join(" "), `p${key}`)}
      </p>,
    );
  }

  return <div className="space-y-1.5">{blocks}</div>;
}
