import { NextResponse } from "next/server";
import { listSessions, getSession, saveSession, deleteSession, type StoredMessage } from "@/services/clients/chat-store";

export const dynamic = "force-dynamic";

/** GET (list) atau GET ?id= (satu sesi lengkap). */
export async function GET(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  try {
    if (id) {
      const session = await getSession(id);
      if (!session) return NextResponse.json({ error: "sesi tidak ditemukan" }, { status: 404 });
      return NextResponse.json({ session });
    }
    return NextResponse.json({ sessions: await listSessions() });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

/** Simpan sesi (buat/replace). Body {id?, mode, messages}. */
export async function POST(req: Request) {
  try {
    const body = (await req.json()) as { id?: string; mode?: string; messages?: StoredMessage[] };
    if (!Array.isArray(body.messages) || body.messages.length === 0) {
      return NextResponse.json({ error: "messages kosong" }, { status: 400 });
    }
    const res = await saveSession({ id: body.id, mode: body.mode ?? "ask", messages: body.messages });
    return NextResponse.json({ ok: true, ...res });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

export async function DELETE(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  if (!id) return NextResponse.json({ error: "id wajib" }, { status: 400 });
  try {
    await deleteSession(id);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
