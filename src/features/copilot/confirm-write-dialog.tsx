"use client";

import * as React from "react";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { useCopilot } from "./use-copilot";

/**
 * Gerbang persetujuan sebelum Copilot menjalankan aksi yang mengubah data.
 *
 * Dipasang sekali di `CopilotProvider` sehingga berlaku untuk SEMUA pintu
 * masuk percakapan (halaman `/copilot`, dock global, dan pill saran) tanpa
 * masing-masing perlu memasang dialognya sendiri.
 *
 * Kenapa konfirmasinya per-pesan, bukan per-tool: tool loop dieksekusi di
 * backend — `POST /api/ai/chat` baru mengembalikan `toolTrace` setelah semua
 * tool selesai dijalankan. Tidak ada titik di frontend untuk menyela di
 * tengah, jadi persetujuan diminta pada satu-satunya saat yang masih bisa
 * mencegah eksekusi: sebelum request dikirim.
 */
export function CopilotConfirmWriteDialog() {
  const { pendingSend, writeCaps, confirmSend, cancelSend } = useCopilot();

  const capLabels = writeCaps.map((c) => c.label).join(", ");

  return (
    <ConfirmActionDialog
      open={pendingSend !== null}
      onOpenChange={(open) => {
        if (!open) cancelSend();
      }}
      title="Run Copilot in Build mode?"
      description="Copilot may create, update, or delete resources to fulfil this request."
      impact={
        capLabels
          ? `Enabled write capabilities: ${capLabels}. Turn them off in the Tools menu to keep this conversation read-only.`
          : undefined
      }
      confirmLabel="Run"
      destructive
      onConfirm={confirmSend}
    >
      {pendingSend ? (
        <p className="rounded-md bg-muted/50 px-3 py-2 text-sm text-muted-foreground">
          {pendingSend}
        </p>
      ) : null}
    </ConfirmActionDialog>
  );
}
