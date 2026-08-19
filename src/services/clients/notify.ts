/**
 * Pengiriman notifikasi (alert & digest) ke DUA channel:
 *  - webhook: POST JSON ke incoming-webhook (Slack/Discord/generic). URL
 *    ditempel user di config → self-contained, tanpa secret di server.
 *  - email: lewat SMTP (nodemailer). Aktif hanya bila env SMTP_* diisi;
 *    server TAK pernah menyimpan password di kode/DB.
 */

export type DeliverResult = { ok: boolean; error?: string };

/** POST ke webhook. Kirim `text` (Slack) + `content` (Discord) + payload kaya. */
export async function sendWebhook(url: string, title: string, text: string): Promise<DeliverResult> {
  if (!/^https?:\/\//i.test(url)) return { ok: false, error: "URL webhook tidak valid" };
  const body = {
    text: `*${title}*\n${text}`,           // Slack
    content: `**${title}**\n${text}`,      // Discord
    title, message: text,                   // generic
  };
  try {
    const res = await fetch(url, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
    });
    if (!res.ok) return { ok: false, error: `webhook HTTP ${res.status}` };
    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** Kirim email via SMTP env. Graceful bila SMTP belum dikonfigurasi. */
export async function sendEmail(to: string, subject: string, html: string): Promise<DeliverResult> {
  const host = process.env.SMTP_HOST;
  if (!host) return { ok: false, error: "SMTP belum dikonfigurasi (set SMTP_HOST/PORT/USER/PASS/FROM di env)" };
  if (!to || !to.includes("@")) return { ok: false, error: "alamat email tidak valid" };
  try {
    const nodemailer = (await import("nodemailer")).default;
    const port = Number(process.env.SMTP_PORT ?? 587);
    const transport = nodemailer.createTransport({
      host, port,
      secure: process.env.SMTP_SECURE === "true" || port === 465,
      auth: process.env.SMTP_USER ? { user: process.env.SMTP_USER, pass: process.env.SMTP_PASS ?? "" } : undefined,
    });
    await transport.sendMail({
      from: process.env.SMTP_FROM ?? process.env.SMTP_USER ?? "rantai-lake@localhost",
      to, subject, html,
    });
    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** Kirim sesuai channel. text = plain; untuk email dibungkus HTML sederhana. */
export async function deliver(channel: string, target: string, title: string, text: string): Promise<DeliverResult> {
  if (channel === "webhook") return sendWebhook(target, title, text);
  if (channel === "email") {
    const html = `<div style="font-family:system-ui,sans-serif"><h3 style="margin:0 0 8px">${title}</h3><pre style="white-space:pre-wrap;font:inherit;margin:0">${text}</pre><p style="color:#888;font-size:12px;margin-top:16px">Rantai Lake — Enterprise Lakehouse Console</p></div>`;
    return sendEmail(target, title, text ? html : html);
  }
  return { ok: false, error: `channel tak dikenal: ${channel}` };
}
