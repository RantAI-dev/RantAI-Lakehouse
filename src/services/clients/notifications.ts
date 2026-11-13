import type { Notifications, NotificationsService } from "../contracts/notifications";
import { apiFetch } from "../http";
import { ServiceError, deniedFor } from "../errors";

/**
 * `GET /api/notifications` (WS5 item F1). The `supported: false` branch is
 * a valid, successful response body (HTTP 200) — not a fetch error — so it
 * is returned as-is, never thrown.
 */
export const notificationsService: NotificationsService = {
  async list(signal) {
    const res = await apiFetch("/api/notifications", { signal });
    const json = await res.json();
    if (!res.ok) {
      // WEB-12: a refusal is a refusal, not the service being unavailable.
      const denied = deniedFor(res.status, json);
      if (denied) throw denied;
      throw new ServiceError("unavailable", json?.error ?? "Failed to load notifications");
    }
    return json as Notifications;
  },
  async markMentionsRead(board, signal) {
    const res = await apiFetch("/api/dashboard/comments/mentions/read", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      // `{}` marks every mention; a named board marks only its own (`BI-38` D5).
      body: JSON.stringify(board === undefined ? {} : { board }),
      signal,
    });
    if (!res.ok) {
      // WEB-12 (D5): a refusal names the permission; every other failure keeps
      // the fixed sentence the two callers treat as "still unread".
      const json = await res.json().catch(() => null);
      const denied = deniedFor(res.status, json);
      if (denied) throw denied;
      throw new ServiceError("unavailable", "The mentions could not be marked as read");
    }
  },
};
