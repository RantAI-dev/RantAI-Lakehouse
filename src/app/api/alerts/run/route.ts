import { NextResponse } from "next/server";
import { runRules } from "@/services/clients/alert-store";

export const dynamic = "force-dynamic";
export const maxDuration = 60;

/**
 * Evaluasi + kirim alert/digest. Titik picu PENJADWALAN — panggil periodik dari
 * cron (kontainer cron Portainer, cron OS, atau scheduler eksternal), mis:
 *   * / 15 * * * *  curl -s http://HOST:3031/api/alerts/run
 * Query: ?id=<ruleId> untuk menjalankan/menguji satu rule saja.
 * Auth ringan opsional: set ALERTS_RUN_TOKEN → wajib header x-run-token / ?token.
 */
async function handle(req: Request) {
  const url = new URL(req.url);
  const need = process.env.ALERTS_RUN_TOKEN;
  if (need) {
    const got = req.headers.get("x-run-token") || url.searchParams.get("token");
    if (got !== need) return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }
  const only = url.searchParams.get("id") || undefined;
  try {
    const results = await runRules(only, req.signal);
    return NextResponse.json({ ran: results.length, results });
  } catch (e) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

export async function GET(req: Request) { return handle(req); }
export async function POST(req: Request) { return handle(req); }
