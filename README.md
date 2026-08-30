# Rantai Lake

Next.js 16 project dengan App Router, TypeScript, Tailwind CSS, dan shadcn/ui.

Styling & tema mengikuti **Rantai Design System** (`design-system/`): token warna OKLCH biru/navy, font Geist, dan mode gelap (dark) sebagai default.

## Tech Stack

- **Bun** (runtime, package manager, test runner)
- **Next.js** (App Router)
- **TypeScript**
- **Tailwind CSS**
- **shadcn/ui** (komponen UI)
- **lucide-react** (ikon)
- **next-themes** (dark mode)
- **clsx** (utility class names)

## Struktur Folder `src/`

```
src/
├── app/                 # App Router (layout, page, routes)
├── components/
│   ├── ui/              # Komponen shadcn/ui
│   └── shared/          # Komponen shared (ThemeProvider, dll)
├── lib/                 # Utilitas (utils, config)
├── hooks/               # Custom React hooks
└── types/               # TypeScript types/interfaces
```

## Runtime: Bun

Project ini berjalan di atas **Bun** (bukan Node.js). Butuh Bun `>= 1.3.0`
([install](https://bun.sh/docs/installation)):

```bash
curl -fsSL https://bun.sh/install | bash
```

Semua script (`dev`, `build`, `start`, `lint`, `typecheck`) memakai flag
`--bun` supaya Next.js dieksekusi oleh runtime Bun, bukan Node. Lockfile-nya
`bun.lock` — jangan pakai `npm install`, karena akan membuat `package-lock.json`
yang tidak sinkron.

> Catatan: di `ps`, proses server tampil sebagai `node` karena Bun sengaja
> menyamar demi kompatibilitas tooling. Untuk memastikan, cek
> `readlink /proc/<pid>/exe` — hasilnya menunjuk ke binary `bun`.

## Menjalankan Server Lokal

```bash
# Install dependencies (jika belum)
bun install

# Development
bun run dev
```

Buka [http://localhost:3000](http://localhost:3000).

```bash
# Build production
bun run build

# Jalankan production
bun start
```

## Test, Lint, Typecheck

```bash
bun run test       # bun test (unit test di src/lib)
bun run lint       # eslint
bun run typecheck  # tsc --noEmit
```

## Menambah Komponen shadcn Pertama Kali

1. **Lihat daftar komponen**
   - Buka [shadcn/ui Components](https://ui.shadcn.com/docs/components)

2. **Tambahkan komponen via CLI**
   ```bash
   bunx shadcn@latest add <nama-komponen>
   ```
   Contoh:
   ```bash
   bunx shadcn@latest add card
   bunx shadcn@latest add dialog
   bunx shadcn@latest add input
   ```

3. **Lokasi file**
   - Komponen akan ditambahkan di `src/components/ui/` (sesuai `components.json`).

4. **Cara pakai**
   ```tsx
   import { Button } from "@/components/ui/button"
   import { Card, CardContent, CardHeader } from "@/components/ui/card"

   export default function Page() {
     return (
       <Card>
         <CardHeader>Judul</CardHeader>
         <CardContent>
           <Button>Klik</Button>
         </CardContent>
       </Card>
     )
   }
   ```

## Path Alias

- `@/*` → `./src/*` (sudah dikonfigurasi di `tsconfig.json`)

## Dark Mode

Project memakai `next-themes` lewat `ThemeProvider` dari design system (`@rantai/design-system/components/theme-provider`) di `src/app/layout.tsx`. Sesuai design system, mode **dark dipaksa (forced)**. Untuk mengaktifkan toggle light/dark, hapus prop `forcedTheme` di komponen tersebut.
