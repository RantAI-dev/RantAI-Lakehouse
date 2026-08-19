import { NextResponse } from "next/server";
import { getBoard } from "@/services/clients/bi-store";
import { getEmbedSecret, signEmbed } from "@/services/clients/embed-jwt";

export const dynamic = "force-dynamic";

/**
 * Info signed-embed untuk pemilik dashboard (di dalam konsol): embedding secret,
 * status embed board, dan CONTOH JWT yang valid (exp +1 jam) agar UI bisa
 * menampilkan link preview & snippet. Secret ini kunci penandatangan — hanya
 * tampil di konsol (surface tepercaya), sama seperti "secret key" di admin
 * Metabase. Host memakainya untuk menandatangani JWT di server mereka.
 */
export async function GET(req: Request) {
  const id = new URL(req.url).searchParams.get("board") || "";
  if (!id || id === "default") return NextResponse.json({ error: "dashboard tidak valid" }, { status: 400 });
  try {
    const board = await getBoard(id);
    if (!board) return NextResponse.json({ error: "not_found" }, { status: 404 });
    const secret = await getEmbedSecret();
    const exp = Math.floor(Date.now() / 1000) + 3600;
    const sampleToken = signEmbed({ resource: { dashboard: id }, params: {}, exp }, secret);
    return NextResponse.json({ secret, enabled: !!board.embedEnabled, sampleToken });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}
