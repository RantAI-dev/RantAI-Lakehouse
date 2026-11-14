"use client"

import * as React from "react"
import { useRouter } from "next/navigation"

import { ErrorState } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { dashboardService } from "@/services"
import { DashboardPageSkeleton } from "./dashboard-skeleton"
import { forgetLastBoard, readLastBoard } from "./last-board"

/**
 * `/dashboards` tidak menampilkan apa-apa — ia memutuskan ke mana kita pergi.
 *
 * Urutannya: board yang terakhir dibuka, lalu — kalau belum ada ingatan dan
 * belum ada board buatan sendiri — board bawaan, dan baru daftar di
 * `/dashboards/browse` kalau memang ada sesuatu untuk dipilih. Daftar adalah
 * tempat mengelola dashboard, bukan pintu yang harus dilewati setiap kali.
 *
 * Ingatan divalidasi terhadap daftar dari server, jadi board yang sudah
 * dihapus (atau dihapus dari perangkat lain) tidak membuat kita mendarat di
 * kanvas kosong.
 */
export function DashboardResolver() {
  const router = useRouter()
  // Bukan destructuring: `status` dan `error` satu discriminated union,
  // memecahnya membuat TypeScript kehilangan penyempitan tipe.
  const state = useService((signal) => dashboardService.listBoards(signal), [])
  const { status, data } = state

  React.useEffect(() => {
    if (status !== "success") return

    const boards = data ?? []
    const remembered = readLastBoard()

    if (remembered) {
      if (boards.some((b) => b.id === remembered)) {
        router.replace(`/dashboards/${remembered}`)
        return
      }
      // Board-nya sudah tidak ada — buang ingatannya, jangan diikuti.
      forgetLastBoard(remembered)
    }

    // Board bawaan SELALU ada di daftar, jadi `boards.length` tidak pernah
    // menjawab "berapa dashboard yang saya punya". Yang ditanya di sini
    // adalah apakah sudah ada board buatan sendiri; kalau belum, tidak ada
    // pilihan untuk dibuat dan daftar hanya jadi perantara kosong.
    const own = boards.filter((b) => !b.builtin)
    if (own.length === 0) {
      const builtin = boards.find((b) => b.builtin) ?? boards[0]
      if (builtin) {
        router.replace(`/dashboards/${builtin.id}`)
        return
      }
    }

    router.replace("/dashboards/browse")
  }, [status, data, router])

  if (state.status === "error") {
    return (
      <div className="p-4">
        <ErrorState error={state.error} onRetry={state.reload} />
      </div>
    )
  }

  // Termasuk saat status sudah "success": `replace` baru jalan setelah efek,
  // dan menampilkan daftar sekejap di sini justru kedipan yang ingin dihindari.
  // The dashboard's own placeholder, the one the loading shell draws for this
  // route (`shellPlaceholderFor`) and the board page keeps until its data
  // lands: the usual way on from here is a board, so a refresh of
  // `/dashboards` is one loading state, not three (owner QA 2026-11-14).
  return <DashboardPageSkeleton />
}
