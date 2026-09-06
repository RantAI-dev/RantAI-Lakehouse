/**
 * Logika perbandingan untuk sorting tabel — dipisah dari
 * `@/components/patterns/data-table` supaya bisa diuji tanpa React/DOM.
 */

export type SortableValue = string | number | boolean | null | undefined

/**
 * Membandingkan dua nilai sortir.
 *
 * Aturan yang dijaga:
 * - `null`/`undefined` selalu jatuh ke akhir daftar, apa pun arah urutannya
 *   saat dibandingkan satu sama lain, sehingga baris tanpa data tidak
 *   menyela baris yang berisi.
 * - Angka dibandingkan secara numerik, bukan leksikografis (`10` setelah `9`).
 * - String memakai `localeCompare` dengan opsi `numeric` supaya nama seperti
 *   `asset_2` berada sebelum `asset_10`.
 */
export function compareValues(a: SortableValue, b: SortableValue): number {
  const aEmpty = a === null || a === undefined
  const bEmpty = b === null || b === undefined
  if (aEmpty && bEmpty) return 0
  if (aEmpty) return 1
  if (bEmpty) return -1
  if (typeof a === "number" && typeof b === "number") return a - b
  return String(a).localeCompare(String(b), undefined, { numeric: true })
}
