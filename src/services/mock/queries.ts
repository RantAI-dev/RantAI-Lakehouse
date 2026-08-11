import { mockCall, stableHash } from "../transport"
import { agoIso } from "./mock-time"
import type { QueryService } from "../contracts/queries"
import type { EngineCategory, WorkloadClass } from "@/lib/status"

function classify(sql: string): {
  workloadClass: WorkloadClass
  engine: EngineCategory
  cacheEligible: boolean
} {
  const s = sql.toLowerCase()
  if (s.includes("vector") || s.includes("embedding")) {
    return { workloadClass: "retrieval", engine: "ai-store", cacheEligible: false }
  }
  if (s.includes("iceberg") || s.includes("lake.") || s.includes("join")) {
    return {
      workloadClass: s.includes("join") ? "join-heavy" : "federated",
      engine: "federated-compute",
      cacheEligible: false,
    }
  }
  return {
    workloadClass: "hot-analytics",
    engine: "hot-store",
    cacheEligible: !s.includes("now()"),
  }
}

export const mockQueryService: QueryService = {
  listSaved(signal) {
    return mockCall(
      () => [
        {
          id: "sq-1",
          title: "Revenue by region",
          sql: "SELECT region, sum(amount) FROM gold.revenue GROUP BY region",
          owner: "Rina Wijaya",
          updatedAt: agoIso(120),
          tags: ["finance", "gold"],
        },
        {
          id: "sq-2",
          title: "Hot + cold customer join",
          sql: "SELECT c.id, h.orders FROM hot.customers c JOIN lake.orders_history h ON c.id = h.customer_id LIMIT 100",
          owner: "Bayu Pratama",
          updatedAt: agoIso(400),
          tags: ["federated"],
        },
      ],
      { signal }
    )
  },
  listHistory(signal) {
    return mockCall(
      () => [
        {
          id: "qh-1",
          sql: "SELECT count(*) FROM payments_enriched WHERE day = today()",
          user: "Rina Wijaya",
          at: agoIso(9),
          status: "completed" as const,
          durationMs: 420,
          scannedBytes: 2.1 * 1024 ** 9,
          costUnits: 0.014,
          workloadClass: "hot-analytics" as const,
          engine: "hot-store" as const,
          cacheAssisted: true,
        },
        {
          id: "qh-2",
          sql: "SELECT * FROM lake.orders_history JOIN core.customer.customer_360 USING (customer_id) LIMIT 500",
          user: "Bayu Pratama",
          at: agoIso(40),
          status: "completed" as const,
          durationMs: 3200,
          scannedBytes: 48 * 1024 ** 9,
          costUnits: 0.42,
          workloadClass: "federated" as const,
          engine: "federated-compute" as const,
          cacheAssisted: false,
        },
      ],
      { signal }
    )
  },
  estimate(sql, signal) {
    return mockCall(() => {
      const c = classify(sql)
      const h = stableHash(sql)
      const bytes = 50_000_000 + (h % 80) * 25_000_000
      return {
        estimatedBytes: bytes,
        estimatedCostMin: Number(((bytes / 1e12) * 0.8).toFixed(4)),
        estimatedCostMax: Number(((bytes / 1e12) * 1.4).toFixed(4)),
        workloadClass: c.workloadClass,
        engine: c.engine,
        cacheEligible: c.cacheEligible,
        freshnessLagSeconds: 12 + (h % 120),
        policyObligations: ["column mask: email", "row filter: tenant_id"],
        sources:
          c.engine === "federated-compute"
            ? ["hot analytical store", "open cold tables"]
            : ["hot analytical store"],
      }
    }, { signal, delayMs: 200 })
  },
  run(sql, signal) {
    return mockCall(() => {
      const c = classify(sql)
      const h = stableHash(sql)
      return {
        columns: ["region", "amount", "orders"],
        rows: [
          { region: "Jabodetabek", amount: "1284000000", orders: "84211" },
          { region: "Jawa Timur", amount: "612000000", orders: "40122" },
          { region: "Bali", amount: "198500000", orders: "12004" },
        ],
        metrics: {
          durationMs: 380 + (h % 900),
          scannedBytes: 80_000_000 + (h % 40) * 10_000_000,
          costUnits: Number((0.01 + (h % 50) / 1000).toFixed(4)),
          engine: c.engine,
          workloadClass: c.workloadClass,
          cacheHit: c.cacheEligible && h % 3 === 0,
          pushdowns: ["filter", "projection", "partial aggregate"],
          policyObligations: ["column mask: email", "row filter: tenant_id"],
        },
      }
    }, { signal, delayMs: 700 })
  },
  generateSql(question, signal) {
    return mockCall(
      () => ({
        sql: `SELECT region, sum(amount) AS revenue\nFROM gold.revenue\nWHERE order_date >= today() - 90\nGROUP BY region\nORDER BY revenue DESC`,
        explanation: `This query aggregates revenue by region for the last 90 days from the curated gold.revenue table.`,
        assumptions: [
          `Interpreted “last quarter” as trailing 90 days.`,
          `Used gold.revenue as the governed metric source.`,
        ],
      }),
      { signal, delayMs: 900 }
    )
  },
  listCollaboration(signal) {
    return mockCall(
      () => [
        {
          id: "col-finance",
          name: "Finance analytics workspace",
          members: 8,
          updatedAt: agoIso(60),
          description: "Shared revenue and collections queries.",
        },
        {
          id: "col-risk",
          name: "Risk investigation",
          members: 5,
          updatedAt: agoIso(200),
          description: "Fraud and credit risk collaborative notebooks.",
        },
      ],
      { signal }
    )
  },
}
