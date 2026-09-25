import type { Board } from "./bi-store"
import type {
  CreateBoardInput,
  DashboardService,
  UpdateBoardInput,
} from "../contracts/dashboards"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * DashboardService NYATA — board disimpan di ClickHouse
 * (`console.bi_board`), lewat route `/api/dashboard/boards`.
 *
 * Perlu diketahui saat membaca hasil `listBoards`: board bawaan (`default`)
 * bukan baris tabel. Ia disintesis server dan hanya membawa
 * `id`/`name`/`layout`/`chartCount`/`builtin` — tanpa timestamp, deskripsi,
 * atau token share. Ia juga tidak bisa di-rename maupun dihapus. Pemanggil
 * harus bercabang pada `board.builtin`, bukan berasumsi setiap field ada.
 */

async function readJson<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    const kind =
      res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request"
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`)
  }
  return json as T
}

async function writeBoard(
  method: "POST" | "PUT",
  body: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<Board> {
  const res = await apiFetch("/api/dashboard/boards", {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  })
  const json = await readJson<{ board?: Board }>(res)
  // PUT hanya mengembalikan `{ok:true}`; POST mengembalikan board barunya.
  return json.board ?? ({ id: String(body.id ?? ""), name: "" } as Board)
}

export const clickhouseDashboardService: DashboardService = {
  async listBoards(signal) {
    const res = await apiFetch("/api/dashboard/boards", { signal })
    const json = await readJson<{ boards?: Board[] }>(res)
    return json.boards ?? []
  },
  createBoard(input: CreateBoardInput, signal) {
    return writeBoard("POST", { name: input.name, description: input.description }, signal)
  },
  duplicateBoard(id, signal) {
    return writeBoard("POST", { duplicate: id }, signal)
  },
  async updateBoard(input: UpdateBoardInput, signal) {
    await writeBoard(
      "PUT",
      { id: input.id, name: input.name, description: input.description },
      signal,
    )
  },
  async deleteBoard(id, signal) {
    const res = await apiFetch(`/api/dashboard/boards?id=${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    })
    await readJson<unknown>(res)
  },
}
