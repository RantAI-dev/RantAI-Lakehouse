"use client";

import * as React from "react";
import { GripVertical, Pencil, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LayoutMap, TileBox } from "@/services/clients/bi-store";

export type GridItem = {
  id: string;
  title: string;
  subtitle?: string;
  badge?: React.ReactNode;
  body: React.ReactNode;
  onEdit?: () => void;
  onRemove?: () => void;
};

const COLS = 12;
const MARGIN = 12;
const ROW_H = 44;
const DEFAULT: TileBox = { x: 0, y: 0, w: 6, h: 6 };

/** Pastikan tiap item punya kotak; yang belum → tempatkan 2-per-baris di bawah. */
function resolve(items: GridItem[], layout: LayoutMap): LayoutMap {
  const out: LayoutMap = {};
  const ids = new Set(items.map((i) => i.id));
  for (const [id, box] of Object.entries(layout)) if (ids.has(id)) out[id] = box;
  let maxY = 0;
  for (const b of Object.values(out)) maxY = Math.max(maxY, b.y + b.h);
  let col = 0;
  for (const it of items) {
    if (out[it.id]) continue;
    out[it.id] = { x: col, y: maxY, w: DEFAULT.w, h: DEFAULT.h };
    col += DEFAULT.w;
    if (col >= COLS) { col = 0; maxY += DEFAULT.h; }
  }
  return out;
}

type Drag = { id: string; mode: "move" | "resize"; px: number; py: number; box: TileBox };

/**
 * Kanvas dashboard 12-kolom dengan DRAG (pindah) & RESIZE (handle sudut) ala
 * Tableau. Kustom (pointer events) — aman untuk React 19. Di mode `editable`,
 * tile bisa digeser & diubah ukurannya; perubahan dikirim via onLayoutChange.
 */
export function DashboardGrid({
  items, layout, editable, onLayoutChange,
}: {
  items: GridItem[];
  layout: LayoutMap;
  editable: boolean;
  onLayoutChange: (next: LayoutMap) => void;
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(0);
  const [drag, setDrag] = React.useState<Drag | null>(null);
  const [preview, setPreview] = React.useState<LayoutMap | null>(null);

  // Ukur lebar SEKETIKA (useLayoutEffect) agar tile tak collapse di render awal,
  // plus ResizeObserver untuk perubahan lebar.
  React.useLayoutEffect(() => {
    if (ref.current) setWidth(ref.current.getBoundingClientRect().width);
  }, []);
  React.useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((e) => setWidth(e[0].contentRect.width));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);

  const base = React.useMemo(() => resolve(items, layout), [items, layout]);
  const view = preview ?? base;

  const colW = width > 0 ? (width - MARGIN * (COLS + 1)) / COLS : 0;
  const unitX = colW + MARGIN;
  const unitY = ROW_H + MARGIN;
  const boxStyle = (b: TileBox): React.CSSProperties => ({
    position: "absolute",
    left: MARGIN + b.x * unitX,
    top: MARGIN + b.y * unitY,
    width: b.w * colW + (b.w - 1) * MARGIN,
    height: b.h * ROW_H + (b.h - 1) * MARGIN,
  });

  const maxY = Math.max(0, ...Object.values(view).map((b) => b.y + b.h));
  const canvasH = MARGIN + maxY * unitY + (editable ? unitY : 0);

  const startDrag = (e: React.PointerEvent, id: string, mode: "move" | "resize") => {
    if (!editable) return;
    e.preventDefault();
    setDrag({ id, mode, px: e.clientX, py: e.clientY, box: base[id] });
    setPreview(base);
  };

  React.useEffect(() => {
    if (!drag) return;
    const move = (e: PointerEvent) => {
      const dCols = Math.round((e.clientX - drag.px) / unitX);
      const dRows = Math.round((e.clientY - drag.py) / unitY);
      const b = drag.box;
      let nb: TileBox;
      if (drag.mode === "move") {
        nb = {
          x: Math.min(Math.max(0, b.x + dCols), COLS - b.w),
          y: Math.max(0, b.y + dRows),
          w: b.w, h: b.h,
        };
      } else {
        nb = {
          x: b.x, y: b.y,
          w: Math.min(Math.max(2, b.w + dCols), COLS - b.x),
          h: Math.max(3, b.h + dRows),
        };
      }
      setPreview({ ...base, [drag.id]: nb });
    };
    const up = () => {
      setPreview((p) => {
        if (p) onLayoutChange(p);
        return null;
      });
      setDrag(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [drag, base, unitX, unitY, onLayoutChange]);

  return (
    <div ref={ref} className="relative w-full" style={{ height: canvasH, minHeight: 200 }}>
      {editable ? (
        <div
          className="pointer-events-none absolute inset-0 rounded-lg"
          style={{
            backgroundImage: `linear-gradient(90deg, var(--border) 1px, transparent 1px)`,
            backgroundSize: `${unitX}px 100%`,
            backgroundPosition: `${MARGIN}px 0`,
            opacity: 0.25,
          }}
        />
      ) : null}
      {items.map((it) => {
        const b = view[it.id];
        if (!b) return null;
        const dragging = drag?.id === it.id;
        return (
          <div key={it.id} style={boxStyle(b)} className={cn("transition-[left,top,width,height]", dragging && "z-20 transition-none")}>
            <div className={cn("flex h-full flex-col overflow-hidden rounded-xl border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]", dragging && "ring-2 ring-primary/40")}>
              {/* Header / drag handle */}
              <div className={cn("flex items-center gap-2 border-b border-border px-3 py-1.5", editable && "cursor-move select-none bg-muted/40")}
                   style={editable ? { touchAction: "none" } : undefined}
                   onPointerDown={editable ? (e) => startDrag(e, it.id, "move") : undefined}>
                {editable ? <GripVertical className="size-4 shrink-0 text-muted-foreground" /> : null}
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold leading-tight">{it.title}</p>
                  {it.subtitle ? <p className="truncate text-[11px] text-muted-foreground">{it.subtitle}</p> : null}
                </div>
                {it.badge}
                {editable ? (
                  <div className="flex items-center gap-0.5" onPointerDown={(e) => e.stopPropagation()}>
                    {it.onEdit ? (
                      <button type="button" onClick={it.onEdit} aria-label="Ubah" className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground">
                        <Pencil className="size-3.5" />
                      </button>
                    ) : null}
                    {it.onRemove ? (
                      <button type="button" onClick={it.onRemove} aria-label="Hapus" className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-destructive">
                        <Trash2 className="size-3.5" />
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
              {/* Body */}
              <div className="min-h-0 flex-1 p-2">{it.body}</div>
            </div>
            {/* Resize handle (pojok kanan-bawah, jelas) */}
            {editable ? (
              <div
                onPointerDown={(e) => startDrag(e, it.id, "resize")}
                className="absolute -bottom-1 -right-1 z-10 grid size-6 cursor-se-resize place-items-center rounded-md border border-border bg-card text-muted-foreground shadow-sm hover:bg-muted hover:text-foreground"
                style={{ touchAction: "none" }}
                title="Tarik untuk ubah ukuran"
              >
                <svg viewBox="0 0 10 10" className="size-3"><path d="M9 3v6H3" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
