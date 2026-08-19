# Mega Plan — Dashboards ala Tableau/Metabase (1:1 fitur)

Tujuan: menjadikan **Dashboards** produk BI penuh di dalam konsol lakehouse —
multi-dashboard (buat/kelola seperti chat), kanvas **drag/drop/resize**, banyak
tipe tile, filter interaktif, mode edit/lihat, tersimpan di lakehouse.

## Prinsip
- Semua tersimpan di ClickHouse (`console.*`) — dashboard hidup di lakehouse.
- Manual (UI) & agentic (AI chat) menulis artefak yang sama.
- Dependency-ringan; ECharts (Apache-2.0) untuk grafik.

## Model data (target)
- `console.bi_dashboard(id, name, layout_json, filters_json, created_at, is_deleted)`
  - `layout_json`: array tile `{ chartId, x, y, w, h }` (grid 12 kolom).
  - `filters_json`: filter tingkat-dashboard (period/dimensi).
- `console.bi_chart(... , board→dashboard_id)` — tile chart (sudah ada; +field tipe untuk KPI/tabel/teks).
- Migrasi: `bi_board` → `bi_dashboard`; `chart.board` → `chart.dashboard_id`.

## Fase

### Fase 0 — Dashboards jadi first-class + sidebar (seperti chat) ✅ target awal
- Menu **Dashboards** = section utama. Sidebar (slot bawah) menampilkan DAFTAR
  dashboard (seperti riwayat chat) + tombol **"+ Dashboard"** (= add chat).
- Route `/dashboards` (daftar / dashboard terakhir) & `/dashboards/[id]`.
- API: `console.bi_dashboard` CRUD (list/create/rename/duplicate/delete).

### Fase 1 — Kanvas grid: drag / drop / resize (mode Edit) ⭐ headline
- Grid 12-kolom; tiap tile punya `{x,y,w,h}`. Drag pindah, handle sudut resize.
- Toggle **Edit / Lihat**. Simpan layout ke `layout_json` (debounced).
- Implementasi: grid ringan buatan sendiri (pointer events) atau react-grid-layout.

### Fase 2 — Tipe tile
- Chart (bar/hbar/line/area/pie/stacked/**combo**), **KPI/angka besar**,
  **Tabel data**, **Teks/Markdown**, **Filter** (kontrol).

### Fase 3 — Editor chart lanjutan
- Sort, limit, format angka, warna, multi-measure, **filter per-chart**,
  sumbu/label, target/line. Live preview di editor.

### Fase 4 — Filter dashboard (global, interaktif)
- Bar filter: rentang tanggal, dropdown dimensi → menyuntik WHERE ke semua tile.
- **Cross-filter**: klik bar → memfilter tile lain. Drill-down.

### Fase 5 — Mode lihat / presentasi
- View bersih, fullscreen, per-tile menu (edit/duplikat/hapus), auto-refresh.

### Fase 6 — Persistensi, ekspor, share
- Layout + filter tersimpan; ekspor YAML (termasuk layout) & PNG (ECharts).
- Duplikat dashboard; template.

### Fase 7 — Agentic
- AI: "buatkan dashboard X" → buat dashboard + tile + tata letak otomatis.

## Urutan eksekusi
0 → 1 → 2 → 5 → 3 → 4 → 6 → 7 (nilai tertinggi dulu: struktur + kanvas + tile +
mode lihat, lalu editor & filter, lalu ekspor & agentic).

## Status
- [x] Fase 0  - [x] Fase 1  - [x] Fase 2  - [x] Fase 3
- [x] Fase 4  - [x] Fase 5  - [x] Fase 6  - [x] Fase 7
