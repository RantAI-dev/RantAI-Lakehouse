import type { Mode } from "./use-copilot";

/**
 * Katalog tool AI (untuk UI menu "Tools" di composer, ala RantAI-Agents).
 * `write: true` = mengubah lakehouse/dashboard → hanya tersedia di mode Build.
 * Nama harus sama persis dengan tool di services/clients/ai-tools.ts.
 */
export type ToolInfo = { name: string; label: string; desc: string; write?: boolean };

export const TOOL_CATALOG: ToolInfo[] = [
  { name: "run_sql", label: "Query SQL", desc: "Jalankan SELECT ke ClickHouse" },
  { name: "list_datasets", label: "Cari dataset", desc: "Telusuri katalog lakehouse" },
  { name: "describe_dataset", label: "Skema dataset", desc: "Kolom & jumlah baris dataset" },
  { name: "get_lineage", label: "Silsilah data", desc: "Source → Bronze → Silver" },
  { name: "get_quality", label: "Kualitas data", desc: "Ringkasan cek pass/warn/fail" },
  { name: "describe_mart", label: "Lihat mart Gold", desc: "Mart & kolom untuk chart" },
  { name: "get_build_status", label: "Status build", desc: "Run pipeline Dagster terakhir" },
  { name: "list_charts", label: "Daftar chart", desc: "Kartu dashboard tersimpan" },
  { name: "list_boards", label: "Daftar board", desc: "Dashboard bernama" },
  { name: "suggest_dashboard", label: "Rancang dashboard", desc: "Katalog semua mart untuk usul kartu" },
  { name: "create_chart", label: "Buat chart", desc: "Tambah kartu ke dashboard", write: true },
  { name: "update_chart", label: "Ubah chart", desc: "Edit kartu tersimpan", write: true },
  { name: "delete_chart", label: "Hapus chart", desc: "Hapus kartu tersimpan", write: true },
  { name: "create_board", label: "Buat board", desc: "Dashboard bernama baru", write: true },
  { name: "trigger_lakehouse_build", label: "Bangun lakehouse", desc: "Refresh Bronze→Silver→Gold", write: true },
];

/** Tool yang tersedia untuk sebuah mode (Ask menyembunyikan tool tulis). */
export function toolsForMode(mode: Mode): ToolInfo[] {
  return mode === "build" ? TOOL_CATALOG : TOOL_CATALOG.filter((t) => !t.write);
}
