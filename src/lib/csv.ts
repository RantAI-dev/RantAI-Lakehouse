/**
 * Serialisasi hasil query ke CSV (RFC 4180).
 *
 * Dipisah dari komponen supaya aturan escaping-nya bisa diuji — inilah bagian
 * yang paling mudah salah dan paling mahal kalau bocor ke file yang diunduh
 * pengguna.
 */

/**
 * Membungkus satu sel bila mengandung karakter yang bisa merusak struktur.
 *
 * Aturan RFC 4180: sel yang memuat koma, tanda kutip ganda, CR, atau LF harus
 * dibungkus tanda kutip ganda, dan setiap tanda kutip di dalamnya digandakan.
 */
function escapeCell(value: string): string {
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
