# Rantai Lake — Spesifikasi Produk & Perilaku Fitur

Dokumen ini menjelaskan **spesifikasi fungsional** dan **perilaku yang diharapkan** setiap area aplikasi Rantai Lake. Dipakai sebagai referensi kontrak antara desain, produk, engineering, dan AI agent saat mengembangkan atau menguji fitur.

**Hubungan dengan dokumen lain**

- `AI_PROJECT_INSIGHTS.md` — konteks teknis (stack, struktur folder, alur data, known issues).
- `README.md` — cara menjalankan project lokal.
- `design-kit/` — referensi visual Figma & token (bukan spesifikasi perilaku).

---

## 1. Konvensi dalam dokumen ini

| Istilah | Arti |
|--------|------|
| **MUST** | Wajib; perilaku ini harus ada agar fitur dianggap benar. |
| **SHOULD** | Disarankan; ideal untuk UX/konsistensi, boleh ditunda jika ada trade-off. |
| **MAY** | Opsional; nice-to-have atau bergantung pada integrasi backend nanti. |

**Status implementasi saat ini**

- Build yang ada adalah **UI demo / preview**: data statis atau in-memory, tanpa API backend resmi.
- Bagian yang berlabel **(Mock)** menjelaskan perilaku **seperti sekarang** di kode. Bagian **(Produksi — target)** menjelaskan arah ketika backend tersedia, tanpa mengubah requirement visual kecuali disepakati terpisah.

---

## 2. Visi produk & persona

### 2.1 Visi

Rantai Lake adalah antarmuka untuk **mengelola dan menjelajahi data lakehouse**: zona data (Raw → Semantic), pipeline transformasi dan embedding vektor, query (NL + SQL), katalog, lineage, kebijakan governance, audit, dan administrasi organisasi.

### 2.2 Persona utama

| Persona | Kebutuhan utama |
|--------|------------------|
| **Data Engineer** | Membuat/monitor pipeline, lineage, konektor. |
| **Analyst** | Query Studio (NL/SQL), kolaborasi, semantic search. |
| **Data Steward / Security** | Governance, klasifikasi, masking, RLS, audit. |
| **Platform Admin** | User & tenant management. |

---

## 3. Shell aplikasi (global)

### 3.1 Layout root

| ID | Requirement |
|----|--------------|
| L-01 | MUST: Setiap halaman route berada di dalam `SidebarProvider` + `AppSidebar` (kiri) + `SidebarInset` berisi `AppNavbar` (atas) dan area konten `children` dengan padding konsisten. |
| L-02 | MUST: `ThemeProvider` (next-themes) mengatur mode `light` / `dark` / `system`; default **system**. |
| L-03 | MUST: `TooltipProvider` tersedia global untuk komponen yang memakai tooltip. |
| L-04 | SHOULD: Navbar menyediakan toggle sidebar, field pencarian, dan tombol notifikasi (ikon saja). **(Mock)** pencarian dan notifikasi tidak memanggil backend. |

### 3.2 Navigasi sidebar

| ID | Requirement |
|----|--------------|
| N-01 | MUST: Tiga grup label: **Main**, **Discover**, **Administration** dengan tautan yang terdefinisi di `app-sidebar.tsx`. |
| N-02 | MUST: Item aktif ter-highlight berdasarkan `usePathname()`: untuk `/` hanya aktif jika pathname tepat `/`; untuk route lain aktif jika pathname sama atau prefix `href/`. |
| N-03 | SHOULD: Saat sidebar collapsed (icon mode), item tetap dapat diakses (tooltip judul). |
| N-04 | **(Produksi — target)** MAY: Grup nav dan hak akses per role dikonfigurasi dari backend. |

### 3.3 Branding teks UI

| ID | Requirement |
|----|--------------|
| B-01 | MUST: Label yang mengikuti Figma (termasuk ejaan seperti pada desain) **tidak diubah** tanpa persetujuan produk/desain — termasuk judul halaman Intelligence Knowledge di UI. |

---

## 4. Dashboard (`/`)

### 4.1 Tujuan

Memberi **gambaran agregat** per zona lake dan **daftar tabel** yang dapat difilter dan di-pagination.

### 4.2 Spesifikasi perilaku

| ID | Requirement |
|----|--------------|
| D-01 | MUST: Menampilkan lima kartu ringkasan untuk layer: Raw, Bronze, Silver, Gold, Semantic — masing-masing menampilkan jumlah tabel dan ukuran penyimpanan (nilai dari fixture). |
| D-02 | MUST: Bagian "List Tables" memiliki **Tabs** per layer; hanya tabel dengan `layer` yang cocok yang ditampilkan. |
| D-03 | MUST: **Search** memfilter nama tabel dan tag (case-insensitive, partial match). |
| D-04 | MUST: Pagination **6 item per halaman**; tombol prev/next dan nomor halaman memperbarui slice data; halaman reset ke 1 saat layer atau search berubah. |
| D-05 | MUST: Jika tidak ada tabel yang cocok, menampilkan pesan empty state (teks bahwa tidak ada match). |
| D-06 | MUST: Tombol "Explore" pada kartu mengarah ke `/tables/{id}` dengan `id` dari fixture. |
| D-07 | **(Mock)** Data tabel dan summary bersumber dari konstanta di `app/page.tsx`. |
| D-08 | **(Produksi — target)** SHOULD: Summary dan list memanggil API katalog; search server-side atau debounced client-side. |

---

## 5. Detail tabel (`/tables/[id]`)

### 5.1 Tujuan

Menampilkan **metadata**, **skema kolom**, **ringkasan**, dan **preview baris** untuk satu tabel.

### 5.2 Spesifikasi perilaku

| ID | Requirement |
|----|--------------|
| T-01 | MUST: Halaman adalah **Server Component**; `id` diambil dari route params (Promise di-await sesuai Next.js). |
| T-02 | MUST: Data di-resolve lewat `getTableDetail(id)` dari `table-fixtures.ts`. |
| T-03 | MUST: Jika `id` tidak ada di map fixture, tetap render halaman dengan nama generik `Table_{id}` dan skema/preview default (bukan hard 404 halaman kosong tanpa konten). |
| T-04 | MUST: Aksi header (mis. Download, Run) tampil sebagai tombol sesuai layout desain. **(Mock)** tidak perlu memicu job nyata. |
| T-05 | **(Produksi — target)** MUST: Metadata dari metastore/API; otorisasi per tenant/user. |

---

## 6. Pipelines

### 6.1 Daftar & tab (`/pipelines`)

| ID | Requirement |
|----|--------------|
| P-01 | MUST: Dua tab: **Data pipelines** dan **Vector jobs**; tab aktif tercermin di URL: default tanpa query; tab vector dengan `?tab=vector`. |
| P-02 | MUST: Perubahan tab memanggil `router.replace` tanpa scroll jump. |
| P-03 | MUST: Sinkronisasi state tab dari `searchParams` saat navigasi browser (back/forward). |
| P-04 | MUST: Tab **Data pipelines**: kartu ringkasan angka (total, healthy, running, failed) dari fixture; grid kartu pipeline dengan status badge, alur `from → to`, metadata singkat, tombol "View details" dan "Run history". |
| P-05 | MUST: Sheet "Run history" menampilkan tabel run contoh (fixture) saat tombol Run history diklik. |
| P-06 | MUST: Tombol **Agentic Builder** membuka dialog builder. |
| P-07 | MUST: Tab **Vector jobs**: daftar job di kiri, panel pengaturan job di kanan; job dapat dipilih, diedit fieldnya, **Save** menampilkan badge "Saved" singkat, **Run now** mensimulasikan status Running lalu Idle dengan metrik diperbarui. |
| P-08 | MUST: **New vector job** menambah job baru ke daftar dan memilihnya. |
| P-09 | SHOULD: Tombol "Show info" / "Hide info" men-toggle penjelasan perbedaan data pipeline vs vector job. |
| P-10 | **(Mock)** List pipeline awal dari `SAMPLE_PIPELINES`; embedding jobs dari konstanta di page. |
| P-11 | **(Produksi — target)** MUST: CRUD pipeline & embedding jobs via API; run history dari orchestrator. |

### 6.2 Agentic Pipeline Builder (dialog)

| ID | Requirement |
|----|--------------|
| A-01 | MUST: Field: model AI, instruksi NL (wajib), upload file opsional (hanya nama file disimpan di state), sumber database. |
| A-02 | MUST: Tanpa instruksi kosong, tampilkan error validasi (teks) dan jangan lanjutkan generasi. |
| A-03 | MUST: Saat generate: tampilkan fase loading bergantian (teks fase), lalu tutup dialog dan **prepend** pipeline baru ke list. |
| A-04 | MUST: Pipeline yang dihasilkan disimpan ke `sessionStorage` dengan key `rantai-pipeline-{id}` agar halaman detail dapat membacanya. |
| A-05 | MUST: Saat dialog ditutup, form di-reset ke default. |
| A-06 | **(Mock)** Tidak ada panggilan LLM/orchestrator nyata. |
| A-07 | **(Produksi — target)** MUST: Panggilan async ke layanan agent; streaming error handling; idempotensi. |

### 6.3 Detail pipeline (`/pipelines/[id]`)

| ID | Requirement |
|----|--------------|
| PD-01 | MUST: Urutan resolusi data: (1) `sessionStorage` untuk id yang sama, (2) `getStaticPipelineById(id)`. |
| PD-02 | MUST: Jika id route tidak valid: pesan "Missing pipeline id". |
| PD-03 | MUST: Jika pipeline tidak ditemukan: state "Pipeline not found" dengan penjelasan dan link kembali ke daftar. |
| PD-04 | MUST: Jika ada `lastSuccessResult`, tampilkan kartu output run (tabel preview, catatan jika `previewNote` ada). |
| PD-05 | MUST: Jika status Failed tanpa output sukses, tampilkan kartu penjelasan tidak ada output baru. |
| PD-06 | MUST: Bagian Steps, Source, Destination, Transformation logic dari `detail` pipeline atau fallback `defaultDetailFromCard`. |
| PD-07 | **(Produksi — target)** MUST: Detail dari API; RBAC untuk melihat transform/credentials. |

### 6.4 Buat pipeline — wizard (`/pipelines/create`)

| ID | Requirement |
|----|--------------|
| W-01 | MUST: Empat langkah berurutan: Source → Transform → Target → Review; stepper kiri menunjukkan langkah aktif dan selesai. |
| W-02 | MUST: **Next** disabled sampai validasi langkah terpenuhi: Source (nama pipeline + zone + table + kolom inkremental); Transform (minimal satu chip transform ATAU FBIC on); Target (zone + table). |
| W-03 | MUST: **Previous** mengurangi index step; stepper boleh diklik mundur ke step yang sudah pernah dicapai (sesuai implementasi saat ini). |
| W-04 | MUST: Langkah Review menampilkan ringkasan read-only semua pilihan. |
| W-05 | MUST: Tombol final mengarahkan ke `/pipelines` (navigasi saja). **(Mock)** tidak membuat resource backend. |
| W-06 | MUST: Label/chip transform mengikuti desain Figma (termasuk ejaan khusus pada chip tertentu). |

### 6.5 Redirect

| ID | Requirement |
|----|--------------|
| R-01 | MUST: `/embeddings` → redirect ke `/pipelines?tab=vector` (lihat `src/app/embeddings/page.tsx`). |
| R-02 | MUST: `/similarity-explorer` → redirect ke `/semantic-search`. |

---

## 7. Query Studio (`/query-studio`)

### 7.1 Tab Natural Language

| ID | Requirement |
|----|--------------|
| Q-NL-01 | MUST: Tab "Natural language" vs "SQL editor" jelas dan state tab independen di halaman. |
| Q-NL-02 | MUST: Saat idle: carousel **topic cards** dengan navigasi prev/next halaman topik; tombol panah pada kartu mengisi prompt dengan deskripsi topik. |
| Q-NL-03 | MUST: Setelah run: menampilkan judul permintaan aktif, indikator "SQL query is running" selama fase generating, lalu generated SQL (pre), cost dock (pre-run + post-run setelah selesai), grid hasil sample, tab **Result Query** vs **Query explanation (AI trace)**. |
| Q-NL-04 | MUST: Submit prompt: Enter di input atau tombol submit; disabled saat generating; tidak boleh double-submit saat generating. |
| Q-NL-05 | MUST: **Saved** sheet: simpan prompt NL saat ini ke daftar in-memory (dedupe, cap); klik item mengisi kembali prompt dan pindah ke tab NL. |
| Q-NL-06 | MUST: **History** sheet: daftar entri NL/SQL dengan badge jenis dan waktu relatif; tap buka detail percakapan; **Load into editor** memuat pesan user terakhir ke NL atau SQL sesuai jenis entri. |
| Q-NL-07 | MUST: **Prompt actions menu** (`+`): upload dokumen (append nama file ke prompt), pilih model agent, toggle Intelligence Knowledge + multi-select entri knowledge, mention table/schema (append `@token`). |
| Q-NL-08 | SHOULD: **Dictation**: jika browser mendukung Web Speech API, tombol mic aktif; jika tidak, disabled dengan aria-label yang menjelaskan. |
| Q-NL-09 | MUST: Prompt bar fixed bottom saat generating/ready (spacing bottom untuk konten). |
| Q-NL-10 | **(Mock)** Delay generasi tetap (~2s); SQL dan baris hasil dari fungsi helper, bukan engine warehouse. |
| Q-NL-11 | **(Produksi — target)** MUST: Streaming response; cancellation; rate limit; logging query. |

### 7.2 Tab SQL editor

| ID | Requirement |
|----|--------------|
| Q-SQL-01 | MUST: Editor CodeMirror dengan syntax SQL, line numbers, tema mengikuti dark/light. |
| Q-SQL-02 | MUST: **Explain** cost dock memperbarui estimasi saat teks SQL berubah (debounce singkat) ketika tab SQL aktif. |
| Q-SQL-03 | MUST: **Run** memulai streaming baris ke grid dari fixture; menampilkan badge Streaming; setelah selesai mengisi post-run metrics. |
| Q-SQL-04 | MUST: **Reset sample** mengembalikan SQL ke placeholder contoh. |
| Q-SQL-05 | MUST: Sebelum run pertama, placeholder grid hasil (empty state). |
| Q-SQL-06 | MUST: Setelah run, tampil **Query explanation (AI trace)** sebagai ordered list langkah mock. |
| Q-SQL-07 | **(Produksi — target)** MUST: Eksekusi terbatas oleh warehouse policy; hasil kolom dinamis sesuai query. |

---

## 8. Query Studio — Kolaborasi

### 8.1 Daftar proyek (`/query-studio/collaboration`)

| ID | Requirement |
|----|--------------|
| C-01 | MUST: Breadcrumb: Query Studio → Collaboration. |
| C-02 | MUST: Daftar proyek dari fixture `MOCK_COLLABORATION_PROJECTS` + proyek yang dibuat user di sesi (in-memory). |
| C-03 | MUST: **Create Project** membuka sheet: nama wajib, kolaborator (comma-separated) wajib minimal satu; validasi error teks; preview badge nama; submit menambah proyek dan menutup sheet. |
| C-04 | MUST: Link ke detail proyek per `id` (slug-like). |

### 8.2 Studio kolaboratif per proyek (`/query-studio/collaboration/[projectId]`)

| ID | Requirement |
|----|--------------|
| CP-01 | MUST: Jika `projectId` tidak ada di fixture + bukan proyek baru di memori yang sama id, tampilkan **Project not found** dengan tombol kembali ke daftar. |
| CP-02 | MUST: Jika ditemukan: header proyek, anggota dengan status (online/away/offline), tab NL vs SQL mirip Query Studio utama tetapi disederhanakan. |
| CP-03 | MUST: Run NL/SQL mensimulasikan loading dan hasil mock (sesuai implementasi file). |
| CP-04 | **(Produksi — target)** MUST: Presence real-time, lock edit, versi query, audit siapa menjalankan query. |

---

## 9. Intelligence Knowledge (`/intelligence-knowledge`)

| ID | Requirement |
|----|--------------|
| IK-01 | MUST: Dua tab: **Query knowledge** dan **Documents & context**. |
| IK-02 | MUST: Query knowledge: tabel entri dapat di-expand untuk source query, ringkasan, statistik users/rows; tombol ingest mensimulasikan delay lalu menambah satu entri mock. |
| IK-03 | MUST: Documents: upload multi-file menambah baris dokumen (metadata file, bukan parse isi); **New context** membuka sheet; save membutuhkan title + body; cancel/close mereset form sesuai implementasi. |
| IK-04 | MUST: Row dokumen expandable menampilkan detail upload vs web context. |
| IK-05 | **(Produksi — target)** MUST: Indexing ke vector store / OCR / virus scan sesuai kebijakan. |

---

## 10. Data Catalog (`/data-catalog`)

| ID | Requirement |
|----|--------------|
| DC-01 | MUST: Menjelajahi aset/tabel dengan metadata (zona, owner, klasifikasi, permission). |
| DC-02 | MUST: Detail aset dalam sheet/drawer sesuai UI (search, filter jika ada). |
| DC-03 | **(Mock)** Data dari konstanta di page. |
| DC-04 | **(Produksi — target)** SHOULD: Integrasi dengan metastore (Glue, Unity Catalog, dll.). |

---

## 11. Data Governance (`/data-governance`)

| ID | Requirement |
|----|--------------|
| DG-01 | MUST: Multi-tab: Classification, Masking, Retention, Access, Data Quality, RLS (sesuai konstanta tab di page). |
| DG-02 | MUST: Setiap tab menampilkan tabel/kartu kebijakan dengan data contoh; tombol aksi header (+) tampil untuk menambah kebijakan **(Mock)** tanpa persist. |
| DG-03 | MUST: Badge level/status konsisten (warna restricted vs public, dll.). |
| DG-04 | **(Produksi — target)** MUST: Evaluasi kebijakan terikat engine query (mis. column masking, RLS). |

---

## 12. Lineage (`/lineage`)

| ID | Requirement |
|----|--------------|
| LN-01 | MUST: Pemilih **Focus table** mengubah diagram alur node (upstream → target) sesuai map fixture. |
| LN-02 | MUST: Sub-tab **Table lineage** vs **Column lineage**; column lineage menampilkan mapping kolom ke `gold.orders_fact` (teks statis sesuai implementasi). |
| LN-03 | **(Produksi — target)** SHOULD: Graf interaktif (zoom, export OpenLineage). |

---

## 13. Connectors (`/connectors`)

| ID | Requirement |
|----|--------------|
| CO-01 | MUST: Kartu per konektor dengan status (Connected, dll.), toggle enable/disable memperbarui state lokal. |
| CO-02 | **(Mock)** Tidak memanggil orchestrator untuk pause job. |
| CO-03 | **(Produksi — target)** MUST: Health check & secret rotation. |

---

## 14. Semantic Search (`/semantic-search`)

| ID | Requirement |
|----|--------------|
| SS-01 | MUST: Tab pemisahan antara pencarian semantik dan similarity (sesuai UI). |
| SS-02 | MUST: Hasil berupa daftar hit dengan skor, snippet, zona — dari fixture / simulasi. |
| SS-03 | **(Produksi — target)** MUST: Vector search terhadap index yang dikonfigurasi di Pipelines → Vector jobs. |

---

## 15. Audit Logs (`/audit-logs`)

| ID | Requirement |
|----|--------------|
| AL-01 | MUST: Tabel log dengan kolom waktu, event, user, outcome, dll. sesuai UI. |
| AL-02 | SHOULD: Filter/pencarian jika disediakan di implementasi. |
| AL-03 | **(Produksi — target)** MUST: Export immutable log; retensi compliance. |

---

## 16. User Management (`/user-management`)

| ID | Requirement |
|----|--------------|
| UM-01 | MUST: Daftar user dengan role, status, MFA, provider, grup; sheet detail untuk satu user. |
| UM-02 | MUST: Aksi administratif mengikuti pola UI (invite, suspend, dll.) sesuai tombol yang ada. **(Mock)** state lokal saja. |
| UM-03 | **(Produksi — target)** MUST: SSO group sync; audit trail per perubahan role. |

---

## 17. Tenant Management (`/tenant-management`)

| ID | Requirement |
|----|--------------|
| TM-01 | MUST: Daftar tenant / organisasi dengan metadata sesuai UI. |
| TM-02 | **(Mock)** Tanpa API multi-tenant nyata. |
| TM-03 | **(Produksi — target)** MUST: Isolasi data per tenant; limit kuota. |

---

## 18. Non-fungsional (cross-cutting)

| ID | Requirement |
|----|--------------|
| NF-01 | MUST: Aplikasi buildable dengan `npm run build` dan typecheck bersih (`tsc --noEmit`) pada baseline repo. |
| NF-02 | SHOULD: Tidak menambah error ESLint baru; known lint intentional didokumentasikan di `AI_PROJECT_INSIGHTS.md`. |
| NF-03 | SHOULD: Komponen interaktif memiliki `aria-label` untuk tombol ikon-only dimana sudah ada pola tersebut. |
| NF-04 | **(Produksi — target)** MUST: Autentikasi, otorisasi, CSRF, rate limit API, i18n jika diperlukan. |

---

## 19. Matriks route → dokumen

| Route | Bagian spek |
|-------|-------------|
| `/` | §4 Dashboard |
| `/tables/[id]` | §5 Detail tabel |
| `/pipelines` | §6.1 |
| `/pipelines/create` | §6.4 |
| `/pipelines/[id]` | §6.3 |
| `/embeddings` | §6.5 Redirect |
| `/query-studio` | §7 |
| `/query-studio/collaboration` | §8.1 |
| `/query-studio/collaboration/[projectId]` | §8.2 |
| `/intelligence-knowledge` | §9 |
| `/data-catalog` | §10 |
| `/data-governance` | §11 |
| `/lineage` | §12 |
| `/connectors` | §13 |
| `/semantic-search` | §14 |
| `/similarity-explorer` | §6.5 Redirect |
| `/audit-logs` | §15 |
| `/user-management` | §16 |
| `/tenant-management` | §17 |
| Shell global | §3 |

---

## 20. Maintenance dokumen ini

| Kapan update | Apa yang diubah |
|--------------|------------------|
| Fitur baru / route baru | Tambah section + baris matriks §19. |
| Perubahan perilaku mock | Sesuaikan ID requirement dan tandai **(Mock)**. |
| Rilis ke produksi | Tambah/ubah bagian **(Produksi — target)**; pindahkan MUST yang sudah terpenuhi ke checklist release. |
| Perubahan besar arsitektur | Sinkronkan dengan `AI_PROJECT_INSIGHTS.md`. |

---

_Dokumen ini menjelaskan **harusnya** fitur berperilaku dari sudut produk dan QA; implementasi aktual harus tetap diverifikasi di kode dan dengan uji manual._
