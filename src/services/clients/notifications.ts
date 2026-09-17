import type { Notifications, NotificationsService } from "../contracts/notifications";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

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
      throw new ServiceError("unavailable", json?.error ?? "Failed to load notifications");
    }
    return json as Notifications;
  },
};
