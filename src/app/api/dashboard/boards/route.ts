import { NextResponse } from "next/server";
import { listBoards, createBoard, deleteBoard } from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

/** Daftar board (dashboard bernama). */
export async function GET() {
  try {
    const boards = await listBoards();
    return NextResponse.json({ boards: [{ id: "default", name: "Utama" }, ...boards] });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

/** Buat board baru. Body {name}. */
export async function POST(req: Request) {
  try {
    const { name } = (await req.json()) as { name?: string };
    const board = await createBoard(String(name ?? ""));
    return NextResponse.json({ ok: true, board });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Hapus board (?id=). Chart di dalamnya dikembalikan ke Utama. */
export async function DELETE(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  if (!id || id === "default") return NextResponse.json({ error: "board tidak valid" }, { status: 400 });
  try {
    await deleteBoard(id);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
