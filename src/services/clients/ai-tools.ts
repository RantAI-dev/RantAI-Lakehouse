import { chQuery, chRows } from "./clickhouse";
import { listJobs, listRuns, launchRun, mapRunStatus } from "./dagster";
import { isReadOnlySql } from "./agent-tools";
import {
  specFromInput,
  insertChart,
  listStoredCharts,
  deleteChart,
  listBoards,
  createBoard,
  type ChartInput,
} from "./bi-store";

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

  // ── Dashboarding lewat chat (agentic BI) ────────────────────────────────
  describe_mart: {
    schema: {
      type: "function",
      function: {
        name: "describe_mart",
        description:
          "Lihat mart Gold (serving.*) yang bisa divisualisasikan. Tanpa argumen: daftar semua mart. " +
          "Dengan `mart`: kolom mart itu, terbagi dimensi (kategori/waktu) & measure (angka). " +
          "PANGGIL INI DULU sebelum create_chart agar memilih kolom yang benar-benar ada.",
        parameters: {
          type: "object",
          properties: { mart: { type: "string", description: "nama mart, mis. mart_wisman" } },
        },
      },
    },
    async run(args) {
      const mart = String(args.mart ?? "").replace(/[^a-zA-Z0-9_]/g, "");
      if (!mart) {
        const rows = await chRows<{ name: string; total_rows: string }>(
          `SELECT name, toString(total_rows) AS total_rows FROM system.tables
            WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name`,
        );
        return { marts: rows.map((r) => ({ mart: r.name, rows: Number(r.total_rows) })) };
      }
      const cols = await chRows<{ name: string; type: string }>(
        `SELECT name, type FROM system.columns WHERE database='serving' AND table='${mart}' ORDER BY position`,
      );
      if (cols.length === 0) return { error: `mart '${mart}' tidak ditemukan di serving.` };
      const numeric = /Int|Float|Decimal/;
      return {
        mart,
        dimensions: cols.filter((c) => !numeric.test(c.type)).map((c) => c.name),
        measures: cols.filter((c) => numeric.test(c.type)).map((c) => c.name),
      };
    },
  },

  create_chart: {
    schema: {
      type: "function",
      function: {
        name: "create_chart",
        description:
          "Buat kartu chart baru di dashboard (/dashboards) dari mart Gold. Server menyusun SQL-nya " +
          "sendiri dari kolom yang kamu pilih (agregasi per dimensi) — kamu TIDAK menulis SQL. " +
          "Panggil describe_mart dulu untuk tahu kolom valid. Chart langsung tersimpan & tampil.",
        parameters: {
          type: "object",
          properties: {
            title: { type: "string", description: "judul kartu" },
            subtitle: { type: "string" },
            mart: { type: "string", description: "nama mart Gold, mis. mart_wisman" },
            kind: {
              type: "string",
              enum: ["bar", "hbar", "line", "area", "pie", "stacked", "kpi", "table", "text"],
              description: "hbar=peringkat; stacked butuh ≥2 measure; kpi=angka besar (mart+measure, tanpa dimensi); table=tabel; text=catatan markdown (isi `text`, tanpa mart)",
            },
            text: { type: "string", description: "konten markdown untuk kind=text" },
            caption: { type: "string", description: "caption/unit untuk kind=kpi" },
            dimension: { type: "string", description: "kolom kategori/waktu untuk sumbu-X" },
            measures: {
              type: "array",
              items: { type: "string" },
              description: "kolom angka yang diagregasi (1 kolom; ≥2 untuk stacked)",
            },
            breakdown: {
              type: "string",
              description:
                "opsional: kolom dimensi ke-2 untuk memecah jadi banyak seri (mis. multi-line per kawasan, " +
                "grouped/stacked bar per kategori). Pakai 1 measure saja. Tidak untuk pie.",
            },
            aggregate: { type: "string", enum: ["sum", "avg", "max", "min", "count"] },
            limit: { type: "number", description: "maks kategori (default 20)" },
            span: { type: "number", enum: [1, 2], description: "2 = lebar penuh" },
            board: { type: "string", description: "id board tujuan (opsional; default 'default'). Buat dulu via create_board bila perlu." },
          },
          required: ["title", "kind"],
        },
      },
    },
    async run(args) {
      try {
        const spec = await specFromInput(args as unknown as ChartInput, "ai", "ai");
        await insertChart(spec);
        return {
          created: true,
          id: spec.id,
          title: spec.title,
          kind: spec.kind,
          mart: spec.mart,
          board: spec.board,
          url: "/dashboards",
          note: "Chart tersimpan & langsung tampil di halaman Dashboards.",
        };
      } catch (e) {
        return { error: e instanceof Error ? e.message : String(e) };
      }
    },
  },

  update_chart: {
    schema: {
      type: "function",
      function: {
        name: "update_chart",
        description:
          "Ubah chart tersimpan (by id) — mempertahankan id, mengganti definisinya. Kirim SEMUA field " +
          "seperti create_chart (title, mart, kind, dimension, measures, dst) dengan nilai baru. " +
          "Pakai list_charts untuk tahu id.",
        parameters: {
          type: "object",
          properties: {
            id: { type: "string" },
            title: { type: "string" },
            subtitle: { type: "string" },
            mart: { type: "string" },
            kind: { type: "string", enum: ["bar", "hbar", "line", "area", "pie", "stacked", "kpi", "table", "text"] },
            dimension: { type: "string" },
            measures: { type: "array", items: { type: "string" } },
            breakdown: { type: "string" },
            aggregate: { type: "string", enum: ["sum", "avg", "max", "min", "count"] },
            limit: { type: "number" },
            span: { type: "number", enum: [1, 2] },
            board: { type: "string" },
          },
          required: ["id", "title", "kind"],
        },
      },
    },
    async run(args) {
      const id = String(args.id ?? "");
      if (!id) return { error: "id wajib" };
      try {
        const spec = await specFromInput(args as unknown as ChartInput, "ai", "ai", id);
        await insertChart(spec);
        return { updated: true, id: spec.id, title: spec.title, kind: spec.kind, mart: spec.mart };
      } catch (e) {
        return { error: e instanceof Error ? e.message : String(e) };
      }
    },
  },

  create_board: {
    schema: {
      type: "function",
      function: {
        name: "create_board",
        description: "Buat board (dashboard bernama) baru. Kembalikan id-nya untuk dipakai di create_chart.",
        parameters: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
      },
    },
    async run(args) {
      try {
        const board = await createBoard(String(args.name ?? ""));
        return { created: true, id: board.id, name: board.name, note: "Pakai id ini di create_chart.board." };
      } catch (e) {
        return { error: e instanceof Error ? e.message : String(e) };
      }
    },
  },

  list_boards: {
    schema: {
      type: "function",
      function: {
        name: "list_boards",
        description: "Daftar board (dashboard bernama) yang ada.",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      const boards = await listBoards();
      return { boards: [{ id: "default", name: "Main" }, ...boards] };
    },
  },

  suggest_dashboard: {
    schema: {
      type: "function",
      function: {
        name: "suggest_dashboard",
        description:
          "Ambil katalog SEMUA mart Gold beserta dimensi & measure-nya sekaligus — untuk MENGUSULKAN " +
          "set kartu dashboard. Pakai saat user minta 'buatkan/sarankan dashboard' tanpa detail. Setelah " +
          "ini, usulkan kartu lalu buat via create_chart.",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      const marts = await chRows<{ name: string; total_rows: string }>(
        `SELECT name, toString(total_rows) AS total_rows FROM system.tables
          WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name`,
      );
      const numeric = /Int|Float|Decimal/;
      const out = [];
      for (const m of marts) {
        const cols = await chRows<{ name: string; type: string }>(
          `SELECT name, type FROM system.columns WHERE database='serving' AND table='${m.name}' ORDER BY position`,
        );
        out.push({
          mart: m.name,
          rows: Number(m.total_rows),
          dimensions: cols.filter((c) => !numeric.test(c.type)).map((c) => c.name),
          measures: cols.filter((c) => numeric.test(c.type)).map((c) => c.name),
        });
      }
      return { marts: out };
    },
  },

  list_charts: {
    schema: {
      type: "function",
      function: {
        name: "list_charts",
        description: "Daftar kartu chart tersimpan di dashboard (yang dibuat lewat chat/UI).",
        parameters: { type: "object", properties: {} },
      },
    },
    async run() {
      const charts = await listStoredCharts();
      return {
        total: charts.length,
        charts: charts.map((c) => ({ id: c.id, title: c.title, kind: c.kind, mart: c.mart, source: c.source })),
      };
    },
  },

  delete_chart: {
    schema: {
      type: "function",
      function: {
        name: "delete_chart",
        description: "Hapus satu kartu chart tersimpan dari dashboard (by id). Spec bawaan tak bisa dihapus.",
        parameters: {
          type: "object",
          properties: { id: { type: "string" } },
          required: ["id"],
        },
      },
    },
    async run(args) {
      const id = String(args.id ?? "");
      if (!id) return { error: "id wajib" };
      try {
        await deleteChart(id);
        return { deleted: true, id };
      } catch (e) {
        return { error: e instanceof Error ? e.message : String(e) };
      }
    },
  },
};

export const TOOL_SCHEMAS = Object.values(TOOLS).map((t) => t.schema);

export async function runTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  const tool = TOOLS[name];
  if (!tool) return { error: `tool tak dikenal: ${name}` };
  return tool.run(args);
}
