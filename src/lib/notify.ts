import { toast } from "sonner"
import { buildErrorDescription } from "./notify-message"

/**
 * Umpan balik aksi mutasi (simpan, hapus, test connection, trigger run).
 *
 * Dipakai sebagai satu-satunya pintu ke `sonner` supaya:
 * 1. Pesan error `ServiceError` diterjemahkan seragam ke bahasa yang bisa
 *    ditindaklanjuti, bukan kode mentah seperti `permission_denied`.
 * 2. Error `aborted` TIDAK PERNAH memunculkan toast. Request yang dibatalkan
 *    (unmount / `useServiceAction` menimpa panggilan sebelumnya) adalah alur
 *    normal, bukan kegagalan.
 *
 * Penyusunan teksnya sendiri ada di `./notify-message` supaya bisa diuji
 * tanpa memuat `sonner` (yang butuh DOM).
 *
 * `<Toaster />` dipasang di `src/app/layout.tsx`.
 */

/** Notifikasi sukses. */
export function notifySuccess(message: string, description?: string): void {
  toast.success(message, { description })
}

/** Notifikasi informasi netral. */
export function notifyInfo(message: string, description?: string): void {
  toast.info(message, { description })
}

/**
 * Notifikasi gagal dari nilai apa pun yang dilempar.
 *
 * Mengembalikan `false` ketika error-nya `aborted` (tidak ada toast yang
 * ditampilkan) sehingga pemanggil bisa membedakan "gagal betulan" dari
 * "dibatalkan" tanpa mengulang pengecekan kode.
 */
export function notifyError(title: string, err: unknown): boolean {
  const description = buildErrorDescription(err)
  if (description === null) return false

  toast.error(title, { description })
  return true
}

/**
 * Membungkus fungsi aksi agar otomatis mengabarkan hasilnya.
 *
 * Dipakai sebagai pembungkus argumen `useServiceAction` supaya notifikasi
 * terjadi DI DALAM action. Membaca `action.error` tepat setelah `await run()`
 * tidak bisa diandalkan karena state baru ter-commit pada render berikutnya.
 *
 * Error tetap dilempar ulang supaya `useServiceAction` tetap mencatat
 * status `error` dan UI di halaman tidak berubah perilakunya.
 */
export function withNotify<Args extends unknown[], T>(
  messages: { success: string; error: string },
  action: (signal: AbortSignal, ...args: Args) => Promise<T>
): (signal: AbortSignal, ...args: Args) => Promise<T> {
  return async (signal, ...args) => {
    try {
      const result = await action(signal, ...args)
      // Request yang dibatalkan bukan keberhasilan — jangan beri kabar apa pun.
      if (!signal.aborted) notifySuccess(messages.success)
      return result
    } catch (err) {
      notifyError(messages.error, err)
      throw err
    }
  }
}
