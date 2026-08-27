import type {
  CreateKnowledgeSourceInput,
  CreateVectorJobInput,
  KnowledgeService,
  KnowledgeSource,
  SearchHit,
  SearchStrategy,
  VectorJob,
} from "../contracts/knowledge"
import { ServiceError } from "../errors"
import { mockKnowledgeService } from "../mock/knowledge"

/**
 * KnowledgeService SEBAGIAN NYATA — sumber pengetahuan (`sources`) dan
 * vector job tersimpan di Postgres lewat route `/api/knowledge/*` (crate
 * `lakehouse-store`, Task 2.8).
 *
 * `search` TETAP mock, dengan sengaja — bukan celah yang terlewat. Tidak
 * ada vector database, mesin embedding, atau index pencarian di mana pun
 * dalam repo ini maupun infrastruktur nyata yang dituju deployment ini
 * (lihat `rust/crates/lakehouse-store/src/knowledge.rs` untuk catatan
 * investigasi: query langsung ke ClickHouse produksi tidak menemukan kolom
 * `Array(Float32|Float64)` embedding atau vector index apa pun).
 * Memalsukan skor kemiripan terhadap dokumen yang sebenarnya tidak
 * terindeks di mana pun akan LEBIH BURUK daripada mock yang jujur — jadi
 * `search` di sini murni mendelegasikan ke `mockKnowledgeService.search`.
 */

function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message)
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message)
  if (status === 401 || status === 403) return new ServiceError("permission_denied", message)
  return new ServiceError("unavailable", message)
}

async function request<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await fetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? fallback)
  }
  return json as T
}

function get<T>(url: string, signal: AbortSignal | undefined, fallback: string): Promise<T> {
  return request<T>(url, { signal }, fallback)
}

function post<T>(
  url: string,
  body: unknown,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  return request<T>(
    url,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal,
    },
    fallback
  )
}

export const postgresKnowledgeService: KnowledgeService = {
  listSources(signal) {
    return get<KnowledgeSource[]>(
      "/api/knowledge/sources",
      signal,
      "Daftar sumber pengetahuan gagal dimuat"
    )
  },
  listVectorJobs(signal) {
    return get<VectorJob[]>("/api/knowledge/vector-jobs", signal, "Daftar vector job gagal dimuat")
  },
  // Tidak nyata — lihat catatan komentar modul ini. Mendelegasikan ke mock.
  search(query: string, strategy: SearchStrategy, signal?: AbortSignal): Promise<SearchHit[]> {
    return mockKnowledgeService.search(query, strategy, signal)
  },
  createSource(input: CreateKnowledgeSourceInput, signal) {
    return post<KnowledgeSource>(
      "/api/knowledge/sources",
      input,
      signal,
      "Membuat sumber pengetahuan gagal"
    )
  },
  createVectorJob(input: CreateVectorJobInput, signal) {
    return post<VectorJob>(
      "/api/knowledge/vector-jobs",
      input,
      signal,
      "Membuat vector job gagal"
    )
  },
}
