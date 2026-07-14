/**
 * Sample table detail data for the table browser — shared names and metadata.
 */
export const TABLE_DETAIL_BY_ID: Record<
  string,
  {
    name: string
    schema: { column: string; type: string; nullable: string; description: string }[]
    summary: { rows: string; size: string; columns: number; lastUpdate: string }
    metadata: { partitions: number; avgRowSize: string; format: string; compression: string }
    preview?: { col1: string; col2: string; col3: string; col4: string }[]
    formats?: string[]
  }
> = {
  "1": {
    name: "Customer_revenue",
    schema: [
      { column: "ID", type: "INT64", nullable: "No", description: "node2" },
      { column: "NAME", type: "STRING", nullable: "Yes", description: "node2" },
      { column: "USERNAME", type: "STRING", nullable: "No", description: "node2" },
      { column: "AGE", type: "INT", nullable: "No", description: "node2" },
      { column: "COST", type: "INT", nullable: "No", description: "node2" },
      { column: "REVENUE", type: "INT", nullable: "No", description: "node2" },
    ],
    summary: { rows: "125.000", size: "12 GB", columns: 8, lastUpdate: "5 Min ago" },
    metadata: { partitions: 12, avgRowSize: "18.4 kb", format: "Parquet", compression: "Snappy" },
    formats: ["Parquet", "Delta", "Iceberg"],
    preview: [
      { col1: "1042", col2: "PT Maju", col3: "4.2B", col4: "2026-04-10" },
      { col1: "881", col2: "Ana Wijaya", col3: "3.1B", col4: "2026-04-09" },
      { col1: "2201", col2: "Nexus Retail", col3: "1.8B", col4: "2026-04-09" },
    ],
  },
  "2": {
    name: "Customer_revenue",
    schema: [
      { column: "ID", type: "INT64", nullable: "No", description: "node2" },
      { column: "NAME", type: "STRING", nullable: "Yes", description: "node2" },
      { column: "REVENUE", type: "INT", nullable: "No", description: "node2" },
    ],
    summary: { rows: "2.8 M", size: "1.2 GB", columns: 6, lastUpdate: "1 Hours ago" },
    metadata: { partitions: 8, avgRowSize: "12.2 kb", format: "Parquet", compression: "Snappy" },
  },
  "3": { name: "Customer_revenue", schema: [], summary: { rows: "0", size: "0 B", columns: 0, lastUpdate: "-" }, metadata: { partitions: 0, avgRowSize: "-", format: "-", compression: "-" } },
  "4": { name: "Sales_analytics", schema: [], summary: { rows: "1.5 M", size: "800 MB", columns: 5, lastUpdate: "2 Hours ago" }, metadata: { partitions: 6, avgRowSize: "10 kb", format: "Parquet", compression: "Gzip" } },
  "5": { name: "Inventory_snapshot", schema: [], summary: { rows: "500 K", size: "300 MB", columns: 4, lastUpdate: "5 Hours ago" }, metadata: { partitions: 4, avgRowSize: "8 kb", format: "Parquet", compression: "Snappy" } },
  "6": { name: "Orders_fact", schema: [], summary: { rows: "5.1 M", size: "2.4 GB", columns: 10, lastUpdate: "30 Min ago" }, metadata: { partitions: 16, avgRowSize: "20 kb", format: "Parquet", compression: "Snappy" } },
  "7": { name: "Events_log", schema: [], summary: { rows: "12 M", size: "4.8 GB", columns: 7, lastUpdate: "15 Min ago" }, metadata: { partitions: 24, avgRowSize: "15 kb", format: "Parquet", compression: "Snappy" } },
  "8": { name: "Products_staging", schema: [], summary: { rows: "800 K", size: "450 MB", columns: 6, lastUpdate: "3 Hours ago" }, metadata: { partitions: 8, avgRowSize: "14 kb", format: "Parquet", compression: "Snappy" } },
  "9": { name: "Users_activity", schema: [], summary: { rows: "9.2 M", size: "3.1 GB", columns: 9, lastUpdate: "45 Min ago" }, metadata: { partitions: 20, avgRowSize: "16 kb", format: "Parquet", compression: "Snappy" } },
}

/** Default schema when a table has no column list in fixtures */
const DEFAULT_SCHEMA = [
  { column: "ID", type: "INT64", nullable: "No", description: "node2" },
  { column: "NAME", type: "STRING", nullable: "Yes", description: "node2" },
  { column: "USERNAME", type: "STRING", nullable: "No", description: "node2" },
  { column: "AGE", type: "INT", nullable: "No", description: "node2" },
  { column: "COST", type: "INT", nullable: "No", description: "node2" },
  { column: "REVENUE", type: "INT", nullable: "No", description: "node2" },
]

const DEFAULT_PREVIEW = [
  { col1: "1", col2: "Sample A", col3: "100", col4: "2026-04-01" },
  { col1: "2", col2: "Sample B", col3: "200", col4: "2026-04-02" },
]

/**
 * Returns table fixture data for the table detail page (`/tables/[id]`).
 *
 * Behavior:
 * - If the id is unknown, returns a generic placeholder named `Table_<id>` so the
 *   page can still render without a 404.
 * - If the entry exists but lacks a schema/preview/formats, common defaults are
 *   filled in to keep the UI populated.
 */
export function getTableDetail(id: string) {
  const row = TABLE_DETAIL_BY_ID[id]
  if (!row) {
    return {
      name: `Table_${id}`,
      schema: DEFAULT_SCHEMA,
      summary: { rows: "0", size: "0 B", columns: 0, lastUpdate: "-" },
      metadata: { partitions: 0, avgRowSize: "-", format: "-", compression: "-" },
      preview: DEFAULT_PREVIEW,
      formats: ["Parquet", "Delta", "Iceberg"],
    }
  }
  return {
    ...row,
    schema: row.schema.length > 0 ? row.schema : DEFAULT_SCHEMA,
    preview: row.preview ?? DEFAULT_PREVIEW,
    formats: row.formats ?? ["Parquet"],
  }
}
