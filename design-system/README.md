# RantAI Design System

Design system plug-and-play yang diekstrak dari `rantai-page`.

Berisi tokens, 43 shadcn/ui components (radix-nova), motion primitives, fonts, dan setup files — siap copy ke repo Next.js lain.

---

## Quick Start

```bash
# 1. Copy folder ke repo target
cp -r design-system/ ../repo-baru/

# 2. Install deps (lihat setup/install.md untuk detail)
npm install tailwindcss@^4 @tailwindcss/postcss @tailwindcss/typography tw-animate-css \
  clsx tailwind-merge class-variance-authority motion next-themes lucide-react sonner radix-ui
npm install -D shadcn@^4 && npx shadcn init

# 3. Setup PostCSS + tsconfig paths (lihat setup/)
# 4. src/app/globals.css
@import "../../design-system/globals.css";

# 5. Wire layout.tsx (fonts + ThemeProvider) — lihat setup/install.md
```

Panduan lengkap: **[setup/install.md](./setup/install.md)**

---

## Apa yang included

| Layer | Isi |
|-------|-----|
| **Tokens** | OKLCH colors (light + dark), radius, brand hex, animations |
| **CSS** | `globals.css` modular — imports `tokens/*.css` |
| **Fonts** | Geist + Geist Mono via `next/font` |
| **UI** | 43 shadcn components (Button, Card, Dialog, Sidebar, …) |
| **Motion** | FadeUp, StaggerContainer, ScaleIn, SlideIn |
| **Theme** | ThemeProvider (dark default) |
| **Utils** | `cn()`, motion variants, tag color presets |
| **Setup** | `package.json` peerDeps, tsconfig/postcss examples |

---

## Import examples

```tsx
import { Button } from "@rantai/design-system/ui/button";
import { Card, CardContent } from "@rantai/design-system/ui/card";
import { cn } from "@rantai/design-system/lib/utils";
import { FadeUp } from "@rantai/design-system/components/motion";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { gradientForColor } from "@rantai/design-system/lib/tag-colors";
```

Barrel export semua UI:

```tsx
import { Button, Card, Input, Dialog } from "@rantai/design-system/ui";
```

---

## Brand identity

| Token | Value | Role |
|-------|-------|------|
| `--brand-1` | `#5cb6f9` | Sky blue highlight |
| `--brand-2` | `#050a30` | Deep navy |
| Primary (dark) | `oklch(0.443 0.11 240.79)` | Buttons, links |
| Fonts | Geist + Geist Mono | Sans + mono labels |
| Radius base | `0.45rem` | Slightly rounded |
| Mode | Dark (default) | Blue-tinted OKLCH palette |

---

## Token architecture

```
globals.css
  ├── tokens/colors.css    ← :root + .dark raw values
  ├── tokens/spacing.css   ← --radius, sidebar widths
  ├── tokens/theme.css     ← @theme inline → Tailwind utilities
  └── tokens/animations.css← keyframes
```

Single source of truth — tidak ada duplikasi antara token files dan globals.

---

## UI components (43)

`alert` · `alert-dialog` · `aspect-ratio` · `avatar` · `badge` · `bento-grid` · `breadcrumb` · `button` · `button-group` · `calendar` · `card` · `carousel` · `checkbox` · `combobox` · `command` · `dialog` · `drawer` · `dropdown-menu` · `empty` · `empty-state` · `field` · `hover-card` · `input` · `input-group` · `item` · `label` · `navigation-menu` · `particles` · `popover` · `resizable` · `scroll-area` · `select` · `separator` · `sheet` · `sidebar` · `skeleton` · `slider` · `sonner` · `switch` · `table` · `tabs` · `textarea` · `tooltip`

Semua import internal sudah relative (`../lib/utils`, `./button`) — **tidak bergantung pada path alias repo asal**.

---

## peerDependencies

Lihat `package.json` untuk daftar lengkap. Core:

- `next` >= 15
- `tailwindcss` ^4
- `tw-animate-css`
- `shadcn` (dev — untuk `shadcn/tailwind.css`)
- `motion`, `next-themes`, `clsx`, `tailwind-merge`
- `radix-ui`, `class-variance-authority`, `lucide-react`

---

## Stack

| Layer | Teknologi |
|-------|-----------|
| CSS | Tailwind CSS v4 (CSS-first) |
| UI | shadcn/ui radix-nova |
| Fonts | Geist + Geist Mono |
| Animation | motion/react + tw-animate-css |
| Theming | next-themes (forced dark) |
