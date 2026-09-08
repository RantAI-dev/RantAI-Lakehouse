import type {
  Connector,
  ConnectorDetail,
  ConnectorService,
  ConnectorTestResult,
  CreateConnectorInput,
} from "../contracts/connectors";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * ConnectorService NYATA — definisi konektor (source/sink) lewat route
 * `/api/connectors`, backed oleh Postgres (`lakehouse-store`, Task 2.7).
 *
 * CATATAN KREDENSIAL: `CreateConnectorInput.secretRef` adalah REFERENSI ke
 * tempat kredensial disimpan (nama env var, path secret manager) — bukan
 * nilai kredensial itu sendiri. Backend tidak pernah menyimpan, mengembalikan,
 * mencatat log, atau menampilkan nilai kredensial; `Connector`/`ConnectorDetail`
 * bahkan tidak punya field untuk itu.
 *
 * `testConnection` di sini SEKARANG melakukan probe jaringan nyata — tapi
 * hanya untuk PostgreSQL dan object storage S3-compatible, satu-satunya dua
 * tipe yang build ini tahu cara menghubunginya. Tipe lain (Kafka, MQTT,
 * MongoDB, dst.) mengembalikan `supported: false`, bukan hasil palsu. Lihat
 * `rust/crates/lakehouse-api/src/connector_probe.rs` untuk implementasinya
 * dan `rust/crates/lakehouse-store/src/connectors.rs` untuk catatan
 * keputusan kredensial lengkap.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
  return json as T;
}

async function postJson<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
  return json as T;
}

export const postgresConnectorService: ConnectorService = {
  listConnectors(signal) {
    return getJson<Connector[]>("/api/connectors", { signal });
  },
  getConnector(id, signal) {
    return getJson<ConnectorDetail>(`/api/connectors/${encodeURIComponent(id)}`, { signal });
  },
  createConnector(input: CreateConnectorInput, signal) {
    return postJson<Connector>("/api/connectors", input, signal);
  },
  testConnection(id, signal) {
    return postJson<ConnectorTestResult>(`/api/connectors/${encodeURIComponent(id)}/test`, undefined, signal);
  },
};
