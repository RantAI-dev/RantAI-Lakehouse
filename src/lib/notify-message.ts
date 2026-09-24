import { toServiceError, type ServiceErrorCode } from "@/services/errors"

/**
 * Penyusunan pesan notifikasi — sengaja dipisah dari `notify.ts`.
 *
 * `notify.ts` mengimpor `sonner`, yang menyentuh DOM saat dimuat sehingga
 * tidak bisa diimpor dari unit test yang berjalan di Node. Logika yang
 * benar-benar perlu diuji (pemetaan kode error ke saran langkah lanjut, dan
 * keputusan menelan `aborted`) tinggal di sini supaya bisa diuji langsung
 * tanpa DOM maupun hook khusus test di kode produksi.
 */

/**
 * Saran langkah lanjut per kode error. Rubrik audit menuntut error yang
 * "menjelaskan langkah berikutnya, bukan sekadar kode".
 */
const HINTS: Record<ServiceErrorCode, string> = {
  not_found: "It may already have been deleted. Reload the list.",
  permission_denied:
    "This account does not have permission. Ask a workspace admin.",
  unavailable: "The service is unavailable right now. Try again shortly.",
  invalid_request: "Check the input before sending it again.",
  aborted: "",
}

/**
 * Menyusun deskripsi toast dari nilai apa pun yang dilempar.
 *
 * Mengembalikan `null` untuk error `aborted`: request yang dibatalkan
 * (unmount, atau `useServiceAction` menimpa panggilan sebelumnya) adalah alur
 * normal, bukan kegagalan. Ini konsisten dengan `useService`/`useServiceAction`
 * di `@/hooks/use-service` yang juga menelan kode tersebut.
 */
export function buildErrorDescription(err: unknown): string | null {
  const serviceError = toServiceError(err)
  if (serviceError.code === "aborted") return null

  const hint = HINTS[serviceError.code]
  const detail = serviceError.message.trim()
  return detail && hint ? `${detail} ${hint}` : detail || hint
}
