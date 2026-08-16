import { chQuery, chRows } from "./clickhouse";
import { listJobs, listRuns, launchRun, mapRunStatus } from "./dagster";
import { isReadOnlySql } from "./agent-tools";

/**
 * Registry TOOL untuk AI Copilot lakehouse. Tiap tool membungkus kapabilitas
 * NYATA (ClickHouse / Dagster). LLM (MiniMax) memutuskan tool mana dipanggil
 * lewat function-calling; server mengeksekusinya dan mengembalikan hasilnya.
 *
 * Ini yang bikin copilot bisa "tanya soal semua data" DAN "bikin Bronze/Silver/
 * Gold lewat chat" (trigger pipeline Dagster).
 */

export type ToolDef = {
  schema: {
    type: "function";
    function: { name: string; description: string; parameters: Record<string, unknown> };
  };
  run: (args: Record<string, unknown>) => Promise<unknown>;
};

const CATALOG_UNION =
  "(SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta.dataset_catalog` " +
  "UNION ALL SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta_sec.dataset_catalog`)";

export const TOOLS: Record<string, ToolDef> = {
  // ── Tanya data ──────────────────────────────────────────────────────────
  run_sql: {
    schema: {
      type: "function",
      function: {
        name: "run_sql",
        description:
          "Jalankan query SELECT ClickHouse (read-only) untuk menjawab pertanyaan data. " +
          "Gunakan tabel serving.mart_* (Gold) untuk agregasi atau silver.`<nama>` untuk detail. " +
          "Selalu SELECT saja, LIMIT wajar.",
        parameters: {
          type: "object",
          properties: { sql: { type: "string", description: "Query SELECT ClickHouse" } },
          required: ["sql"],
        },
      },
    },
    async run(args) {
      const sql = String(args.sql ?? "");
      if (!isReadOnlySql(sql)) return { error: "Hanya SELECT diizinkan." };
      try {
        const r = await chQuery(sql);
        return { columns: r.meta.map((m) => m.name), rows: r.data.slice(0, 50), rowCount: r.rows };
      } catch (e) {
        return { error: e instanceof Error ? e.message : String(e) };
      }
    },
  },

  list_datasets: {
    schema: {
      type: "function",
      function: {
        name: "list_datasets",
        description: "Daftar dataset di katalog lakehouse (opsional filter kata kunci / tier primer|sekunder).",
        parameters: {
          type: "object",
          properties: {
            search: { type: "string" },
            tier: { type: "string", enum: ["primer", "sekunder"] },
          },
        },
      },
    },
    async run(args) {
      const rows = await chRows<{ slug: string; title: string; tier: string }>(
        `SELECT slug, title, tier FROM ${CATALOG_UNION} LIMIT 500`,
      );
      const term = String(args.search ?? "").toLowerCase();
      const tier = args.tier ? String(args.tier) : "";
      const hit = rows
        .filter((r) => (!tier || r.tier === tier) && (!term || `${r.title} ${r.slug}`.toLowerCase().includes(term)))
        .slice(0, 40);
      return { total: hit.length, datasets: hit };
    },
  },

  describe_dataset: {
    schema: {
      type: "function",
      function: {
        name: "describe_dataset",
        description: "Metadata + skema kolom + jumlah baris satu dataset (by slug).",
        parameters: {
          type: "object",
          properties: { slug: { type: "string" } },
          required: ["slug"],
        },
      },
    },
    async run(args) {
      const slug = String(args.slug ?? "").replace(/'/g, "");
      const meta = (
        await chRows<{ title: string; table_name: string; tier: string }>(
          `SELECT title, table_name, tier FROM ${CATALOG_UNION} WHERE slug='${slug}' LIMIT 1`,
        )
      )[0];
      if (!meta) return { error: "dataset tidak ditemukan" };
      const cols = await chRows<{ key_asli: string; tipe: string; deskripsi: string }>(
        `SELECT key_asli, tipe, deskripsi FROM lake.\`bronze_meta.dataset_column\` WHERE slug='${slug}'
         UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.\`bronze_meta_sec.dataset_column\` WHERE slug='${slug}'`,
      );
      let rows = 0;
      try {
        rows = Number((await chRows<{ n: string }>(`SELECT toString(count()) n FROM silver.\`${meta.table_name}\``))[0]?.n) || 0;
      } catch {
        /* silver belum ada */
      }
      return { title: meta.title, tier: meta.tier, table: meta.table_name, rows, columns: cols };
    },
  },

  get_lineage: {
    schema: {
      type: "function",
      function: {
        name: "get_lineage",
        description: "Silsilah sebuah dataset: source → Bronze → Silver + mapping kolom (by slug).",
        parameters: { type: "object", properties: { slug: { type: "string" } }, required: ["slug"] },
      },
    },
    async run(args) {
      const slug = String(args.slug ?? "").replace(/'/g, "");
      const meta = (
        await chRows<{ table_name: string; tier: string }>(
          `SELECT table_name, tier FROM ${CATALOG_UNION} WHERE slug='${slug}' LIMIT 1`,
        )
      )[0];
      if (!meta) return { error: "dataset tidak ditemukan" };
      const cols = await chRows<{ kolom: string; tipe: string }>(
        `SELECT kolom, tipe FROM _silver_meta.kolom_tipe WHERE tabel='${meta.table_name.replace(/'/g, "")}' LIMIT 100`,
      );
      return {
        chain: `${meta.tier === "sekunder" ? "Sumber sekunder" : "Satu Data Jakarta"} → bronze.${meta.table_name} → silver.${meta.table_name}`,
        columnMappings: cols.map((c) => `${c.kolom} → ${c.tipe}`),
      };
    },
  },

  get_quality: {
    schema: {
      type: "function",
      function: {
        name: "get_quality",
        description: "Ringkasan kualitas data lakehouse (jumlah cek pass/warn/fail + contoh masalah).",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      try {
        const summary = await chRows<{ verdict: string; n: string }>(
          `SELECT verdict, toString(count()) n FROM (
             SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict FROM _silver_meta.quality GROUP BY tabel, cek
           ) GROUP BY verdict`,
        );
        return { summary };
      } catch (e) {
        return { error: `quality belum tersedia: ${e}` };
      }
    },
  },

  // ── Operasikan lakehouse (bikin Bronze/Silver/Gold via chat) ────────────
  trigger_lakehouse_build: {
    schema: {
      type: "function",
      function: {
        name: "trigger_lakehouse_build",
        description:
          "BANGUN ULANG lakehouse: tarik data SDI+berkas ke Bronze, generate Silver bertipe, " +
          "build mart Gold. Menjalankan job Dagster 'refresh_lakehouse'. Pakai saat user minta " +
          "membangun/menyegarkan data Bronze/Silver/Gold.",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      const r = await launchRun("refresh_lakehouse");
      if (r.error) return { error: r.error };
      return { launched: true, runId: r.runId, note: "Pipeline Bronze→Silver→Gold dijalankan. Cek status dengan get_build_status." };
    },
  },

  get_build_status: {
    schema: {
      type: "function",
      function: {
        name: "get_build_status",
        description: "Status run pipeline lakehouse terakhir (Dagster).",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      const [jobs, runs] = await Promise.all([listJobs(), listRuns(undefined, 10)]);
      return {
        jobs: jobs.map((j) => j.name),
        recentRuns: runs.map((r) => ({
          job: r.jobName,
          status: mapRunStatus(r.status),
          startedAt: r.startTime ? new Date(r.startTime * 1000).toISOString() : null,
        })),
      };
    },
  },
};

export const TOOL_SCHEMAS = Object.values(TOOLS).map((t) => t.schema);

export async function runTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  const tool = TOOLS[name];
  if (!tool) return { error: `tool tak dikenal: ${name}` };
  return tool.run(args);
}
