import type {
  Connector,
  ConnectorDetail,
  ConnectorService,
  ConnectorTestResult,
  ConnectorType,
  CreateConnectorInput,
  CreateConnectorResponse,
  DebeziumProperties,
  DiscoverResult,
  IngestibleConnector,
  IngestRun,
  IngestRunResult,
  IngestSpec,
  IngestSpecInput,
  ProbeHistoryResponse,
  RotateConnectorSecretRequest,
  RotateConnectorSecretResponse,
} from "../contracts/connectors";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * ConnectorService is real — connector definitions (source/sink) go through
 * the `/api/connectors` route, backed by Postgres (`lakehouse-store`, Task 2.7).
 *
 * CREDENTIAL NOTE: `CreateConnectorInput.credential` (ADR 0002 Addendum 3)
 * chooses a source/kind, never a reference NAME — the server derives the
 * actual reference (where a credential is stored: an env var name, a
 * secret-manager path) from the id it generates, and returns it once in
 * `CreateConnectorResponse.credential`. The backend never stores, returns,
 * logs, or displays a credential VALUE; `Connector`/`ConnectorDetail` do not
 * even have a field for one.
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status);
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status);
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
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status);
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
    return postJson<CreateConnectorResponse>("/api/connectors", input, signal);
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
  listTypes(signal) {
    return getJson<ConnectorType[]>("/api/connectors/types", { signal });
  },
  listIngestible(signal) {
    return getJson<IngestibleConnector[]>("/api/connectors/ingestible", { signal });
  },
  discoverConnector(id, signal) {
    return postJson<DiscoverResult>(`/api/connectors/${encodeURIComponent(id)}/discover`, undefined, signal);
  },
  runIngest(id, signal) {
    return postJson<IngestRunResult>(`/api/connectors/${encodeURIComponent(id)}/ingest/run`, undefined, signal);
  },
  listIngestRuns(connectorId, signal) {
    return getJson<IngestRun[]>(
      `/api/governance/ingest-runs?connectorId=${encodeURIComponent(connectorId)}`,
      { signal }
    );
  },
  getDebeziumProperties(id, table, signal) {
    return getJson<DebeziumProperties>(
      `/api/connectors/${encodeURIComponent(id)}/debezium-properties?table=${encodeURIComponent(table)}`,
      { signal }
    );
  },
  listProbeHistory(id, limit, signal) {
    const query = limit === undefined ? "" : `?limit=${encodeURIComponent(limit)}`;
    return getJson<ProbeHistoryResponse>(
      `/api/connectors/${encodeURIComponent(id)}/probe-history${query}`,
      { signal }
    );
  },
  rotateSecret(id, body: RotateConnectorSecretRequest, signal) {
    return putJson<RotateConnectorSecretResponse>(
      `/api/connectors/${encodeURIComponent(id)}/secret`,
      body,
      signal
    );
  },
};
