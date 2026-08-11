import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { StreamingJob, StreamingService } from "../contracts/streaming"

const JOBS: StreamingJob[] = [
  {
    id: "sj-payments-flow",
    name: "rt.payments_flow_mv",
    status: "degraded",
    owner: "Payments Platform",
    sources: ["kafka.payments.events", "cdc.accounts"],
    sinks: ["hot.payments_flow", "kafka.agent.triggers"],
    lagSeconds: 42,
    throughputPerSec: 18_400,
    stateSizeBytes: 12.4 * 1024 ** 3,
    watermarkIntervalSec: 5,
    lastBarrierAt: agoIso(1),
  },
  {
    id: "sj-fraud-windows",
    name: "rt.fraud_window_agg",
    status: "running",
    owner: "Risk",
    sources: ["kafka.auth.events"],
    sinks: ["hot.fraud_signals"],
    lagSeconds: 3,
    throughputPerSec: 6_200,
    stateSizeBytes: 2.1 * 1024 ** 3,
    watermarkIntervalSec: 2,
    lastBarrierAt: agoIso(0),
  },
]

export const mockStreamingService: StreamingService = {
  listJobs(signal) {
    return mockCall(() => JOBS, { signal })
  },
  getJob(id, signal) {
    return mockCall(() => {
      const job = JOBS.find((j) => j.id === id)
      if (!job) throw new ServiceError("not_found", `Streaming job ${id} not found`)
      return {
        ...job,
        definitionSql: `CREATE MATERIALIZED VIEW ${job.name} AS\nSELECT window_start, count(*) AS events\nFROM ${job.sources[0]}\nGROUP BY tumble(event_time, INTERVAL '1' MINUTE);`,
        triggers: [
          {
            id: "tr-1",
            condition: "lag_seconds > 30",
            target: "agents/workflows/wf-lag-triage",
          },
        ],
        checkpoints: [
          { id: "cp-1", at: agoIso(5), sizeBytes: job.stateSizeBytes },
          { id: "cp-2", at: agoIso(20), sizeBytes: job.stateSizeBytes * 0.98 },
        ],
      }
    }, { signal })
  },
}
