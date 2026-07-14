# Setup Guide — RantAI Design System

Panduan plug-and-play untuk repo Next.js + Tailwind v4 baru.

---

## Prasyarat

- Next.js 15+ (App Router)
- Tailwind CSS v4
- TypeScript

---

## Langkah 1 — Copy folder

```bash
cp -r design-system/ /path/to/repo-baru/design-system/
```

---

## Langkah 2 — Install dependencies

Jalankan dari root repo target:

```bash
npm install \
  tailwindcss@^4 @tailwindcss/postcss @tailwindcss/typography tw-animate-css \
  clsx tailwind-merge class-variance-authority \
  motion next-themes lucide-react sonner \
  radix-ui cmdk vaul embla-carousel-react react-day-picker date-fns \
  react-resizable-panels @radix-ui/react-icons

npm install -D shadcn@^4
npx shadcn@latest init
```

Saat `shadcn init`, gunakan setting yang sama dengan `setup/components.json`:
- Style: **radix-nova**
- Base color: **neutral**
- CSS variables: **yes**

Atau copy langsung:

```bash
cp design-system/setup/components.json ./components.json
```

> **Catatan:** `shadcn/tailwind.css` diperlukan oleh `globals.css`. Package `shadcn` harus ter-install (`npm install -D shadcn`).

---

## Langkah 3 — PostCSS

```bash
cp design-system/setup/postcss.config.example.mjs ./postcss.config.mjs
```

---

## Langkah 4 — TypeScript paths

Merge ke `tsconfig.json`:

```json
{
  "compilerOptions": {
    "paths": {
      "@/*": ["./src/*"],
      "@rantai/design-system": ["./design-system"],
      "@rantai/design-system/*": ["./design-system/*"]
    }
  }
}
```

Referensi lengkap: `setup/tsconfig.paths.example.json`

---

## Langkah 5 — Wire globals.css

Buat `src/app/globals.css` yang hanya import design system:

```css
@import "../../design-system/globals.css";
```

Atau copy `design-system/globals.css` langsung ke `src/app/globals.css`.

Pastikan `src/app/layout.tsx` meng-import `./globals.css`.

---

## Langkah 6 — Fonts + ThemeProvider

```tsx
// src/app/layout.tsx
import "./globals.css";

import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { TooltipProvider } from "@rantai/design-system/ui/tooltip";
import { Toaster } from "@rantai/design-system/ui/sonner";
import { cn } from "@rantai/design-system/lib/utils";

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={cn("antialiased", geist.variable, geistMono.variable, "font-sans")}
    >
      <head>
        <meta name="theme-color" content="#050A30" />
      </head>
      <body>
        <ThemeProvider>
          <TooltipProvider>{children}</TooltipProvider>
        </ThemeProvider>
        <Toaster />
      </body>
    </html>
  );
}
```

---

## Langkah 7 — Verifikasi

```bash
npm run dev
```

Test import komponen:

```tsx
import { Button } from "@rantai/design-system/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@rantai/design-system/ui/card";
import { FadeUp } from "@rantai/design-system/components/motion";

export default function Page() {
  return (
    <FadeUp className="p-8 space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>RantAI Design System</CardTitle>
        </CardHeader>
        <CardContent>
          <Button>Primary</Button>
        </CardContent>
      </Card>
    </FadeUp>
  );
}
```

---

## Import paths

| Resource | Import |
|----------|--------|
| UI components | `@rantai/design-system/ui/button` |
| All UI (barrel) | `@rantai/design-system/ui` |
| Utils | `@rantai/design-system/lib/utils` |
| Motion | `@rantai/design-system/components/motion` |
| Theme | `@rantai/design-system/components/theme-provider` |
| Fonts | `@rantai/design-system/fonts/fonts` |
| Tag colors | `@rantai/design-system/lib/tag-colors` |
| TipTap editor CSS | `@import "../../design-system/tiptap.css"` |

---

## Struktur folder

```
design-system/
├── package.json              ← peerDependencies manifest
├── globals.css               ← Entry point CSS (imports tokens/*)
├── tiptap.css                ← Editor tokens (opsional)
├── tokens/
│   ├── colors.css            ← Raw OKLCH values (:root + .dark)
│   ├── spacing.css           ← Radius + sidebar widths
│   ├── theme.css             ← Tailwind @theme mappings
│   ├── typography.css        ← Type scale reference (docs)
│   └── animations.css        ← Keyframes + transition tokens
├── fonts/fonts.ts
├── lib/                      ← utils, motion-variants, tag-colors
├── hooks/use-mobile.ts
├── components/
│   ├── motion.tsx
│   └── theme-provider.tsx
├── ui/                       ← 43 shadcn components (radix-nova)
│   └── index.ts              ← barrel export
└── setup/
    ├── components.json
    ├── postcss.config.example.mjs
    ├── tsconfig.paths.example.json
    └── install.md
```

---

## Troubleshooting

### `Cannot find module 'shadcn/tailwind.css'`
Install shadcn CLI package: `npm install -D shadcn` lalu jalankan `npx shadcn init`.

### `@rantai/design-system/*` tidak resolve
Pastikan `tsconfig.json` paths sudah di-merge dan restart TS server.

### Font tidak muncul
Pastikan `geist.variable` dan `geistMono.variable` ada di className `<html>`.

### Dark mode tidak aktif
`ThemeProvider` default memaksa dark mode. Tambahkan class `dark` di `<html>` atau hapus `forcedTheme` di theme-provider.

---

## Opsional — Toggle light/dark

Edit `design-system/components/theme-provider.tsx`:

```tsx
<NextThemesProvider
  attribute="class"
  defaultTheme="dark"
  enableSystem
  disableTransitionOnChange
>
```

Hapus prop `forcedTheme="dark"`.
