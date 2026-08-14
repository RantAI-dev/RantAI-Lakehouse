import type {
  OverviewService,
  OverviewSummary,
  ActivityItem,
} from "../contracts/overview";
import { mockOverviewService } from "../mock/overview";
import { ServiceError } from "../errors";

/**
 * OverviewService: getSummary + listActivity NYATA (agregat lakehouse).
 * Alerts (list/ack/resolve) masih mock — belum ada alert engine.
 */
export const clickhouseOverviewService: OverviewService = {
  async getSummary(signal) {
    const res = await fetch("/api/overview", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Overview gagal dimuat");
    return json as OverviewSummary;
  },
  async listActivity(signal) {
    const res = await fetch("/api/overview", { method: "POST", signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Activity gagal dimuat");
    return json.activity as ActivityItem[];
  },
  listAlerts: (s) => mockOverviewService.listAlerts(s),
  acknowledgeAlert: (id, s) => mockOverviewService.acknowledgeAlert(id, s),
  resolveAlert: (id, note, s) => mockOverviewService.resolveAlert(id, note, s),
};
