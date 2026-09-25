import type { Board } from "../clients/bi-store"

/**
 * Kontrak untuk halaman DAFTAR dashboard (`/dashboards/browse`).
 *
 * Sengaja sempit. Kanvas dashboard (`/dashboards/[id]`) masih memanggil
 * `/api/dashboard/*` lewat `apiFetch` langsung, dan memindahkannya ke sini
 * akan menyentuh berkas yang belum perlu disentuh. Yang dibutuhkan halaman
 * daftar hanya: baca semua board, lalu buat / rename / ubah deskripsi /
 * duplikat / hapus satu board.
 */
export type DashboardService = {
  /** Semua board, board bawaan (`default`) di urutan pertama. */
  listBoards(signal?: AbortSignal): Promise<Board[]>
  createBoard(input: CreateBoardInput, signal?: AbortSignal): Promise<Board>
  /** Salin sebuah board beserta seluruh chart-nya. */
  duplicateBoard(id: string, signal?: AbortSignal): Promise<Board>
  updateBoard(input: UpdateBoardInput, signal?: AbortSignal): Promise<void>
  deleteBoard(id: string, signal?: AbortSignal): Promise<void>
}

export type CreateBoardInput = {
  name: string
  description?: string
}

/**
 * Field yang tidak dikirim tidak akan diubah — bukan dikosongkan. Karena
 * itu `description: ""` adalah cara sah untuk MENGHAPUS deskripsi, dan
 * berbeda maknanya dari `description: undefined`.
 */
export type UpdateBoardInput = {
  id: string
  name?: string
  description?: string
}
