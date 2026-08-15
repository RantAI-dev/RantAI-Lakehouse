import { NextResponse } from "next/server";
import { chRows } from "@/services/clients/clickhouse";
import { listRuns, mapRunStatus } from "@/services/clients/dagster";

export const dynamic = "force-dynamic";

/**
 * Governance NYATA dari data lakehouse kita:
 *  - quality: _silver_meta.quality (hasil quality-gate) — real
 *  - audit: run Dagster (siapa/kapan jalan apa) — real
 *  - classification: tier katalog (primer/sekunder) — partial
 *  - residency: tenant/site (id-jakarta) — partial
 */
export async function GET(_req: Request, { params }: { params: Promise<{ kind: string }> }) {
  const { kind } = await params;
  try {
    if (kind === "quality") {
      // Ambil verdict TERBARU per (tabel, cek) dari quality-gate.
      const rows = await chRows<{ tabel: string; cek: string; verdict: string; nilai: string; at: string }>(
        `SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict,
                toString(argMax(nilai, dibuat_pada)) nilai,
                toString(max(dibuat_pada)) at
         FROM _silver_meta.quality GROUP BY tabel, cek ORDER BY tabel, cek LIMIT 500`,
      );
      const mapStatus = (v: string) => (v === "fail" ? "failed" : v === "warn" ? "warning" : "passed");
      const mapSev = (v: string) => (v === "fail" ? "high" : v === "warn" ? "medium" : "info");
      return NextResponse.json({
        quality: rows.map((r, i) => ({
          id: `q-${i}`,
          name: r.cek.startsWith("null_rate") ? `Konversi kolom ${r.cek.split(":")[1]}` : r.cek,
          asset: r.tabel,
          dimension: r.cek.startsWith("null_rate") ? "validity" : r.cek === "row_count" ? "completeness" : "accuracy",
          threshold: r.cek.startsWith("null_rate") ? "null <5%" : "row_count > 0 & tidak anjlok >50%",
          severity: mapSev(r.verdict),
          lastStatus: mapStatus(r.verdict),
          lastRunAt: r.at,
        })),
      });
    }

    if (kind === "audit") {
      const runs = await listRuns(undefined, 50);
      return NextResponse.json({
        audit: runs.map((r) => ({
          id: r.runId,
          at: r.startTime ? new Date(r.startTime * 1000).toISOString() : "",
          actor: "Dagster",
          actorKind: "service",
          tenant: "dispar-dki",
          action: `pipeline ${mapRunStatus(r.status)}: ${r.jobName}`,
          resource: r.jobName,
          outcome: r.status === "FAILURE" ? "error" : "success",
          policyDecision: "allow",
          obligations: [],
          engineCategory: "hot-store",
        })),
      });
    }

    if (kind === "classification") {
      const rows = await chRows<{ slug: string; title: string; tier: string }>(
        `SELECT slug, title, tier FROM lake.\`bronze_meta.dataset_catalog\`
         UNION ALL SELECT slug, title, tier FROM lake.\`bronze_meta_sec.dataset_catalog\` LIMIT 500`,
      );
      return NextResponse.json({
        classifications: rows.map((r) => ({
          id: `c-${r.slug}`,
          asset: r.title,
          classification: "internal",
          confidence: 1,
          reviewStatus: "auto",
        })),
      });
    }

    if (kind === "residency") {
      return NextResponse.json({
        residency: [
          {
            id: "res-dispar-dki",
            tenant: "dispar-dki",
            classification: "internal",
            approvedSites: ["Depok (187)"],
            crossSiteAllowed: false,
            allowedOutput: "on-premise DKI",
            violations7d: 0,
          },
        ],
      });
    }

    return NextResponse.json({ error: `kind tak dikenal: ${kind}` }, { status: 400 });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 503 });
  }
}
