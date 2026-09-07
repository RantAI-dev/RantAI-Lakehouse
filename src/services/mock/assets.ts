import { ServiceError } from "../errors"
import { mockCall, stableHash } from "../transport"
import { agoIso, daysAgoIso } from "./mock-time"
import type {
  Asset,
  AssetDetail,
  AssetService,
  CatalogNamespace,
} from "../contracts/assets"

const ASSETS: Asset[] = [
  {
    id: "tbl-orders-events",
    name: "orders_events",
    namespace: "core.sales",
    type: "table",
    layer: "raw",
    tier: "hot",
    classification: "internal",
    owner: "Data Platform",
    domain: "Sales",
    description: "Raw order events ingested from the commerce stream.",
    format: "MergeTree",
    engine: "hot-store",
    rows: 2_140_000_000,
    sizeBytes: 3.1 * 1024 ** 4,
    columnCount: 24,
    freshnessLagSeconds: 8,
    lastUpdated: agoIso(1),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "tbl-payments-enriched",
    name: "payments_enriched",
    namespace: "core.finance",
    type: "table",
    layer: "silver",
    tier: "hot",
    classification: "confidential",
    owner: "Finance Data",
    domain: "Finance",
    description: "Validated and enriched payment transactions.",
    format: "MergeTree",
    engine: "hot-store",
    rows: 890_000_000,
    sizeBytes: 1.8 * 1024 ** 4,
    columnCount: 38,
    freshnessLagSeconds: 45,
    lastUpdated: agoIso(2),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "tbl-customer-360",
    name: "customer_360",
    namespace: "core.customer",
    type: "table",
    layer: "gold",
    tier: "hot",
    classification: "confidential",
    owner: "Customer Analytics",
    domain: "Customer",
    description: "Curated single view of customer across products.",
    format: "MergeTree",
    engine: "hot-store",
    rows: 48_000_000,
    sizeBytes: 420 * 1024 ** 3,
    columnCount: 92,
    freshnessLagSeconds: 1800,
    lastUpdated: agoIso(30),
    health: "degraded",
    residency: "Jakarta (ID)",
  },
  {
    id: "ice-orders-history",
    name: "orders_history",
    namespace: "lake.sales",
    type: "iceberg-table",
    layer: "silver",
    tier: "cold",
    classification: "internal",
    owner: "Data Platform",
    domain: "Sales",
    description: "Historical orders beyond 90 days, open format with time travel.",
    format: "Iceberg / Parquet",
    engine: "federated-compute",
    rows: 18_400_000_000,
    sizeBytes: 64 * 1024 ** 4,
    columnCount: 24,
    freshnessLagSeconds: 6 * 3600,
    lastUpdated: agoIso(360),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "ice-customer-dim",
    name: "customer_dim",
    namespace: "lake.customer",
    type: "iceberg-table",
    layer: "gold",
    tier: "cold",
    classification: "confidential",
    owner: "Customer Analytics",
    domain: "Customer",
    description: "Slowly changing customer dimension shared across engines.",
    format: "Iceberg / Parquet",
    engine: "federated-compute",
    rows: 51_000_000,
    sizeBytes: 210 * 1024 ** 3,
    columnCount: 44,
    freshnessLagSeconds: 12 * 3600,
    lastUpdated: agoIso(720),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "vec-support-kb",
    name: "support_kb_chunks",
    namespace: "ai.support",
    type: "vector-dataset",
    layer: "semantic",
    tier: "ai",
    classification: "internal",
    owner: "AI Platform",
    domain: "Support",
    description: "Chunked and embedded support knowledge base for retrieval.",
    format: "Lance",
    engine: "ai-store",
    rows: 3_400_000,
    sizeBytes: 96 * 1024 ** 3,
    columnCount: 12,
    freshnessLagSeconds: 55,
    lastUpdated: agoIso(1),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "vec-product-embeddings",
    name: "product_embeddings",
    namespace: "ai.commerce",
    type: "vector-dataset",
    layer: "semantic",
    tier: "ai",
    classification: "internal",
    owner: "AI Platform",
    domain: "Sales",
    description: "Product feature vectors for similarity and recommendations.",
    format: "Lance",
    engine: "ai-store",
    rows: 12_800_000,
    sizeBytes: 310 * 1024 ** 3,
    columnCount: 9,
    freshnessLagSeconds: 4 * 3600,
    lastUpdated: agoIso(240),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "ext-legacy-warehouse",
    name: "legacy_dwh.sales_facts",
    namespace: "federated.legacy",
    type: "external-source",
    layer: "gold",
    tier: "warm",
    classification: "internal",
    owner: "Enterprise DW",
    domain: "Sales",
    description: "Read-only bridge to the legacy warehouse estate.",
    format: "External (read-only)",
    engine: "federated-compute",
    rows: 6_200_000_000,
    sizeBytes: 18 * 1024 ** 4,
    columnCount: 61,
    freshnessLagSeconds: 24 * 3600,
    lastUpdated: agoIso(1440),
    health: "unknown",
    residency: "Singapore (SG)",
  },
  {
    id: "kn-credit-policy",
    name: "credit-policy-2026",
    namespace: "knowledge.risk",
    type: "knowledge-source",
    layer: "semantic",
    tier: "ai",
    classification: "confidential",
    owner: "Risk Office",
    domain: "Risk",
    description: "Credit policy handbook parsed into governed agent memory.",
    format: "Documents → Lance",
    engine: "ai-store",
    rows: 41_000,
    sizeBytes: 1.2 * 1024 ** 3,
    columnCount: 8,
    freshnessLagSeconds: 30,
    lastUpdated: agoIso(1),
    health: "healthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "tbl-inventory-snapshot",
    name: "inventory_snapshot",
    namespace: "core.supply",
    type: "table",
    layer: "bronze",
    tier: "warm",
    classification: "internal",
    owner: "Supply Chain Data",
    domain: "Supply chain",
    description: "Hourly ERP inventory snapshots, object-backed storage.",
    format: "MergeTree (object-backed)",
    engine: "hot-store",
    rows: 500_000,
    sizeBytes: 300 * 1024 ** 2,
    columnCount: 14,
    freshnessLagSeconds: 2 * 3600,
    lastUpdated: agoIso(120),
    health: "unhealthy",
    residency: "Jakarta (ID)",
  },
  {
    id: "tbl-restricted-ledger",
    name: "regulated_ledger",
    namespace: "core.regulated",
    type: "table",
    layer: "gold",
    tier: "hot",
    classification: "restricted",
    owner: "Compliance",
    domain: "Finance",
    description: "Regulated ledger; access requires compliance clearance.",
    format: "MergeTree",
    engine: "hot-store",
    rows: 120_000_000,
    sizeBytes: 340 * 1024 ** 3,
    columnCount: 27,
    freshnessLagSeconds: 60,
    lastUpdated: agoIso(3),
    health: "healthy",
    residency: "Jakarta (ID) — on-premise only",
  },
]

const NAMESPACES: CatalogNamespace[] = [
  {
    id: "ns-core-sales",
    name: "core.sales",
    description: "Hot analytical sales tables of record.",
    assetCount: 34,
    owner: "Data Platform",
    residency: "Jakarta (ID)",
    sourceEngine: "Hot analytical store",
  },
  {
    id: "ns-core-finance",
    name: "core.finance",
    description: "Payments and finance curated tables.",
    assetCount: 28,
    owner: "Finance Data",
    residency: "Jakarta (ID)",
    sourceEngine: "Hot analytical store",
  },
  {
    id: "ns-lake-sales",
    name: "lake.sales",
    description: "Open-format historical sales data with snapshots.",
    assetCount: 19,
    owner: "Data Platform",
    residency: "Jakarta (ID)",
    sourceEngine: "Open lake catalog",
  },
  {
    id: "ns-ai-support",
    name: "ai.support",
    description: "Vector datasets and knowledge chunks for support AI.",
    assetCount: 11,
    owner: "AI Platform",
    residency: "Jakarta (ID)",
    sourceEngine: "AI retrieval store",
  },
  {
    id: "ns-federated-legacy",
    name: "federated.legacy",
    description: "Read-only bridges to existing external estates.",
    assetCount: 9,
    owner: "Enterprise DW",
    residency: "Singapore (SG)",
    sourceEngine: "External (federated)",
  },
]

/** Builds a full detail record for a base asset with deterministic extras. */
function buildDetail(asset: Asset): AssetDetail {
  const h = stableHash(asset.id)
  return {
    ...asset,
    schema: [
      { name: `${asset.name.slice(0, 3)}_id`, dataType: "UUID", description: "Primary identifier" },
      { name: "tenant_id", dataType: "String", description: "Owning tenant" },
      { name: "amount", dataType: "Decimal(18,2)", description: "Monetary amount", masked: asset.classification === "confidential" || asset.classification === "restricted", classification: asset.classification },
      { name: "status", dataType: "LowCardinality(String)", description: "Lifecycle status" },
      { name: "customer_email", dataType: "String", description: "Contact email", masked: asset.classification !== "public" && asset.classification !== "internal", classification: asset.classification },
      { name: "created_at", dataType: "DateTime64(3)", description: "Event time" },
      { name: "updated_at", dataType: "DateTime64(3)", description: "Last change time" },
    ],
    sample: [
      { id: `${1000 + (h % 900)}`, tenant: "nusantara-finance", amount: asset.classification === "confidential" || asset.classification === "restricted" ? "•••••" : "1,240,500.00", status: "settled", created: agoIso(60).slice(0, 16).replace("T", " ") },
      { id: `${2000 + (h % 700)}`, tenant: "archi-retail", amount: asset.classification === "confidential" || asset.classification === "restricted" ? "•••••" : "88,900.00", status: "pending", created: agoIso(95).slice(0, 16).replace("T", " ") },
      { id: `${3000 + (h % 500)}`, tenant: "borneo-logistics", amount: asset.classification === "confidential" || asset.classification === "restricted" ? "•••••" : "402,150.00", status: "settled", created: agoIso(130).slice(0, 16).replace("T", " ") },
    ],
    qualityChecks: [
      { id: "q1", name: "Primary key uniqueness", dimension: "Uniqueness", status: "passed", lastRun: agoIso(45) },
      { id: "q2", name: "Amount not null", dimension: "Completeness", status: asset.health === "degraded" ? "warning" : "passed", lastRun: agoIso(45) },
      { id: "q3", name: "Freshness within SLA", dimension: "Freshness", status: asset.freshnessLagSeconds > 3600 ? "failed" : "passed", lastRun: agoIso(15) },
    ],
    policySummary: [
      { id: "pol-1", name: "Tenant row isolation", effect: "Row filter by tenant_id" },
      { id: "pol-2", name: `${asset.classification} column masking`, effect: "Masks contact and amount columns for non-privileged roles" },
      { id: "pol-residency", name: "Residency", effect: `Scans restricted to ${asset.residency}` },
    ],
    usage: {
      queries7d: 400 + (h % 4200),
      users7d: 4 + (h % 38),
      avgLatencyMs: 120 + (h % 700),
    },
    recentQueries: [
      { id: "qh-1", sql: `SELECT status, count() FROM ${asset.name} GROUP BY status`, user: "Rina Wijaya", at: agoIso(35) },
      { id: "qh-2", sql: `SELECT * FROM ${asset.name} WHERE created_at > now() - INTERVAL 1 DAY LIMIT 100`, user: "Bayu Pratama", at: agoIso(120) },
    ],
    dependents: [
      { id: "pl-orders-rollup", name: "orders_hourly_rollup", kind: "Pipeline" },
      { id: "sq-1", name: "exec-revenue-dashboard", kind: "Dashboard query" },
      { id: "emp-collections", name: "collections-copilot", kind: "Agent (retrieval)" },
    ],
    changeHistory: [
      { id: "ch1", at: daysAgoIso(2), actor: "Bayu Pratama", summary: "Added column email_verified (Bool)" },
      { id: "ch2", at: daysAgoIso(9), actor: "schema-sync", summary: "Registered snapshot compaction" },
      { id: "ch3", at: daysAgoIso(21), actor: "Dewi Anggraini", summary: "Classification raised to " + asset.classification },
    ],
    snapshots: asset.type === "iceberg-table"
      ? [
          { id: "snap-9812", committedAt: agoIso(360), operation: "append", records: 1_240_000 },
          { id: "snap-9788", committedAt: daysAgoIso(1), operation: "append", records: 1_180_000 },
          { id: "snap-9714", committedAt: daysAgoIso(2), operation: "compaction", records: 0 },
        ]
      : [],
    schemaVersions: [
      { version: 3, at: daysAgoIso(2), change: "Added email_verified (Bool)" },
      { version: 2, at: daysAgoIso(30), change: "Widened amount to Decimal(18,2)" },
      { version: 1, at: daysAgoIso(120), change: "Initial registration" },
    ],
    upstream: asset.layer === "raw" ? [] : [
      { id: "tbl-orders-events", name: "orders_events" },
      { id: "ice-customer-dim", name: "customer_dim" },
    ],
    downstream: [
      { id: "tbl-customer-360", name: "customer_360" },
    ].filter((d) => d.id !== asset.id),
    lifecyclePolicy:
      asset.tier === "hot"
        ? "Hot 90 days → warm 275 days → cold export"
        : asset.tier === "warm"
          ? "Warm 275 days → cold export"
          : asset.tier === "cold"
            ? "Retained 7 years, snapshot reads"
            : "Rebuildable from lineage; refreshed incrementally",
  }
}

/** Mock adapter for Data Explorer, asset detail, and catalog namespaces. */
export const mockAssetService: AssetService = {
  listAssets(filter, signal) {
    return mockCall(() => {
      const q = filter.search?.trim().toLowerCase() ?? ""
      return ASSETS.filter((a) => {
        if (filter.tier && filter.tier !== "all" && a.tier !== filter.tier) return false
        if (filter.layer && filter.layer !== "all" && a.layer !== filter.layer) return false
        if (filter.type && filter.type !== "all" && a.type !== filter.type) return false
        if (
          filter.classification &&
          filter.classification !== "all" &&
          a.classification !== filter.classification
        )
          return false
        if (q) {
          const hay = `${a.name} ${a.namespace} ${a.domain} ${a.owner} ${a.description}`.toLowerCase()
          if (!hay.includes(q)) return false
        }
        return true
      })
    }, { signal })
  },
  getAsset(id, signal) {
    return mockCall(() => {
      if (id === "tbl-restricted-ledger") {
        throw new ServiceError(
          "permission_denied",
          "This asset requires compliance clearance."
        )
      }
      const asset = ASSETS.find((a) => a.id === id)
      if (!asset) {
        throw new ServiceError("not_found", `Asset "${id}" was not found.`)
      }
      return buildDetail(asset)
    }, { signal })
  },
  listNamespaces(signal) {
    return mockCall(() => NAMESPACES, { signal })
  },
}
