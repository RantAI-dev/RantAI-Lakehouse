import type { Health } from "@/lib/status"
import type { Measured } from "@/lib/measured"

export type PlatformService = {
  id: string
  name: string
  health: Health
  // `true` only for ClickHouse and Dagster, the two services actually
  // probed this request; Iceberg/Lakekeeper and RustFS are never probed
  // (WS5 is expected to add real probes for them).
  checked: boolean
  // Nothing measures version, replica count, error rate, or latency for
  // any service today; null replaces the "-"/1/0/0 literals.
  version: string | null
  site: string
  replicas: Measured
  errorRate: Measured
  latencyMs: Measured
  dependencies: string[]
}

export interface OpsService {
  listServices(signal?: AbortSignal): Promise<PlatformService[]>
}
