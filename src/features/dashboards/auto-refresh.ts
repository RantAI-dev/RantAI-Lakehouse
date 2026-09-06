"use client";

import * as React from "react";

/**
 * Auto-refresh berkala untuk kanvas dashboard.
 *
 * BATASAN YANG DISENGAJA: ini penyegaran sisi-KLIEN, bukan penjadwalan
 * sungguhan. Backend Rust tidak menyediakan endpoint penjadwalan dashboard
 * (lihat tabel route di `rust/crates/lakehouse-api/src/routes/mod.rs` — ada
 * `/api/dashboard/{specs,boards,fields,records,values,export,embed-info}`,
 * tidak ada satu pun untuk jadwal), dan penjadwalan sejati butuh penyimpanan
 * jadwal plus scheduler yang jalan di server, bukan di tab browser.
 *
 * Konsekuensinya: interval hanya berlaku selama tab terbuka dan tidak
 * tersimpan antar sesi. Label di UI harus menyebutnya "Auto-refresh", bukan
 * "Schedule", supaya harapan penggunanya tidak keliru.
 */

/**
 * Pilihan interval, dalam DETIK sebagai string agar cocok dengan nilai
 * `Select`. `"0"` berarti penyegaran manual.
 */
export const REFRESH_INTERVALS = [
  { value: "0", label: "Manual" },
  { value: "30", label: "Every 30s" },
  { value: "60", label: "Every 1m" },
  { value: "300", label: "Every 5m" },
] as const;

/**
 * Memanggil `onRefresh` setiap `intervalMs`.
 *
 * Penyegaran ditunda ketika tab tersembunyi: menembak query analitik untuk
 * tab yang tidak dilihat siapa pun hanya membuang kuota ClickHouse. Timer
 * dijalankan ulang begitu tab kembali terlihat.
 */
export function useAutoRefresh(
  intervalMs: number,
  onRefresh: () => void | Promise<void>
): void {
  // Simpan callback di ref agar perubahan identitasnya tidak me-reset timer
  // pada setiap render induknya.
  const callbackRef = React.useRef(onRefresh);
  React.useEffect(() => {
    callbackRef.current = onRefresh;
  });

  React.useEffect(() => {
    if (intervalMs <= 0) return;

    let timer: ReturnType<typeof setInterval> | null = null;

    const start = () => {
      if (timer !== null) return;
      timer = setInterval(() => {
        void callbackRef.current();
      }, intervalMs);
    };

    const stop = () => {
      if (timer === null) return;
      clearInterval(timer);
      timer = null;
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };

    if (document.visibilityState === "visible") start();
    document.addEventListener("visibilitychange", onVisibilityChange);

    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [intervalMs]);
}
