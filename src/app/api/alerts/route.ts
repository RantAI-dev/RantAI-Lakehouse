import { NextResponse } from "next/server";
import { listRules, saveRule, deleteRule, type AlertRule } from "@/services/clients/alert-store";

export const dynamic = "force-dynamic";

/** Daftar alert & digest. */
export async function GET() {
  try {
    return NextResponse.json({ rules: await listRules() });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

/** Buat rule baru. */
export async function POST(req: Request) {
  try {
    const body = (await req.json()) as Partial<AlertRule>;
    return NextResponse.json({ ok: true, rule: await saveRule(body) });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Update rule (butuh id). */
export async function PUT(req: Request) {
  try {
    const body = (await req.json()) as Partial<AlertRule> & { id?: string };
    if (!body.id) return NextResponse.json({ error: "id wajib" }, { status: 400 });
    return NextResponse.json({ ok: true, rule: await saveRule(body, body.id) });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Hapus rule (?id=). */
export async function DELETE(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  if (!id) return NextResponse.json({ error: "id wajib" }, { status: 400 });
  try {
    await deleteRule(id);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
