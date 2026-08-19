import { NextResponse } from "next/server";
import {
  listBoards, createBoard, deleteBoard, renameBoard, updateBoardLayout, duplicateBoard,
  type LayoutMap,
} from "@/services/clients/bi-store";

export const dynamic = "force-dynamic";

/** Daftar dashboard (board bernama) + layout. */
export async function GET() {
  try {
    const boards = await listBoards();
    return NextResponse.json({ boards: [{ id: "default", name: "Utama", layout: {} }, ...boards] });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

/** Buat dashboard {name}, atau duplikat {duplicate: id}. */
export async function POST(req: Request) {
  try {
    const body = (await req.json()) as { name?: string; duplicate?: string };
    const board = body.duplicate ? await duplicateBoard(body.duplicate) : await createBoard(String(body.name ?? ""));
    return NextResponse.json({ ok: true, board });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Ubah dashboard: {id, name} rename, atau {id, layout} simpan tata letak. */
export async function PUT(req: Request) {
  try {
    const body = (await req.json()) as { id?: string; name?: string; layout?: LayoutMap };
    const id = String(body.id ?? "");
    if (!id || id === "default") return NextResponse.json({ error: "dashboard tidak valid" }, { status: 400 });
    if (typeof body.name === "string") await renameBoard(id, body.name);
    if (body.layout && typeof body.layout === "object") await updateBoardLayout(id, body.layout);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 400 });
  }
}

/** Hapus dashboard (?id=). Chart di dalamnya ikut terhapus. */
export async function DELETE(req: Request) {
  const id = new URL(req.url).searchParams.get("id");
  if (!id || id === "default") return NextResponse.json({ error: "dashboard tidak valid" }, { status: 400 });
  try {
    await deleteBoard(id);
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
