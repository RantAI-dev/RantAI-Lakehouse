import { NextResponse } from "next/server";

export const dynamic = "force-dynamic";

const DAGSTER_URL = process.env.DAGSTER_URL ?? "http://192.168.18.187:13030/graphql";

/**
 * Status per-step sebuah run Dagster — untuk pohon pipeline live di AI Copilot
 * (Build). Dipanggil berkala oleh UI dengan ?runId=.
 */
export async function GET(req: Request) {
  const runId = new URL(req.url).searchParams.get("runId");
  if (!runId) return NextResponse.json({ error: "runId wajib" }, { status: 400 });

  const query = `query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename
    ... on Run { status stepStats { stepKey status } } } }`;
  try {
    const res = await fetch(DAGSTER_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query, variables: { rid: runId } }),
      cache: "no-store",
      signal: req.signal,
    });
    const json = await res.json();
    const run = json?.data?.pipelineRunOrError;
    if (!run || run.__typename !== "Run") {
      return NextResponse.json({ error: "run tidak ditemukan", status: "unknown", steps: [] }, { status: 404 });
    }
    const steps = (run.stepStats ?? []).map((s: { stepKey: string; status: string }) => ({
      key: s.stepKey,
      status: s.status,
    }));
    return NextResponse.json({ runId, status: run.status, steps });
  } catch (e) {
    return NextResponse.json({ error: String(e), status: "unknown", steps: [] }, { status: 503 });
  }
}
