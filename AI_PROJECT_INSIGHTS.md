# AI Project Insights

Dokumen ini adalah ringkasan konteks project untuk membantu developer atau AI berikutnya melanjutkan pekerjaan tanpa harus membaca seluruh kode dari nol. Update file ini setiap kali ada perubahan besar (struktur, dependency, alur fitur, atau aturan baru).

---

## 1. Project Overview

**Rantai Lake** (`rantai-lake`) adalah UI front-end untuk sebuah platform data lakehouse. Aplikasi ini menampilkan operasi-operasi khas data platform: dashboard zona data (Raw → Bronze → Silver → Gold → Semantic), CRUD pipeline (data + vector embeddings), studio query (Natural Language + SQL), katalog data, lineage, governance, audit, dan administrasi user/tenant.

Catatan penting:

- **Ini adalah UI preview / demo**. Hampir semua data berasal dari fixture in-memory di `src/lib/*-fixtures.ts`. **Tidak ada API call ke backend nyata.** "Run pipeline", "Generate SQL", dan "Ingest" semuanya disimulasikan dengan `setTimeout`.
- Persistensi terbatas pada `sessionStorage` untuk pipeline draft yang baru di-generate (key: `rantai-pipeline-<id>`).
- Visual identity mengikuti **Rantai Design System** (`design-system/`) — navy/blue OKLCH, Geist, forced dark.

---

## 2. Tech Stack

| Kategori | Pilihan |
|---|---|
| Framework | **Next.js 16** (App Router, React 19) |
| Bahasa | **TypeScript 5** (strict, alias `@/*` → `./src/*`, `@rantai/design-system/*` → `./design-system/*`) |
| Styling | **Tailwind CSS v4** + `tw-animate-css` + token dari `design-system/` |
| Komponen UI | **shadcn/ui** di `src/components/ui/` + primitif `@base-ui/react`; tokens/fonts/theme dari design system |
| Ikon | `lucide-react` |
| Theme / Dark mode | `next-themes` via design-system ThemeProvider (**forced dark**) |
| SQL editor | `@uiw/react-codemirror` + `@codemirror/lang-sql` + `@codemirror/theme-one-dark` |
| Class utils | `clsx` + `tailwind-merge` (digabung di `cn()` di `src/lib/utils.ts`) |
| Variants | `class-variance-authority` (cva) untuk Button |
| Fonts | Geist + Geist Mono — dari `@rantai/design-system/fonts/fonts` di `app/layout.tsx` |
| Routing | Next App Router file-based |
| State management | **React lokal saja** — `useState` / `useReducer` per page. Tidak ada Redux / Zustand / Context global aplikasi. Hanya `ThemeProvider`, `TooltipProvider`, dan `SidebarProvider` di root. |
| Lint | ESLint 9 + `eslint-config-next` |

Lihat versi pasti di `package.json`. Jangan ganti library besar tanpa instruksi eksplisit.

---

## 3. Folder Structure

```
rantai-lake/
├── design-kit/                   # Aset desain (Figma) + tokens — referensi visual
├── public/                       # Aset statis (favicon, rantai-logo.png)
├── src/
│   ├── app/                      # Next App Router
│   │   ├── layout.tsx            # Root layout (Theme/Tooltip/Sidebar providers)
│   │   ├── page.tsx              # "/" — Dashboard
│   │   ├── globals.css           # Tailwind v4 + design tokens (palet, dark mode)
│   │   │
│   │   ├── pipelines/
│   │   │   ├── page.tsx          # List + 2 tabs (data/vector)
│   │   │   ├── create/page.tsx   # 4-step wizard
│   │   │   └── [id]/page.tsx     # Detail per pipeline
│   │   │
│   │   ├── query-studio/
│   │   │   ├── page.tsx          # NL + SQL editor (file paling besar, ~1800 baris)
│   │   │   └── collaboration/
│   │   │       ├── page.tsx
│   │   │       └── [projectId]/page.tsx
│   │   │
│   │   ├── tables/[id]/page.tsx  # Detail tabel (server component)
│   │   ├── data-catalog/page.tsx
│   │   ├── data-governance/page.tsx
│   │   ├── intelligence-knowledge/
│   │   │   ├── layout.tsx
│   │   │   └── page.tsx
│   │   ├── lineage/page.tsx
│   │   ├── connectors/page.tsx
│   │   ├── audit-logs/page.tsx
│   │   ├── semantic-search/page.tsx
│   │   ├── user-management/page.tsx
│   │   ├── tenant-management/page.tsx
│   │   ├── embeddings/page.tsx          # Redirect ke /pipelines?tab=vector
│   │   └── similarity-explorer/page.tsx # Redirect ke /semantic-search
│   │
│   ├── components/
│   │   ├── app-navbar.tsx               # Navbar atas (search + bell)
│   │   ├── app-sidebar.tsx              # Sidebar kiri (3 grup nav)
│   │   ├── sql-editor.tsx               # Wrapper CodeMirror untuk SQL
│   │   ├── shared/
│   │   │   └── theme-provider.tsx       # Wrapper next-themes
│   │   ├── pipelines/
│   │   │   └── agentic-pipeline-builder-dialog.tsx
│   │   ├── query-studio/
│   │   │   └── prompt-actions-menu.tsx  # "+" menu di prompt NL
│   │   └── ui/                          # shadcn primitives (jangan diedit manual kecuali perlu)
│   │
│   ├── lib/
│   │   ├── utils.ts                     # cn() helper
│   │   ├── pipeline-fixtures.ts         # SAMPLE_PIPELINES + getStaticPipelineById
│   │   ├── pipeline-agent-output.ts     # buildAgentPipelineSuccessOutput
│   │   ├── table-fixtures.ts            # TABLE_DETAIL_BY_ID + getTableDetail
│   │   ├── knowledge-library-fixtures.ts# Knowledge entries, agent models, mention tokens
│   │   └── query-collaboration-fixtures.ts
│   │
│   ├── hooks/
│   │   └── use-mobile.ts                # useIsMobile() — dipakai oleh Sidebar
│   │
│   └── types/
│       ├── index.ts                     # Re-export shared types (saat ini kosong)
│       └── pipeline.ts                  # PipelineItem, PipelineStatus, PipelineRunResult, dll.
│
├── AI_PROJECT_INSIGHTS.md       # File ini
├── README.md                    # Setup singkat
├── package.json                 # Dependency + scripts (dev/build/start/lint)
├── eslint.config.mjs
├── next.config.ts               # Hampir kosong (default Next.js)
├── postcss.config.mjs
├── tsconfig.json
└── components.json              # Konfigurasi shadcn
```

---

## 4. Main Features

| Fitur | Route | Cara kerja singkat |
|---|---|---|
| Dashboard | `/` | Lima summary card per zona lake + tabel tiles dengan tabs Layer + search + paginasi (6/halaman). Klik "Explore" → `/tables/[id]`. |
| Tables Detail | `/tables/[id]` | Server component. Memanggil `getTableDetail(id)` dari fixtures. |
| Pipelines | `/pipelines` | Dua tab: **Data pipelines** (list card pipeline) dan **Vector jobs** (list + form embedding job). Tab dipersist di URL `?tab=vector`. |
| Pipeline Detail | `/pipelines/[id]` | Coba ambil dari `sessionStorage` dulu (untuk draft agentic), lalu fallback ke `getStaticPipelineById`. Render run output + steps + source/destination/transform. |
| Create Pipeline | `/pipelines/create` | Wizard 4 step: Source → Transform → Target → Review. Validasi per step memblokir Next. Submit hanya navigasi balik ke `/pipelines`. |
| Agentic Builder | Dialog di `/pipelines` | Dialog dengan model + instruksi NL + file + DB source. Simulasi multi-fase (~3.5 detik) lalu emit pipeline baru ke list dan persist ke sessionStorage. |
| Query Studio | `/query-studio` | Dua tab: **Natural language** dan **SQL editor**. NL mensimulasikan generasi SQL + 3-row sample + AI explanation. SQL editor men-stream `SAMPLE_SQL_STREAM_ROWS` ke grid, plus pre-run dan post-run cost dock. Includes saved prompts sheet, prompt history sheet, dan dictation (Web Speech API). |
| Collaboration | `/query-studio/collaboration` & `/[projectId]` | List proyek kolaborasi, detail per proyek (members + activities + history). |
| Intelligence Knowledge | `/intelligence-knowledge` | Dua tab: **Query knowledge** (mock-ingest dari last query) & **Documents & context** (upload file atau tulis web context). Row dapat di-expand. |
| Data Catalog | `/data-catalog` | Browser metadata aset/tabel. |
| Data Governance | `/data-governance` | Tabs: classification, masking, retention, access policies, data quality, RLS. |
| Lineage | `/lineage` | Pilih tabel fokus → tampilkan upstream → target diagram + column mapping. |
| Connectors | `/connectors` | Daftar koneksi (PostgreSQL, S3) dengan toggle enable/disable. |
| Audit Logs | `/audit-logs` | Tabel audit dengan filter outcome. |
| Semantic Search | `/semantic-search` | Tabs: search + similarity. Mock hits. |
| User Management | `/user-management` | Tabel user dengan role/status, sheet detail. |
| Tenant Management | `/tenant-management` | Tabel tenant. |

---

## 5. Important Components

| Komponen | Lokasi | Tanggung jawab |
|---|---|---|
| `RootLayout` | `src/app/layout.tsx` | Root layout. Memasang `ThemeProvider` → `TooltipProvider` → `SidebarProvider` → `AppSidebar` + `AppNavbar` + `{children}`. Server component — jangan tambahkan state client di sini. |
| `AppSidebar` | `src/components/app-sidebar.tsx` | Sidebar 3 grup (Main / Discover / Administration). Menambah halaman top-level = tambah entry di array `mainNav` / `discoverNav` / `adminNav`. |
| `AppNavbar` | `src/components/app-navbar.tsx` | Header atas: tombol toggle sidebar, search input, ikon bell. Sticky di seluruh halaman. |
| `ThemeProvider` | `src/components/shared/theme-provider.tsx` | Thin wrapper `next-themes` agar root layout tetap server component. |
| `SqlEditor` | `src/components/sql-editor.tsx` | CodeMirror + extension SQL. Mengikuti tema next-themes. |
| `AgenticPipelineBuilderDialog` | `src/components/pipelines/agentic-pipeline-builder-dialog.tsx` | Dialog "AI generate pipeline" di list pipelines. Memanggil `onPipelineGenerated` setelah simulasi. |
| `PromptActionsMenu` | `src/components/query-studio/prompt-actions-menu.tsx` | Menu "+" di prompt NL Query Studio: upload, model agent, knowledge sources, mention table/schema. Stateless — semua state di parent. |
| Komponen `ui/*` | `src/components/ui/` | Wrappers shadcn untuk Button, Card, Dialog, Sheet, Sidebar, Tabs, Table, dll. **Jangan dimodifikasi manual kecuali sangat perlu** — diregenerasi via `npx shadcn@latest add ...`. Saat ini ada perubahan di `button.tsx` (custom variant brand). |

---

## 6. Important Functions & Hooks

### Helpers (`src/lib/`)

| Fungsi | Lokasi | Kegunaan |
|---|---|---|
| `cn(...inputs)` | `src/lib/utils.ts` | Menggabungkan className kondisional + dedupe utilitas Tailwind yang konflik. **Selalu pakai ini** untuk merge className. |
| `getStaticPipelineById(id)` | `src/lib/pipeline-fixtures.ts` | Mengambil pipeline fixture berdasarkan id. Dipakai di pipeline detail page sebagai fallback. |
| `buildAgentPipelineSuccessOutput(name)` | `src/lib/pipeline-agent-output.ts` | Bangun payload "last successful run" untuk pipeline yang baru di-generate agentic builder. |
| `getTableDetail(id)` | `src/lib/table-fixtures.ts` | Detail tabel dengan default ketika id tidak dikenal. |
| `getCollaborationProjectById(id)` | `src/lib/query-collaboration-fixtures.ts` | Lookup proyek kolaborasi (return `null` jika tidak ada). |

### Hooks

| Hook | Lokasi | Kegunaan |
|---|---|---|
| `useIsMobile()` | `src/hooks/use-mobile.ts` | True jika viewport < 768px. Dipakai oleh `Sidebar` untuk switch mode mobile/desktop. |
| `useSpeechRecognition(onTranscript, onEnd)` | inline di `src/app/query-studio/page.tsx` | Wrap Web Speech API untuk dictation. Return `{ supported, listening, start, stop }`. |

### Helpers internal di Query Studio

`src/app/query-studio/page.tsx` punya banyak helper untuk simulasi cost / explain:

- `formatBytesShort`, `formatDurationMs` → format display
- `stringHash` → hash deterministik (djb2 variant)
- `mockSqlPreRunEstimate(sql)` → bytes / partitions / credit band dari heuristik (JOIN, LIMIT, WHERE)
- `buildPostRunMetrics(pre, wallMs, rows)` → wall time + bytes scanned + credits
- `buildNlSqlPreview(question)`, `sampleNlResultRows`, `buildNlNaturalExplanation` → mock NL → SQL flow
- History helpers: `newHistoryId`, `formatHistoryWhen`, `prependHistoryEntry`, `appendAssistantMessage`, `lastUserMessage`

---

## 7. Data Flow

Karena tidak ada API real, alurnya selalu berbentuk:

```
User interaction
   ↓
useState / useCallback handler di page
   ↓
Baca fixture dari src/lib/*-fixtures.ts (atau bangun objek baru)
   ↓
setState → React re-render
   ↓
UI update
```

Beberapa alur khusus:

1. **Pipeline draft (agentic)**:
   `AgenticPipelineBuilderDialog.handleGenerate` → simulasi setTimeout → emit `PipelineItem` ke parent (`/pipelines/page.tsx`) → `handleAgenticPipelineGenerated` → `persistPipelineForDetail` (sessionStorage) + prepend ke list. Saat user klik "View details", `/pipelines/[id]/page.tsx` membaca dari sessionStorage dulu sebelum fallback ke fixture.

2. **Tab di Pipelines**:
   State `tab` ↔ URL `?tab=...` 2-arah. `setTabWithUrl` pakai `router.replace(scroll: false)`. Pada `useSearchParams` change, `useEffect` re-syncs state.

3. **NL Query Studio**:
   User type / dictate → `nlPrompt` state → klik submit → `runNl()` → `setTimeout(NL_GENERATE_MS)` → render generated SQL + 3-row table + explanation + cost dock metrics. History entry di-prepend di awal untuk "Just now" feedback.

4. **SQL Query Studio**:
   `runSql()` → start `setInterval(320ms)` mendorong satu row per tick dari `SAMPLE_SQL_STREAM_ROWS` ke grid → ketika habis, hitung `buildPostRunMetrics`.

5. **Theme**:
   `ThemeProvider` (next-themes) → `<html class="dark">` toggle → CSS variables di `globals.css` swap.

---

## 8. State Management

Tidak ada global state library. Yang ada:

- **`SidebarProvider` (`src/components/ui/sidebar.tsx`)** — context untuk open/closed state sidebar (cookie-based persist via `SIDEBAR_COOKIE_NAME`).
- **`TooltipProvider`** — global config untuk shadcn tooltip.
- **`ThemeProvider`** — light/dark/system.
- **State per-page** — semua data fitur (pipelines list, embedding jobs, NL prompt, history, dll.) tinggal di `useState` halaman terkait.
- **`sessionStorage`** — hanya untuk pipeline draft (`rantai-pipeline-<id>`). Hilang saat tab ditutup.

State penting yang patut diingat:

| Page | State kunci |
|---|---|
| `/` | `activeLayer`, `searchQuery`, `currentPage` |
| `/pipelines` | `tab`, `pipelines`, `embeddingJobs`, `selectedEmbId`, `agenticBuilderOpen`, `runHistoryFor`, `showPipelineInfo` |
| `/pipelines/create` | `step`, plus 8 state field form |
| `/query-studio` | `tab`, `nlPrompt`, `sqlQuery`, `nlRunState`, `nlGeneratedSql`, `nlResultRows`, `sqlStreamRows`, `sqlStreaming`, `promptHistory`, `saved`, `nlAgentModel`, `nlUseIntelligenceKnowledge`, `nlKnowledgeSourceIds`, dll. (banyak refs untuk timer) |
| `/intelligence-knowledge` | `entries`, `userDocuments`, `expandedId`, `docExpandedId`, `contextOpen` |

---

## 9. API / Mock Data

**Tidak ada endpoint API yang dipanggil**. Semua data berasal dari file di `src/lib/`:

| File | Isi |
|---|---|
| `pipeline-fixtures.ts` | `SAMPLE_PIPELINES`, `getStaticPipelineById` |
| `pipeline-agent-output.ts` | `buildAgentPipelineSuccessOutput` |
| `table-fixtures.ts` | `TABLE_DETAIL_BY_ID`, `getTableDetail` |
| `knowledge-library-fixtures.ts` | `KNOWLEDGE_LIBRARY_INITIAL_ENTRIES`, `USER_KNOWLEDGE_DOCUMENTS_INITIAL`, `AGENT_MODEL_OPTIONS`, `SCHEMA_TABLE_MENTIONS` |
| `query-collaboration-fixtures.ts` | `MOCK_COLLABORATION_PROJECTS`, `getCollaborationProjectById` |

Beberapa page menyimpan fixture inline di file page (mis. `app/page.tsx` punya `SAMPLE_TABLES` & `SUMMARY_DATA`, `app/audit-logs/page.tsx` punya `SAMPLE_AUDIT_LOGS`, dll). Saat menambah fitur baru, **letakkan fixture besar di `src/lib/`** agar reusable.

Saat menambahkan integrasi backend nanti, ikuti pola: ganti fixture import dengan service call, dan biarkan komponen presentasional tidak berubah.

---

## 10. UI & Styling Notes

### Pola styling

- **Tailwind utility-first** dengan `cn(...)` untuk komposisi kondisional.
- **Design tokens** ada di `src/app/globals.css` (palet primary 50–950, sidebar tokens, dll.) — referensikan via CSS vars `var(--color-primary-700)` atau utility `bg-primary` / `text-primary` / `border-border`.
- Brand color utama: `--color-primary-700` = `#ff5001` (oranye NQ).
- **Dark mode** via `next-themes` + custom variant `@custom-variant dark (&:is(.dark *))`.
- **Font**: `--font-inter` (UI), `--font-montserrat` (brand display), monospace fallback Menlo. Beberapa halaman secara eksplisit `font-[family-name:var(--font-montserrat)]` untuk judul tertentu.
- **Shadow** umum: `shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]` (card subtle) dan `shadow-[0px_1px_2px_0px_rgba(0,0,0,0.1),0px_1px_3px_0px_rgba(0,0,0,0.1)]` (card lebih kuat).
- **Status colors**:
  - Success / healthy: `bg-[#ecfdf2] text-[#008a2e]`
  - Failed / destructive: `bg-destructive/10 text-destructive`
  - Running / info: `bg-primary/10 text-primary`

### Aturan visual yang TIDAK BOLEH diubah tanpa instruksi

- Layout root (sidebar kiri + navbar atas + konten `p-4`).
- Palet primer (oranye `#ff5001` series).
- Struktur header per-halaman (judul + subtitle dengan `border-b border-border pb-2`).
- Spelling label UI yang sudah ada (misal `"Intellegance Knowladge"`, `"Agregate"`) — ini **disengaja** mengikuti Figma dan tidak boleh "diperbaiki". Lihat komentar di `src/app/pipelines/create/page.tsx` (`/* Figma Transform tab — labels match design (incl. "Agregate" spelling). */`).

---

## 11. Known Issues

Issue yang ditemukan saat review (pre-existing, bukan dari perubahan review ini):

1. **`react-hooks/set-state-in-effect` error (ESLint):**
   - `src/app/pipelines/page.tsx` (`useEffect` yang sync `tab` dari URL searchParams).
   - `src/app/query-studio/page.tsx` di `useSpeechRecognition` (`setSupported(!!getSpeechRecognitionCtor())` di mount).

   Keduanya disengaja untuk:
   - Pipelines: hindari hydration flicker karena `router.replace` async.
   - Query Studio: deteksi support `SpeechRecognition` butuh `window`, hanya bisa setelah mount agar tidak SSR-mismatch.

   Status: dibiarkan untuk menjaga behavior. Jika ingin diperbaiki nanti, gunakan derived value (useMemo dari URL) atau lazy initializer dengan guard SSR — tetapi PERLU pengujian manual untuk memastikan tab tidak flicker saat back/forward.

2. **Warning unused `_question`** di `src/app/query-studio/page.tsx` line 211. Parameter sengaja diberi prefix underscore untuk tetap menyimpan signature yang dapat menerima context untuk implementasi masa depan. Aman.

3. **Warning `react-hooks/exhaustive-deps`** di `src/components/ui/sidebar.tsx` line 91 — file shadcn auto-generated. **Jangan** edit manual.

4. **File terlalu besar:**
   - `src/app/query-studio/page.tsx` ~1827 baris. Mengandung beberapa sub-komponen (`SqlMetricRow`, `SqlCostInlineDock`, `HistoryDetailPanel`), helper SQL cost, dan hook dictation. Kandidat split yang aman tanpa mengubah behavior:
     - Pisah `SqlCostInlineDock` & `SqlMetricRow` → `src/components/query-studio/sql-cost-dock.tsx`
     - Pisah helper cost (`formatBytesShort`, `mockSqlPreRunEstimate`, `buildPostRunMetrics`, dll.) → `src/lib/sql-cost.ts`
     - Pisah hook `useSpeechRecognition` → `src/hooks/use-speech-recognition.ts`
     - Pisah `HistoryDetailPanel` & helper history → `src/components/query-studio/history-panel.tsx`

     **Belum dilakukan** karena bisa berisiko menyentuh banyak ref/state. Lakukan dengan testing manual menyeluruh per area.
   - `src/app/pipelines/page.tsx` ~912 baris.
   - `src/app/semantic-search/page.tsx` ~809 baris.
   - `src/app/data-catalog/page.tsx` ~792 baris.

5. **Duplikasi fixture:** `FALLBACK_DETAILS["1"]` dan `FALLBACK_DETAILS["2"]` di `src/app/pipelines/[id]/page.tsx` identik. Bisa di-konsolidasikan jika diinginkan.

6. **Persistence draft pipeline pakai `sessionStorage`**: data hilang saat tab ditutup. Fine untuk demo, tapi bukan UX produksi.

7. **Spelling label** `"Intellegance Knowladge"` (sidebar + page) konsisten dengan Figma; jika ingin koreksi typo, lakukan serentak: `app-sidebar.tsx`, `app/intelligence-knowledge/layout.tsx`, dan `app/intelligence-knowledge/page.tsx`.

8. **`AGENT_MODEL_OPTIONS`** menggunakan `as const` di array level dan tipe `AgentModelId` diturunkan via `(typeof OPTIONS)[number]["id"]`. Bagus untuk type-safety; pertahankan pola ini.

9. **Tidak ada test framework** terpasang. Tidak ada Jest / Vitest / Playwright.

10. **Tidak ada error boundary** global. Page error akan langsung naik ke Next.js default.

---

## 12. Recommended Improvements

Daftar saran perbaikan yang **belum dilakukan** untuk menjaga behavior. Lakukan satu per satu dengan testing manual.

1. **Pisahkan komponen besar Query Studio** (lihat Known Issues #4).
2. **Tambahkan unit test minimal** untuk helper murni: `cn`, `formatBytesShort`, `formatDurationMs`, `stringHash`, `mockSqlPreRunEstimate`, `slugFromInstruction`, `buildAgentPipelineSuccessOutput`. Ini paling rendah risiko.
3. **Konsolidasi fixture** — pindahkan `SAMPLE_TABLES` (di `app/page.tsx`), `SAMPLE_AUDIT_LOGS` (di `app/audit-logs/page.tsx`), dll. ke `src/lib/<feature>-fixtures.ts` agar konsisten dengan pola `pipeline-fixtures.ts`.
4. **Error boundary** di sub-route `app/error.tsx` untuk menampilkan UI error yang on-brand.
5. **Loading state** Suspense fallback yang lebih bagus (Skeleton sudah ada di `components/ui/skeleton.tsx` tapi belum dipakai di mana-mana).
6. **Resolusi setState-in-effect** secara hati-hati (lihat Known Issues #1).
7. **Persist `tab` & filter** di Pipelines / Dashboard ke URL searchParams untuk shareable links yang lebih lengkap.
8. **Replace mock setTimeout** dengan abstraksi `mockApiCall(ms)` di satu tempat agar mudah diganti dengan API call asli.
9. **Aksesibilitas**: cek kembali `aria-label` untuk semua tombol icon-only (banyak yang sudah ada, tetapi tidak konsisten).
10. **Codegen shadcn**: jika menambah komponen UI baru, pakai `npx shadcn@latest add <component>` agar konsisten dengan `components.json`.

---

## 13. Rules for Future AI/Developer

Ikuti aturan berikut setiap kali menyentuh project ini:

1. **JANGAN mengubah tampilan UI** kecuali ada instruksi eksplisit. Tidak ada redesign.
2. **JANGAN mengubah flow / behavior fitur** yang sudah berjalan. Hindari ubah copy/wording UI kecuali typo nyata yang sudah dikonfirmasi.
3. **JANGAN mengganti library / framework / tool** (Next.js, Tailwind, shadcn, codemirror, dll.) tanpa instruksi eksplisit.
4. **Pertahankan struktur folder yang ada**: route di `app/`, business components di `components/<feature>/`, primitives di `components/ui/`, helpers di `lib/`, hooks di `hooks/`, types di `types/`. Untuk fixture besar, taruh di `src/lib/*-fixtures.ts`.
5. **Path alias `@/*`** wajib digunakan. Jangan pakai relative path panjang.
6. **Selalu gunakan `cn(...)`** dari `src/lib/utils.ts` untuk merge className. Jangan nest template strings className kompleks.
7. **Komponen baru wajib dilengkapi JSDoc singkat** di atas fungsi/komponen, menjelaskan tujuan + props penting + side-effect (mis. setTimeout / sessionStorage).
8. **Komentar function**: jelaskan **tujuan** (mengapa), bukan syntax (apa). Hindari komentar redundan seperti `// Increment counter`.
9. **Komponen `components/ui/*` hampir selalu jangan diubah manual** — diregenerasi shadcn. Jika perlu varian baru, edit hanya bagian yang sudah ada (mis. `button.tsx` punya custom variants brand) atau buat wrapper di `components/<feature>/`.
10. **State management**: tetap pakai `useState` / `useReducer` per page. JANGAN tambah Redux / Zustand / Jotai tanpa diskusi — overhead tidak sebanding dengan project ini.
11. **Mock data**: simulasikan delay dengan `setTimeout`. Saat menambahkan API real, gantikan setTimeout dengan service call, jangan ubah komponen presentasional.
12. **Persistence**: jika butuh persist client-side, ikuti pola `sessionStorage` (key `nq-<feature>-<id>`) yang sudah dipakai di `pipelines/page.tsx`. Bungkus di `try/catch` untuk SSR + storage disabled.
13. **Naming**:
    - Komponen: `PascalCase`
    - Hook: `camelCase` diawali `use`
    - File komponen: `kebab-case.tsx`
    - Konstanta atas: `UPPER_SNAKE`
    - Tipe / interface: `PascalCase` (gunakan `type` untuk shape sederhana, `interface` untuk shape yang ingin di-extend)
14. **TypeScript**: jaga `strict` aktif. Hindari `any`. Untuk DOM event speech, lihat tipe `WebSpeechRec` di `query-studio/page.tsx` sebagai contoh pattern fallback.
15. **Lint**:
    - Jalankan `npm run lint` sebelum commit.
    - Baseline saat ini: 2 errors `react-hooks/set-state-in-effect` (intentional) + 2 warnings (intentional). **Jangan menambah error/warning baru.**
16. **Update file ini** (`AI_PROJECT_INSIGHTS.md`) setiap kali:
    - Menambahkan halaman / fitur baru → update bagian 4.
    - Menambahkan komponen besar atau hook baru → update bagian 5/6.
    - Mengubah struktur folder → update bagian 3.
    - Mengintegrasikan API real → update bagian 9 dan 7.
    - Menambah library di `package.json` → update bagian 2.
17. **Pastikan setiap perubahan tidak merusak:**
    - Build Next.js (`npm run build`).
    - Type check (`npx tsc --noEmit`).
    - Lint (`npm run lint`) — jangan tambah error baru.
    - Smoke test manual: buka tiap halaman utama, switch theme dark/light, switch tab Pipelines, jalankan run NL/SQL di Query Studio.

---

_Last reviewed: code review komprehensif menambahkan komentar function, membersihkan unused import (`CardFooter` di `pipelines/page.tsx`, `cn` di `tables/[id]/page.tsx`), dan menulis dokumen ini. Tidak ada perubahan behavior atau tampilan._
