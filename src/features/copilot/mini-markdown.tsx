import * as React from "react";
import { OMITTED_TABLE_TEXT, UNVERIFIED_NUMBER_LABEL, tokenizeInline } from "@/lib/citation-markers";
import { CopyButton } from "./copy-button";

/**
 * A minimal, dependency-free Markdown renderer for AI Copilot answers.
 * Supports: headings, GFM tables, fenced code blocks (with copy), block
 * quotes, rules, nested lists (dash/star/numbered), bold, italic, inline
 * code and http(s) links. Enough for the model's concise output; not full
 * CommonMark.
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
      case "link":
        return (
          <a
            key={key}
            href={tok.href}
            target="_blank"
            rel="noopener noreferrer"
            className="font-medium text-primary underline underline-offset-2"
          >
            {tok.content}
          </a>
        );
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

function CodeBlock({ code, lang }: { code: string; lang: string }) {
  return (
    <div className="my-1 overflow-hidden rounded-lg border border-border bg-muted/40">
      <div className="flex items-center justify-between border-b border-border px-2.5 py-1">
        <span className="font-mono text-[11px] text-muted-foreground">{lang || "code"}</span>
        <CopyButton text={code} label="Copy code" />
      </div>
      <pre className="overflow-x-auto p-2.5 text-xs leading-relaxed">
        <code className="font-mono">{code}</code>
      </pre>
    </div>
  );
}

const LIST_ITEM = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;

export type ListNode = { text: string; children: ListNode[]; ordered: boolean };

/** List items from `lines[start]` on, nested by indentation. */
export function parseList(lines: string[], start: number): { nodes: ListNode[]; next: number } {
  const root: ListNode[] = [];
  const first = LIST_ITEM.exec(lines[start]);
  // Each level is the indentation its items share.
  const stack: { indent: number; items: ListNode[] }[] = [{ indent: first?.[1].length ?? 0, items: root }];
  let i = start;
  while (i < lines.length) {
    const m = LIST_ITEM.exec(lines[i]);
    if (!m) break;
    const indent = m[1].length;
    const node: ListNode = { text: m[3], children: [], ordered: /\d/.test(m[2]) };
    while (stack.length > 1 && indent < stack[stack.length - 1].indent) stack.pop();
    const level = stack[stack.length - 1];
    const prev = level.items[level.items.length - 1];
    if (indent > level.indent && prev) {
      prev.children.push(node);
      stack.push({ indent, items: prev.children });
    } else {
      level.items.push(node);
    }
    i++;
  }
  return { nodes: root, next: i };
}

function renderList(nodes: ListNode[], keyBase: string): React.ReactNode {
  const ordered = nodes[0]?.ordered ?? false;
  const Tag = ordered ? "ol" : "ul";
  return (
    <Tag className={`${ordered ? "list-decimal" : "list-disc"} space-y-0.5 pl-5 text-sm`}>
      {nodes.map((n, j) => (
        <li key={j}>
          {renderInline(n.text, `${keyBase}-${j}`)}
          {n.children.length ? renderList(n.children, `${keyBase}-${j}c`) : null}
        </li>
      ))}
    </Tag>
  );
}

const BLOCK_START = /^(#{1,3}\s|```|>|\s*([-*+]|\d+\.)\s+|\s*\|)/;

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

    // Fenced code block.
    const fence = /^```\s*([\w+-]*)/.exec(line);
    if (fence) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i++; // closing fence (or end of text)
      blocks.push(<CodeBlock key={key++} code={code.join("\n")} lang={fence[1]} />);
      continue;
    }

    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    if (h) {
      const level = h[1].length;
      const cls = level === 1 ? "text-base font-semibold" : level === 2 ? "text-sm font-semibold" : "text-sm font-medium";
      blocks.push(
        <p key={key++} className={`${cls} mt-1 text-foreground`}>
          {renderInline(h[2], `h${key}`)}
        </p>
      );
      i++;
      continue;
    }

    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
      blocks.push(<hr key={key++} className="my-2 border-border" />);
      i++;
      continue;
    }

    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      blocks.push(
        <blockquote key={key++} className="border-l-2 border-border pl-3 text-sm text-muted-foreground">
          {renderInline(quote.join(" "), `q${key}`)}
        </blockquote>
      );
      continue;
    }

    // GFM table: a '|...|' row followed by a '|---|' separator.
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
        </div>
      );
      continue;
    }

    if (LIST_ITEM.test(line)) {
      const { nodes, next } = parseList(lines, i);
      blocks.push(<div key={key++} className="my-1">{renderList(nodes, `l${key}`)}</div>);
      i = next;
      continue;
    }

    // Paragraph: consecutive lines up to a blank line or another block.
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && (para.length === 0 || !BLOCK_START.test(lines[i]))) {
      para.push(lines[i]);
      i++;
    }
    blocks.push(
      <p key={key++} className="text-sm leading-relaxed">
        {renderInline(para.join(" "), `p${key}`)}
      </p>
    );
  }

  return <div className="space-y-1.5">{blocks}</div>;
}
