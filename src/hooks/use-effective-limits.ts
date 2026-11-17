"use client"

import { useService } from "@/hooks/use-service"
import { settingsService } from "@/services"
import type { QueryLimitValues } from "@/services/contracts/settings"

/**
 * The query limits that apply to the signed-in person (`BI-5` plan D1, the
 * `effective` part of `GET /api/settings/limits`), or `null` while they load or
 * when they cannot be read. A caller that prints a number must print it only
 * when this is not `null`: a stale default in a sentence is a made-up limit
 * (principle 2).
 */
export function useEffectiveLimits(): QueryLimitValues | null {
  const state = useService((signal) => settingsService.getLimits(signal), [])
  return state.status === "success" ? state.data.effective : null
}
