# Rantai Lake — Design context & bootstrap guide

Dokumen ini menjelaskan **library UI**, **struktur folder**, **token desain**, dan **langkah membuat proyek baru** agar konsisten dengan aplikasi Rantai Lake.

---

## 1. Ringkasan stack

| Lapisan | Pilihan di Rantai Lake |
|--------|-------------------|
| Framework | **Next.js** 16 (App Router) |
| UI runtime | **React** 19 |
| Styling | **Tailwind CSS** v4 (`@import "tailwindcss"`) |
| Animasi util | **tw-animate-css** |
| Tema komponen | **shadcn/ui** preset **base-nova** + `cssVariables: true` |
| Primitif interaksi | **@base-ui/react** (Button, Dialog→Sheet, Tabs, Select, …) |
| Varian gaya | **class-variance-authority** (cva) |
| Merge class | **clsx** + **tailwind-merge** → `cn()` |
| Ikon | **lucide-react** |
| Dark mode | **next-themes** |

File konfigurasi shadcn di repo: [`components.json`](../components.json) (path relatif dari root repo, satu tingkat di atas `design-kit/`).

---

## 2. Struktur folder aplikasi (referensi)

```
rantai-lake/
├── design-kit/              ← kit ini (token, docs, aset referensi)
├── public/                  ← aset statis (rantai-logo.png, dll.)
├── src/
│   ├── app/                 ← route, layout.tsx, page.tsx, globals.css
│   ├── components/
│   │   ├── ui/              ← primitif shadcn / Base UI
│   │   ├── shared/          ← theme-provider, dsb.
│   │   ├── app-sidebar.tsx
│   │   └── app-navbar.tsx
│   ├── hooks/
│   ├── lib/
│   │   └── utils.ts         ← cn()
│   └── types/
├── components.json
├── package.json
└── postcss.config.mjs
```

**Alias TypeScript** (dari `components.json`): `@/components`, `@/components/ui`, `@/lib`, `@/hooks`.

---

## 3. Tipografi

| Peran | Font | Cara load |
|-------|------|-----------|
| UI / body / heading default | **Inter** | `next/font/google` → CSS variable `--font-inter` |
| Brand / sidebar / beberapa judul | **Montserrat** | `next/font/google` → `--font-montserrat` |
| Kode / SQL | **Menlo** stack | `--font-mono` di `@theme` |

Di `layout.tsx`, terapkan variabel ke `<body>`:

```tsx
<body className={`${inter.variable} ${montserrat.variable} font-sans antialiased`}>
```

Skala heading `h1`–`h4`, tabel, dan utilitas `.text-lead`, `.text-muted`, dll. ada di `src/app/globals.css` bagian `@layer base`.

---

## 4. Warna & token

- **Primary (light):** `#ff5001` — aksi utama, fokus ring, teks sidebar brand.
- **Accent:** `#ffdece` — latar strip ringkasan (`bg-accent`).
- **Accent foreground:** `#551d00` — teks di atas accent.
- **Sidebar:** background `#fafafa`, border `#e5e7eb`.
- **Dark mode:** primary lebih terang (`#ff9c6d`), sidebar/token disesuaikan di `.dark`.

Salinan siap tempel untuk proyek baru: [`tokens/rantai-lake-theme.css`](./tokens/rantai-lake-theme.css).

---

## 5. Bootstrap proyek baru (checklist)

1. **Buat app Next.js** (App Router, TypeScript, Tailwind sesuai versi di [`package.versions.reference.json`](./package.versions.reference.json)).
2. **Pasang dependency** inti: `@base-ui/react`, `class-variance-authority`, `clsx`, `tailwind-merge`, `lucide-react`, `next-themes`, `tw-animate-css`, `shadcn` (CLI).
3. **Inisialisasi shadcn** dengan gaya **base-nova**, `cssVariables: true`, ikon **lucide** (samakan pola [`components.json`](../components.json)).
4. **Susun `globals.css`:** impor `tailwindcss`, `tw-animate-css`, `shadcn/tailwind.css`, lalu sisipkan isi [`tokens/rantai-lake-theme.css`](./tokens/rantai-lake-theme.css) dan blok `@layer base` dari repo utama jika ingin tipografi identik.
5. **Font:** duplikasi pola `Inter` + `Montserrat` dari `src/app/layout.tsx`.
6. **Salin `src/lib/utils.ts`** (`cn`).
7. **Tambah komponen UI** yang dibutuhkan via `npx shadcn@latest add …`.
8. **Aset:** salin logo dari [`assets/`](./assets/) atau `public/rantai-logo.png` ke `public/` proyek baru.
9. **Opsional:** salin [`AI_CONTEXT.md`](./AI_CONTEXT.md) ke `.cursor/rules` atau lampirkan ke agen saat generate UI.

---

## 6. Perbedaan `design-kit` vs `src/components`

- **`design-kit/`** — dokumentasi, token CSS fragment, aset referensi, **tidak** di-import oleh build Next (kecuali Anda sengaja mengubahnya).
- **`src/components/`** — sumber komponen **aktual** yang dipakai aplikasi.

Untuk proyek baru, umumnya Anda **menyalin token + pola**, lalu **menginstal ulang** komponen shadcn; jangan mengandalkan copy-paste seluruh `node_modules`.

---

## 7. Figma & brand

Desain UI dirujuk dari file Figma Rantai Lake (sidebar, navbar, Query Studio, dll.). Saat token di Figma berubah, perbarui **dua tempat**: `src/app/globals.css` dan `design-kit/tokens/rantai-lake-theme.css` agar kit tetap selaras.

---

## 8. File terkait di repo

| File | Fungsi |
|------|--------|
| `src/app/globals.css` | Sumber utama tema + typography |
| `src/app/layout.tsx` | Font + ThemeProvider + shell layout |
| `src/components/ui/button.tsx` | Pola `nativeButton` + `render` |
| `design-kit/AI_CONTEXT.md` | Konteks ringkas untuk AI |
| `design-kit/components/patterns.md` | Snippet pola UI |

---

## 9. Lisensi / penggunaan aset

Logo resmi harus mengikuti pedoman brand perusahaan. File `assets/rantai-mark.svg` di kit ini adalah **placeholder vektor** untuk pengembangan lokal; ganti dengan aset resmi dari tim desain bila tersedia.
