"use client";

import * as React from "react";

/**
 * Periodic auto-refresh for the dashboard canvas.
 *
 * DELIBERATE LIMIT: this is a CLIENT-side refresh, not a real schedule. The
 * Rust backend has no dashboard scheduler, and a real one would need
 * stored schedules plus a server-side runner rather than a browser tab.
 * It only runs while the tab is open and visible. The interval an editor
 * saves on a board (BI-18 part B, `lib/dashboard-refresh.ts`) is where the
 * tab STARTS; it does not make anything refresh while nobody looks. The UI
 * says "Auto-refresh", never "Schedule", so the expectation stays honest.
 */

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
