const LAST_BOARD_KEY = "dashboards:last-board";

/**
 * Ingatan board yang terakhir dibuka.
 *
 * `/dashboards` adalah resolver, bukan halaman: ia meneruskan ke board yang
 * terakhir dibuka supaya kembali ke Dashboards dari menu lain tidak memaksa
 * lewat daftar. Daftarnya ada di `/dashboards/browse`, dituju dengan sengaja.
 *
 * Disimpan di localStorage — per-browser, bukan per-akun. Cukup untuk
 * preferensi navigasi: salah tebak paling buruk hanya membuka board lain,
 * dan tiap board tetap dijaga izinnya di server.
 */
export function readLastBoard(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(LAST_BOARD_KEY) || null;
  } catch {
    return null;
  }
}

export function rememberLastBoard(id: string) {
  if (typeof window === "undefined" || !id) return;
  try {
    window.localStorage.setItem(LAST_BOARD_KEY, id);
  } catch {
    /* mode privat / kuota penuh — navigasi tetap jalan tanpa ingatan */
  }
}

/** Panggil saat board dihapus, supaya resolver tidak menuju board hantu. */
export function forgetLastBoard(id?: string) {
  if (typeof window === "undefined") return;
  try {
    if (id && window.localStorage.getItem(LAST_BOARD_KEY) !== id) return;
    window.localStorage.removeItem(LAST_BOARD_KEY);
  } catch {
    /* ignore */
  }
}
