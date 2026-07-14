# Pola komponen Rantai Lake

Primitif UI hidup di `src/components/ui/` (shadcn + Base UI). Folder ini **bukan** salinan komponen; gunakan CLI shadcn di proyek baru untuk menginstal ulang, lalu terapkan pola di bawah.

## `cn()` utility

```ts
import { cn } from "@/lib/utils"

cn("base", condition && "conditional", className)
```

## Button + Next.js Link

Base UI memperingatkan jika `render` bukan `<button>`. Wrapper `Button` di repo ini mengatur `nativeButton={false}` ketika `render` diset.

```tsx
import Link from "next/link"
import { Button } from "@/components/ui/button"

<Button
  size="default"
  className="h-10 gap-2 rounded-md bg-primary px-4 text-primary-foreground"
  render={<Link href="/target" />}
>
  <Plus className="size-4" />
  Label
</Button>
```

## Header halaman (admin / data)

```tsx
<header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
  <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
    Title
  </h1>
  <div className="flex items-center gap-3">{/* actions */}</div>
</header>
```

## Kartu ringkasan (peach strip)

```tsx
<section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
  {/* Card items */}
</section>
```

## Tab (secondary background)

```tsx
<TabsList className="h-auto min-h-9 w-max max-w-full gap-1 rounded-md bg-secondary p-1 sm:inline-flex sm:w-auto sm:flex-nowrap">
  <TabsTrigger
    value="a"
    className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
  >
    …
  </TabsTrigger>
</TabsList>
```

## Sidebar + layout

- `SidebarProvider` + `AppSidebar` + `SidebarInset` + `AppNavbar`.
- Font label: `font-[family-name:var(--font-montserrat)]` pada item nav sesuai Figma.

## Status badge (pipeline-style)

Hijau sukses: `bg-[#ecfdf2] text-[#008a2e]`. Merah gagal: `bg-destructive/10 text-destructive`.

## Primitif yang dipasang di repo ini

`badge`, `breadcrumb`, `button`, `card`, `input`, `label`, `pagination`, `select`, `separator`, `sheet`, `sidebar`, `skeleton`, `switch`, `table`, `tabs`, `textarea`, `tooltip`

Instal tambahan dengan: `npx shadcn@latest add <name>` (sesuaikan registry / style **base-nova**).
