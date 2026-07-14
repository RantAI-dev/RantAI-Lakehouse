# AI / Cursor context — Rantai Lake UI

Salin isi file ini ke rules proyek baru atau lampirkan saat membangun UI agar output seragam dengan Rantai Lake.

## Stack

- **Framework:** Next.js (App Router), React 19.
- **Styling:** Tailwind CSS v4, `tw-animate-css`, impor `shadcn/tailwind.css`.
- **Komponen UI:** shadcn (preset **base-nova**, `cssVariables: true`), **primitives dari @base-ui/react** (Button, Dialog/Sheet, Tabs, Select, dll.).
- **Util class merge:** `clsx` + `tailwind-merge` → helper `cn()` di `@/lib/utils`.
- **Varian komponen:** `class-variance-authority` (cva).
- **Ikon:** `lucide-react`.
- **Tema:** `next-themes` (ThemeProvider di root layout).

## Tipografi

- **Inter** (`--font-inter`): body, tabel, heading default di `globals.css` `@layer base`.
- **Montserrat** (`--font-montserrat`): label brand / sidebar / beberapa judul halaman (`font-[family-name:var(--font-montserrat)]`).
- **Mono:** Menlo stack untuk SQL / kode.

## Warna merek

- **Primary (light):** `#ff5001` — teks/aksi utama, ring fokus.
- **Accent / peach:** `#ffdece` — area ringkasan (kartu di `bg-accent`), tab group.
- **Accent foreground:** `#551d00` — teks di atas peach.
- **Sidebar:** `#fafafa`, border `#e5e7eb`, teks/aksi orange selaras primary.
- Skala `primary-50` … `primary-950` ada di `@theme inline` (lihat `tokens/rantai-lake-theme.css`).

## Struktur sumber (repo Rantai Lake)

```
src/app/           → halaman & globals.css
src/components/ui/ → primitif shadcn/Base UI
src/components/    → app-sidebar, app-navbar, shared/
src/lib/utils.ts   → cn()
public/            → rantai-logo.png (brand; salin dari desain jika belum ada)
```

## Pola wajib

- **Button + Next Link:** pakai `render={<Link href="..." />}` pada `Button`; wrapper `Button` set `nativeButton={false}` otomatis jika `render` ada (lihat `src/components/ui/button.tsx`).
- **Spacing halaman admin:** header `border-b border-border`, judul `text-[24px] font-semibold text-primary`, kartu ringkasan di `bg-accent p-2`.
- Jangan mengganti token semantik (`primary`, `accent`, `sidebar-*`) tanpa sinkron dengan Figma / design-kit.

## File rujukan di repo

- `components.json` — alias `@/components`, style base-nova.
- `design-kit/tokens/rantai-lake-theme.css` — salinan token untuk proyek baru.
- `design-kit/DESIGN_CONTEXT.md` — panduan bootstrap lengkap.
