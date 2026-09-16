import type {
  Connector,
  ConnectorDetail,
  ConnectorService,
  ConnectorTestResult,
  CreateConnectorInput,
  IngestSpec,
  IngestSpecInput,
} from "../contracts/connectors";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * ConnectorService is real — connector definitions (source/sink) go through
 * the `/api/connectors` route, backed by Postgres (`lakehouse-store`, Task 2.7).
 *
 * CREDENTIAL NOTE: `CreateConnectorInput.secretRef` is a REFERENCE to where a
 * credential is stored (an env var name, a secret-manager path) — not the
 * credential value itself. The backend never stores, returns, logs, or
 * displays a credential value; `Connector`/`ConnectorDetail` do not even have
 * a field for one.
 *
 * `testConnection` here NOW performs a real network probe — but only for
 * PostgreSQL and S3-compatible object storage, the only two types this build
 * knows how to reach. Other types (Kafka, MQTT, MongoDB, etc.) return
 * `supported: false` rather than a fabricated result. See
 * `rust/crates/lakehouse-api/src/connector_probe.rs` for the implementation
 * and `rust/crates/lakehouse-store/src/connectors.rs` for the full
 * credential design rationale.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`);
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`);
  }
  return json as T;
}

async function putJson<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`);
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
  getIngestSpec(id, signal) {
    return getJson<IngestSpec>(`/api/connectors/${encodeURIComponent(id)}/ingest-spec`, { signal });
  },
  setIngestSpec(id, input: IngestSpecInput, signal) {
    return putJson<IngestSpec>(`/api/connectors/${encodeURIComponent(id)}/ingest-spec`, input, signal);
  },
};
