"use client"

import * as React from "react"
import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { cn } from "@/lib/utils"
import { compareValues, type SortableValue } from "@/lib/table-sort"

export type ColumnDef<T> = {
  key: string
  header: string
  className?: string
  render: (row: T) => React.ReactNode
  /**
   * Nilai yang dipakai untuk mengurutkan kolom ini. Kolom tanpa `sortValue`
   * tidak bisa diurutkan — `render` mengembalikan ReactNode yang tidak bisa
   * dibandingkan secara andal, jadi urutan harus berasal dari data mentah.
   */
  sortValue?: (row: T) => SortableValue
}

type SortState = { key: string; direction: "asc" | "desc" }

/**
 * Shared list table. Handles header/body layout, row click affordance, and
 * a built-in empty message so modules never re-invent table plumbing.
 *
 * Sorting dan paginasi bersifat OPT-IN: keduanya mati kecuali kolom
 * menyediakan `sortValue` atau `pageSize` diberikan. Ini disengaja — komponen
 * ini dipakai 30+ halaman, dan mengaktifkan keduanya secara default akan
 * mengubah tampilan setiap halaman sekaligus.
 */
export function DataTable<T>({
  columns,
  rows,
  rowKey,
  onRowClick,
  emptyMessage = "No results match the current filters.",
  emptyState,
  pageSize,
  className,
}: {
  columns: ColumnDef<T>[]
  rows: T[]
  rowKey: (row: T) => string
  onRowClick?: (row: T) => void
  emptyMessage?: string
  /**
   * Tampilan kaya untuk keadaan kosong (mis. `EmptyState` dengan ajakan
   * bertindak). Bila diisi, `emptyMessage` diabaikan.
   */
  emptyState?: React.ReactNode
  /** Jumlah baris per halaman. Tidak diisi berarti tanpa paginasi. */
  pageSize?: number
  className?: string
}) {
  const [sort, setSort] = React.useState<SortState | null>(null)
  const [page, setPage] = React.useState(0)

  const sortedRows = React.useMemo(() => {
    if (!sort) return rows
    const column = columns.find((c) => c.key === sort.key)
    if (!column?.sortValue) return rows
    const getValue = column.sortValue
    // Salin dulu: `Array.prototype.sort` memutasi, dan `rows` milik pemanggil.
    return [...rows].sort((a, b) => {
      const result = compareValues(getValue(a), getValue(b))
      return sort.direction === "asc" ? result : -result
    })
  }, [rows, sort, columns])

  const pageCount = pageSize ? Math.ceil(sortedRows.length / pageSize) : 1
  // Jepit halaman saat filter menyusutkan data agar tidak tampil halaman kosong.
  const safePage = Math.min(page, Math.max(0, pageCount - 1))
  const visibleRows = React.useMemo(() => {
    if (!pageSize) return sortedRows
    const start = safePage * pageSize
    return sortedRows.slice(start, start + pageSize)
  }, [sortedRows, pageSize, safePage])

  // Kembali ke halaman pertama saat data atau urutan berubah.
  React.useEffect(() => {
    setPage(0)
  }, [rows, sort])

  const toggleSort = (key: string) => {
    setSort((prev) => {
      if (prev?.key !== key) return { key, direction: "asc" }
      if (prev.direction === "asc") return { key, direction: "desc" }
      return null // klik ketiga mengembalikan urutan asli
    })
  }

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      <div className="overflow-hidden rounded-lg border border-border bg-card">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            {columns.map((col) => {
              const sortable = Boolean(col.sortValue)
              const active = sort?.key === col.key
              return (
                <TableHead
                  key={col.key}
                  className={cn("text-xs font-medium", col.className)}
                  aria-sort={
                    active
                      ? sort?.direction === "asc"
                        ? "ascending"
                        : "descending"
                      : undefined
                  }
                >
                  {sortable ? (
                    <button
                      type="button"
                      onClick={() => toggleSort(col.key)}
                      className="inline-flex items-center gap-1 rounded-sm hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      {col.header}
                      {active ? (
                        sort?.direction === "asc" ? (
                          <ArrowUp className="size-3" aria-hidden />
                        ) : (
                          <ArrowDown className="size-3" aria-hidden />
                        )
                      ) : (
                        <ChevronsUpDown className="size-3 opacity-40" aria-hidden />
                      )}
                    </button>
                  ) : (
                    col.header
                  )}
                </TableHead>
              )
            })}
          </TableRow>
        </TableHeader>
        <TableBody>
          {visibleRows.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={columns.length}
                className="py-10 text-center text-sm text-muted-foreground"
              >
                {emptyState ?? emptyMessage}
              </TableCell>
            </TableRow>
          ) : (
            visibleRows.map((row) => (
              <TableRow
                key={rowKey(row)}
                className={cn(
                  onRowClick &&
                    "cursor-pointer focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                )}
                // Baris yang bisa diklik diperlakukan sebagai tombol oleh
                // pembaca layar; tanpa `role` ia hanya diumumkan sebagai baris
                // tabel biasa sehingga sifat interaktifnya tidak terdengar.
                role={onRowClick ? "button" : undefined}
                onClick={onRowClick ? () => onRowClick(row) : undefined}
                tabIndex={onRowClick ? 0 : undefined}
                onKeyDown={
                  onRowClick
                    ? (e) => {
                        // Spasi disertakan agar perilakunya sama dengan tombol
                        // asli; `preventDefault` mencegah halaman ikut ter-scroll.
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault()
                          onRowClick(row)
                        }
                      }
                    : undefined
                }
              >
                {columns.map((col) => (
                  <TableCell key={col.key} className={cn("py-2.5", col.className)}>
                    {col.render(row)}
                  </TableCell>
                ))}
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
      </div>

      {pageSize && pageCount > 1 ? (
        <div className="flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground">
          <p aria-live="polite">
            Showing {safePage * pageSize + 1}-
            {Math.min((safePage + 1) * pageSize, sortedRows.length)} of{" "}
            {sortedRows.length}
          </p>
          <div className="flex items-center gap-1">
            <Button
              size="sm"
              variant="outline"
              disabled={safePage === 0}
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              aria-label="Previous page"
            >
              Previous
            </Button>
            <span className="px-2 tabular-nums">
              {safePage + 1} / {pageCount}
            </span>
            <Button
              size="sm"
              variant="outline"
              disabled={safePage >= pageCount - 1}
              onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
              aria-label="Next page"
            >
              Next
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  )
}
