/**
 * Serialisasi hasil query ke CSV (RFC 4180).
 *
 * Dipisah dari komponen supaya aturan escaping-nya bisa diuji — inilah bagian
 * yang paling mudah salah dan paling mahal kalau bocor ke file yang diunduh
 * pengguna.
 */

/**
 * SEC-17: a cell whose first character is one of these is a formula to a
 * spreadsheet (a tab or a carriage return because some programs skip them
 * before looking), so data one person loaded would run on the machine of the
 * person who opens the export.
 */
const FORMULA_START = /^[=+\-@\t\r]/

/**
 * SEC-17: a plain number is never a formula, so `-5` and `+3.2e4` stay
 * numbers. Decided on the cell's text alone: `-1+1` is not a number.
 * `$` without the `m` flag matches only at the very end, so a number with a
 * line break after it is not plain, as in the Rust twin of this rule
 * (`needs_apostrophe` in `rust/crates/lakehouse-api/src/csv_safe.rs`).
 */
const PLAIN_NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/

/**
 * SEC-17: puts `'` in front of a cell that would start a formula. It goes on
 * before the quoting, so it ends up inside the quotes. The price is that a
 * program reading the export back gets `'=A1`, not `=A1`.
 */
function neutralizeFormula(value: string): string {
  return FORMULA_START.test(value) && !PLAIN_NUMBER.test(value)
    ? `'${value}`
    : value
}

/**
 * Membungkus satu sel bila mengandung karakter yang bisa merusak struktur.
 *
 * Aturan RFC 4180: sel yang memuat koma, tanda kutip ganda, CR, atau LF harus
 * dibungkus tanda kutip ganda, dan setiap tanda kutip di dalamnya digandakan.
 * Sebelum itu, sel yang diawali karakter rumus diberi apostrof di depan
 * (SEC-17), untuk header dan isi sama.
 */
function escapeCell(raw: string): string {
  const value = neutralizeFormula(raw)
  if (!/[",\r\n]/.test(value)) return value
  return `"${value.replace(/"/g, '""')}"`
}

/**
 * Menyusun teks CSV dari daftar kolom dan baris.
 *
 * Nilai `null`/`undefined` menjadi sel kosong, bukan string "null", supaya
 * hasil unduhan bisa dibaca ulang sebagai data kosong, bukan literal teks.
 *
 * Pemisah baris memakai CRLF sesuai RFC 4180 agar Excel di Windows tidak
 * menggabungkan seluruh isi ke satu baris.
 */
export function toCsv(
  columns: string[],
  rows: Record<string, string | null | undefined>[]
): string {
  const header = columns.map(escapeCell).join(",")
  const body = rows.map((row) =>
    columns.map((c) => escapeCell(row[c] ?? "")).join(",")
  )
  return [header, ...body].join("\r\n")
}

/**
 * Memicu unduhan file di browser.
 *
 * Object URL sengaja dicabut setelah dipakai; tanpa itu blob-nya bertahan
 * sepanjang umur dokumen dan menahan memori hasil query yang bisa besar.
 */
export function downloadCsv(filename: string, csv: string): void {
  // BOM UTF-8 supaya Excel mengenali karakter non-ASCII dengan benar.
  const blob = new Blob(["\ufeff", csv], {
    type: "text/csv;charset=utf-8;",
  })
  const url = URL.createObjectURL(blob)
  const link = document.createElement("a")
  link.href = url
  link.download = filename
  document.body.appendChild(link)
  link.click()
  document.body.removeChild(link)
  URL.revokeObjectURL(url)
}
