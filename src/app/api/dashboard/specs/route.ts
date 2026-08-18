import { NextResponse } from "next/server";
import {
  listStoredCharts,
  specFromInput,
  insertChart,
  deleteChart,
  type ChartInput,
} from "@/services/clients/bi-store";
import { toRenderSpec } from "@/lib/dashboard-specs";

export const dynamic = "force-dynamic";

/** Daftar chart tersimpan (render metadata). */
export async function GET() {
  try {
    const stored = await listStoredCharts();
    return NextResponse.json({ charts: stored.map((c) => toRenderSpec(c, c.source)) });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

/** Buat chart baru dari input tingkat-tinggi (dipakai UI builder & AI tool). */
export async function POST(req: Request) {
  let body: ChartInput;
  try {
    body = (await req.json()) as ChartInput;
  } catch {
    return NextResponse.json({ error: "body JSON tidak valid" }, { status: 400 });
  }
  try {
    const spec = await specFromInput(body, "ui", "ui");
    await insertChart(spec);
    return NextResponse.json({ ok: true, chart: toRenderSpec(spec, "ui") });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Edit chart tersimpan (pertahankan id). Body = {id, ...ChartInput}. */
export async function PUT(req: Request) {
  let body: ChartInput & { id?: string };
  try {
    body = (await req.json()) as ChartInput & { id?: string };
  } catch {
    return NextResponse.json({ error: "body JSON tidak valid" }, { status: 400 });
  }
  const id = String(body.id ?? "");
  if (!id) return NextResponse.json({ error: "id wajib untuk edit" }, { status: 400 });
  try {
    const spec = await specFromInput(body, "ui", "ui", id);
    await insertChart(spec);
    return NextResponse.json({ ok: true, chart: toRenderSpec(spec, "ui") });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Hapus chart tersimpan (?id=). Spec bawaan tak bisa dihapus. */
export async function DELETE(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  if (!id) return NextResponse.json({ error: "id wajib" }, { status: 400 });
  try {
    await deleteChart(id);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
