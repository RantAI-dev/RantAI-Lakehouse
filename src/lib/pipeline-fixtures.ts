import type { PipelineItem, PipelineRunResult } from "@/types/pipeline"

const CUSTOMER_BRONZE_OUTPUT: PipelineRunResult = {
  runId: "run-9821",
  completedAt: "2026-05-02 09:40:02",
  outputTable: "bronze.customer_revenue_curated",
  rowsWritten: "124,982",
  duration: "3m 12s",
  preview: {
    columns: ["customer_id", "region", "revenue_usd", "ingest_date"],
    rows: [
      ["c-1001", "APAC", "1420.50", "2026-05-02"],
      ["c-1002", "EMEA", "883.10", "2026-05-02"],
      ["c-1003", "AMER", "210.00", "2026-05-02"],
      ["c-1004", "APAC", "3312.75", "2026-05-02"],
    ],
  },
}

const CUSTOMER_BRONZE_OUTPUT_ALT: PipelineRunResult = {
  ...CUSTOMER_BRONZE_OUTPUT,
  runId: "run-9814",
  completedAt: "2026-05-02 09:15:00",
  rowsWritten: "124,110",
  duration: "3m 05s",
}

const SALES_SILVER_LAST_SUCCESS: PipelineRunResult = {
  runId: "run-9701",
  completedAt: "2026-05-02 08:00:00",
  outputTable: "silver.sales_store_daily",
  rowsWritten: "78,400",
  duration: "12m 44s",
  previewNote:
    "Preview is from the last completed successful run. A new run is in progress.",
  preview: {
    columns: ["store_id", "day", "net_sales", "units_sold"],
    rows: [
      ["S-01", "2026-05-01", "48210.00", "920"],
      ["S-02", "2026-05-01", "33102.50", "604"],
      ["S-03", "2026-05-01", "12900.00", "210"],
    ],
  },
}

export const SAMPLE_PIPELINES: PipelineItem[] = [
  {
    id: "1",
    name: "Customer_revenue",
    description: "Customer data transformation pipeline",
    flow: { from: "Raw", to: "Bronze" },
    status: "Success",
    lastRun: "2 min ago",
    records: "125k record",
    schedule: "every 25 min",
    successRate: "99.9%",
    lastSuccessResult: CUSTOMER_BRONZE_OUTPUT,
  },
  {
    id: "2",
    name: "Customer_revenue",
    description: "Customer data transformation pipeline",
    flow: { from: "Raw", to: "Bronze" },
    status: "Success",
    lastRun: "2 min ago",
    records: "125k record",
    schedule: "every 25 min",
    successRate: "99.9%",
    lastSuccessResult: CUSTOMER_BRONZE_OUTPUT_ALT,
  },
  {
    id: "3",
    name: "Sales_analytics",
    description: "Sales aggregation and reporting pipeline",
    flow: { from: "Bronze", to: "Silver" },
    status: "Running",
    lastRun: "5 min ago",
    records: "80k record",
    schedule: "every 1 hour",
    successRate: "98.2%",
    lastSuccessResult: SALES_SILVER_LAST_SUCCESS,
  },
  {
    id: "4",
    name: "Inventory_snapshot",
    description: "Inventory sync and validation pipeline",
    flow: { from: "Raw", to: "Bronze" },
    status: "Failed",
    lastRun: "1 hour ago",
    records: "0 record",
    schedule: "every 6 hours",
    successRate: "94.1%",
  },
]

/**
 * Looks up a pipeline from the static `SAMPLE_PIPELINES` fixture by id.
 *
 * Used by the pipeline detail page as a fallback when no draft pipeline
 * has been persisted to `sessionStorage` yet.
 */
export function getStaticPipelineById(id: string): PipelineItem | undefined {
  return SAMPLE_PIPELINES.find((p) => p.id === id)
}
