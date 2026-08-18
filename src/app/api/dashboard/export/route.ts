import { listStoredCharts, listBoards } from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

/** Emitter YAML minimal (dependency-free) untuk nilai skalar/array/objek datar. */
function yamlValue(v: unknown): string {
  if (v === null || v === undefined) return "~";
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  const s = String(v);
  return /^[\w.\-/]+$/.test(s) ? s : JSON.stringify(s);
}

function yamlChart(c: { id: string; title: string; board: string; def: Record<string, unknown> }): string {
  const d = c.def ?? {};
  const lines = [`- id: ${c.id}`, `  board: ${yamlValue(c.board)}`];
  for (const [k, val] of Object.entries(d)) {
    if (val === undefined) continue;
    if (Array.isArray(val)) {
      lines.push(`  ${k}: [${val.map(yamlValue).join(", ")}]`);
    } else {
      lines.push(`  ${k}: ${yamlValue(val)}`);
    }
  }
  return lines.join("\n");
}

/**
 * Ekspor semua spec chart tersimpan sebagai YAML — "dashboard as code" (ala
 * Rill). Bisa disimpan ke git untuk versioning/kode-review. Definisi terstruktur
 * (mart+kolom+tipe), bukan SQL, jadi mudah dibaca & di-diff.
 */
export async function GET() {
  const [charts, boards] = await Promise.all([listStoredCharts(), listBoards()]);
  const body =
    `# RantAI Lakehouse — dashboard as code\n` +
    `# boards & chart specs, diekspor dari console.bi_chart\n\n` +
    `boards:\n${[{ id: "default", name: "Utama" }, ...boards].map((b) => `  - id: ${b.id}\n    name: ${yamlValue(b.name)}`).join("\n")}\n\n` +
    `charts:\n${charts.map((c) => yamlChart({ id: c.id, title: c.title, board: c.board, def: c.def as Record<string, unknown> })).join("\n")}\n`;
  return new Response(body, {
    headers: {
      "Content-Type": "text/yaml; charset=utf-8",
      "Content-Disposition": 'attachment; filename="dashboards.yaml"',
    },
  });
}
