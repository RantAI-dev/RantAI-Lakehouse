# Rantai Lake

Next.js 16 project dengan App Router, TypeScript, Tailwind CSS, dan shadcn/ui.

Styling & tema mengikuti **Rantai Design System** (`design-system/`): token warna OKLCH biru/navy, font Geist, dan mode gelap (dark) sebagai default.

## Tech Stack

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

## Menjalankan Server Lokal

```bash
# Install dependencies (jika belum)
npm install

# Development
npm run dev
```

Buka [http://localhost:3000](http://localhost:3000).

```bash
# Build production
npm run build

# Jalankan production
npm start
```

## Menambah Komponen shadcn Pertama Kali

1. **Lihat daftar komponen**
   - Buka [shadcn/ui Components](https://ui.shadcn.com/docs/components)

2. **Tambahkan komponen via CLI**
   ```bash
   npx shadcn@latest add <nama-komponen>
   ```
   Contoh:
   ```bash
   npx shadcn@latest add card
   npx shadcn@latest add dialog
   npx shadcn@latest add input
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
