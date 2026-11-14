import * as React from "react"

const LAST_BOARD_KEY = "dashboards:last-board";
// Nama board terakhir, disimpan terpisah dari id supaya nilai lama (hanya id) dan
// pembaca id (resolver, Home) tidak berubah. Berisi `{ id, name }`: nama hanya
// dipakai kalau id-nya masih sama dengan board yang diingat.
const LAST_BOARD_NAME_KEY = "dashboards:last-board-name";
// Perubahan di tab ini diumumkan lewat event ini; tab lain lewat `storage`.
const LAST_BOARD_EVENT = "dashboards:last-board-change";

function announce() {
  try {
    window.dispatchEvent(new Event(LAST_BOARD_EVENT));
  } catch {
    /* ignore */
  }
}

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

/** Nama board yang diingat, atau null (belum ada, nilai lama tanpa nama, atau id tidak cocok). */
export function readLastBoardName(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const id = window.localStorage.getItem(LAST_BOARD_KEY);
    const raw = window.localStorage.getItem(LAST_BOARD_NAME_KEY);
    if (!id || !raw) return null;
    const parsed = JSON.parse(raw) as { id?: unknown; name?: unknown };
    return parsed.id === id && typeof parsed.name === "string" && parsed.name.trim() ? parsed.name : null;
  } catch {
    return null;
  }
}

export function rememberLastBoard(id: string, name?: string) {
  if (typeof window === "undefined" || !id) return;
  try {
    window.localStorage.setItem(LAST_BOARD_KEY, id);
    if (name?.trim()) window.localStorage.setItem(LAST_BOARD_NAME_KEY, JSON.stringify({ id, name }));
    // Tanpa nama, nama lama dibiarkan: pembaca mengabaikannya kalau id-nya lain.
  } catch {
    /* mode privat / kuota penuh — navigasi tetap jalan tanpa ingatan */
  }
  announce();
}

function subscribe(onChange: () => void) {
  window.addEventListener(LAST_BOARD_EVENT, onChange);
  window.addEventListener("storage", onChange);
  return () => {
    window.removeEventListener(LAST_BOARD_EVENT, onChange);
    window.removeEventListener("storage", onChange);
  };
}

/**
 * Nama board yang akan dibuka `/dashboards`, untuk label menu. Server dan render
 * pertama memberi null (localStorage tidak terbaca di sana), jadi tidak ada
 * hydration mismatch; nama masuk setelah mount.
 */
export function useLastBoardName(): string | null {
  return React.useSyncExternalStore(subscribe, readLastBoardName, () => null);
}

/** Panggil saat board dihapus, supaya resolver tidak menuju board hantu. */
export function forgetLastBoard(id?: string) {
  if (typeof window === "undefined") return;
  try {
    if (id && window.localStorage.getItem(LAST_BOARD_KEY) !== id) return;
    window.localStorage.removeItem(LAST_BOARD_KEY);
    window.localStorage.removeItem(LAST_BOARD_NAME_KEY);
  } catch {
    /* ignore */
  }
  announce();
}

/** Id board yang diingat, sebagai hook (null di server dan render pertama). */
export function useLastBoardId(): string | null {
  return React.useSyncExternalStore(subscribe, readLastBoard, () => null);
}
