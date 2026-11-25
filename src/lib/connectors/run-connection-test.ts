import { runDiscovery } from "@/lib/connectors/run-discovery"
import type { ConnectorService, ConnectorTestResult } from "@/services/contracts/connectors"

type TestService = Pick<ConnectorService, "testConnection" | "getDiscovery" | "listProbeHistory">

/**
 * Test a connector's connection and wait for the end of it (`SRC-4` T1.1).
 *
 * Most connectors are tested inside the request. Oracle's driver only the orchestrator has, so for it
 * `POST /connectors/{id}/test` answers `pending: true` at once (nothing dialled, nothing claimed) and the job
 * posts its result a few seconds later. This waits on the connector's `test` row as a listing waits on its
 * answer (`runDiscovery`), then reads the result the API recorded as a test (stored health and probe history):
 * the latest probe, if it is from this test, is the answer, so the sentence shown is the API's fixed one and the
 * latency is the one the orchestrator measured. A test whose job never reported is a `timeout` the API words
 * itself (it is not recorded as a probe); that is shown as its `reason`, a failed test with no latency.
 *
 * A pending answer that is already `ok` is a test that PASSED and only waits for its setup checks (an Oracle
 * change-capture connector: Kafka Connect validated the connection, then the orchestrator reads the source's
 * setup, `SRC-4` Amendment 11). That one is not replaced by a recorded probe: its own message is the answer,
 * and the checks of the orchestrator's read are added to it when they arrive.
 *
 * # Errors
 *
 * Rejects with whatever the service rejects with, and with an `AbortError` when `signal` aborts.
 */
export async function runConnectionTest(
  service: TestService,
  id: string,
  signal?: AbortSignal,
  pollMs?: number
): Promise<ConnectorTestResult> {
  const first = await service.testConnection(id, signal)
  if (!first.pending) return first
  if (first.ok) return withSetupChecks(service, id, first, signal, pollMs)
  const answer = await runDiscovery({
    kind: "test",
    // The launch is the call above; what runs from here is the wait.
    start: async () => ({ kind: "test", state: "running" }),
    get: () => service.getDiscovery(id, "test", signal),
    signal,
    pollMs,
  })
  const history = await service.listProbeHistory(id, 1, signal)
  const latest = history.results[0]
  // Recorded by this test: the API stamps the probe when the result is posted, after it recorded the launch
  // (`startedAt` of the answer; both are the server's clock, never this browser's).
  if (latest && answer.startedAt && Date.parse(latest.testedAt) >= Date.parse(answer.startedAt)) {
    return {
      ok: latest.ok,
      supported: true,
      latencyMs: latest.latencyMs,
      message: latest.message,
      testedAt: latest.testedAt,
      // MongoDB change capture (`SRC-4` Amendment 8, S8-e): the source's setup checks ride with the answer.
      ...(answer.setupChecks && answer.setupChecks.length > 0 ? { setupChecks: answer.setupChecks } : {}),
    }
  }
  return {
    ok: false,
    supported: true,
    latencyMs: null,
    message: answer.reason ?? "The connection test did not finish.",
    testedAt: null,
  }
}

/** The passed answer `first`, with the setup checks of the orchestrator's read once it ends. A read that
 * cannot be waited for leaves the passed answer as it is, without checks: they are advice, never the verdict. */
async function withSetupChecks(
  service: TestService,
  id: string,
  first: ConnectorTestResult,
  signal?: AbortSignal,
  pollMs?: number
): Promise<ConnectorTestResult> {
  try {
    const answer = await runDiscovery({
      kind: "test",
      start: async () => ({ kind: "test", state: "running" }),
      get: () => service.getDiscovery(id, "test", signal),
      signal,
      pollMs,
    })
    return { ...first, pending: false, ...(answer.setupChecks && answer.setupChecks.length > 0 ? { setupChecks: answer.setupChecks } : {}) }
  } catch (error) {
    if (signal?.aborted) throw error
    return { ...first, pending: false }
  }
}
