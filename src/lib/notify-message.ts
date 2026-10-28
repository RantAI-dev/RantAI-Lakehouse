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
  too_many_requests: "Wait for the lockout to end, then try again.",
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
  const known = prefixedHint(serviceError.message)
  if (known !== null) return known
  const detail = serviceError.message.trim()
  return detail && hint ? `${detail} ${hint}` : detail || hint
}

/**
 * The hint of a message that starts with one of the API's error codes
 * (`permission_denied: dashboard:write`, `lakehouse-core` `error.rs`), or
 * `null` when it does not. The code and what follows it are for the log, not
 * the reader (BI-18 part C review fix (SHOULD-FIX) R20).
 */
function prefixedHint(message: string): string | null {
  const code = /^\s*([a-z][a-z_]*)(?::|\s*$)/.exec(message)?.[1]
  if (!code || code === "aborted" || !(code in HINTS)) return null
  return HINTS[code as ServiceErrorCode]
}

/**
 * The text to show for a raw error message (a banner, an inline error): the
 * hint sentence alone when it starts with a known code, else the message as it
 * is. BI-18 part C review fix (SHOULD-FIX) R20; the dashboard page's red
 * banner used to print the raw text, code first.
 */
export function readableErrorMessage(message: string): string {
  const hint = prefixedHint(message)
  if (hint === null) return message
  // BI-18 part C review fix R20 (follow-up: keep the server's sentence): when
  // what follows the code is a sentence the server wrote for the reader ("Downloads
  // are switched off for your role."), show it; the generic hint is for a bare
  // permission id such as `dashboard:write`, which tells the reader nothing.
  const detail = message.slice(message.indexOf(":") + 1).trim()
  return message.includes(":") && /\s/.test(detail) && detail.endsWith(".") ? detail : hint
}
