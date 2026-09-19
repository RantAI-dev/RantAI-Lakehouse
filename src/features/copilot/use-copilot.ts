"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import type { ToolStep } from "./tool-step";
import { ALL_CAP_KEYS, capsForMode, toolsFromCaps } from "./capabilities";
import { derivePageContext, type PageContext } from "./page-context";
import { CopilotConfirmWriteDialog } from "./confirm-write-dialog";
import { apiFetch } from "@/services/http";

export type Mode = "ask" | "build";
export type DockPosition = "bottom" | "right";
export type Msg = {
  role: "user" | "assistant";
  content: string;
  tools?: ToolStep[];
  buildRunId?: string;
  chartCreated?: boolean;
};
export type SessionMeta = { id: string; title: string; mode: string; updatedAt?: string };

/**
 * Otak AI Copilot yang DIPAKAI BERSAMA (lewat context) oleh chat dock global,
 * halaman /copilot, DAN daftar riwayat di sidebar. Mengurus percakapan (mode
 * Ask/Build, kirim, tool loop) DAN riwayat (simpan/muat sesi dari
 * console.chat_session). Satu instance → semua tampilan konsisten.
 */
/** A copy of `messages` with one tool step swapped out; `null` if it no longer exists. */
function replaceToolStep(
  messages: Msg[],
  messageIndex: number,
  stepIndex: number,
  change: (msg: Msg, step: ToolStep) => { step: ToolStep; chartCreated: boolean },
): Msg[] | null {
  const msg = messages[messageIndex];
  const step = msg?.tools?.[stepIndex];
  if (!msg?.tools || !step) return null;
  const next = change(msg, step);
  const tools = msg.tools.slice();
  tools[stepIndex] = next.step;
  const copy = messages.slice();
  copy[messageIndex] = { ...msg, tools, chartCreated: next.chartCreated };
  return copy;
}

function useCopilotState() {
  const [mode, setMode] = React.useState<Mode>("ask");
  const [messages, setMessages] = React.useState<Msg[]>([]);
  // Confirm/Cancel resolve after an `await`; they read the latest list from
  // here so the copy they persist is the same one they render.
  const messagesRef = React.useRef(messages);
  messagesRef.current = messages;
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [sessionId, setSessionId] = React.useState<string | null>(null);
  const [sessions, setSessions] = React.useState<SessionMeta[]>([]);
  const [confirmingKey, setConfirmingKey] = React.useState<string | null>(null);
  const [enabledCaps, setEnabledCaps] = React.useState<Set<string>>(() => new Set(ALL_CAP_KEYS));
  const pathname = usePathname();
  const [pageOverride, setPageOverride] = React.useState<PageContext | null>(null);
  const pageContext = pageOverride ?? derivePageContext(pathname);
  const pageContextRef = React.useRef(pageContext);
  pageContextRef.current = pageContext;
  const setPageContext = React.useCallback((ctx: PageContext | null) => setPageOverride(ctx), []);

  const [dockPosition, setDockPositionState] = React.useState<DockPosition>("bottom");
  const [expanded, setExpandedState] = React.useState(false);
  const [sidebarWidth, setSidebarWidthState] = React.useState(420);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      const savedPos = window.localStorage.getItem("copilot-dock-pos");
      if (savedPos === "right" || savedPos === "bottom") setDockPositionState(savedPos);

      const savedExp = window.localStorage.getItem("copilot-dock-exp");
      if (savedExp === "1") setExpandedState(true);
      else if (savedExp === "0") setExpandedState(false);

      const savedWidth = Number.parseInt(window.localStorage.getItem("copilot-sidebar-width") || "420", 10);
      if (!Number.isNaN(savedWidth) && savedWidth >= 300 && savedWidth <= 900) {
        setSidebarWidthState(savedWidth);
      }
    } catch { /* ignore */ }
  }, []);

  const setDockPosition = React.useCallback((pos: DockPosition) => {
    setDockPositionState(pos);
    try { window.localStorage.setItem("copilot-dock-pos", pos); } catch { /* ignore */ }
  }, []);

  const setExpanded = React.useCallback((exp: boolean | ((prev: boolean) => boolean)) => {
    setExpandedState((prev) => {
      const next = typeof exp === "function" ? exp(prev) : exp;
      try { window.localStorage.setItem("copilot-dock-exp", next ? "1" : "0"); } catch { /* ignore */ }
      return next;
    });
  }, []);

  const setSidebarWidth = React.useCallback((width: number | ((prev: number) => number)) => {
    setSidebarWidthState((prev) => {
      const next = typeof width === "function" ? width(prev) : width;
      const clamped = Math.max(300, Math.min(850, next));
      try { window.localStorage.setItem("copilot-sidebar-width", String(clamped)); } catch { /* ignore */ }
      return clamped;
    });
  }, []);

  const toggleCap = React.useCallback((key: string) => {
    setEnabledCaps((prev) => {
      const n = new Set(prev);
      if (n.has(key)) n.delete(key); else n.add(key);
      return n;
    });
  }, []);

  const refreshSessions = React.useCallback(async () => {
    try {
      const res = await apiFetch("/api/ai/sessions", { cache: "no-store" });
      const json = await res.json();
      if (Array.isArray(json.sessions)) setSessions(json.sessions);
    } catch { /* abaikan */ }
  }, []);

  React.useEffect(() => { void refreshSessions(); }, [refreshSessions]);

  const persist = React.useCallback(async (msgs: Msg[], m: Mode, id: string | null) => {
    try {
      const res = await apiFetch("/api/ai/sessions", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: id ?? undefined, mode: m, messages: msgs }),
      });
      const json = await res.json();
      if (json.id) setSessionId(json.id);
      void refreshSessions();
    } catch { /* abaikan */ }
  }, [refreshSessions]);

  /**
   * Kapabilitas bertanda `write: true` yang sedang aktif.
   *
   * Tool loop dijalankan DI BACKEND (`POST /api/ai/chat` mengembalikan
   * `toolTrace` setelah semuanya selesai), jadi tidak ada titik di frontend
   * untuk menyela satu per satu tool sebelum dieksekusi. Gerbang konfirmasi
   * karena itu dipasang sebelum request dikirim: begitu ada kapabilitas
   * penulis yang aktif di mode Build, pengguna dimintai persetujuan lebih
   * dulu — sesuai rubrik "konfirmasi sebelum aksi yang mengubah atau
   * menghapus data".
   */
  const writeCaps = React.useMemo(
    () => capsForMode(mode).filter((c) => c.write && enabledCaps.has(c.key)),
    [mode, enabledCaps]
  );

  /** Pesan yang menunggu persetujuan; `null` berarti tidak ada. */
  const [pendingSend, setPendingSend] = React.useState<string | null>(null);

  const send = React.useCallback(async (text: string) => {
    const q = text.trim();
    if (!q || busy) return;
    setError(null);
    const next: Msg[] = [...messages, { role: "user", content: q }];
    setMessages(next);
    setBusy(true);
    try {
      const res = await apiFetch("/api/ai/chat", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          mode,
          tools: toolsFromCaps(enabledCaps, mode),
          context: pageContextRef.current?.system,
          messages: next.map((m) => ({ role: m.role, content: m.content })),
        }),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.hint ?? json?.detail ?? json?.error ?? "Copilot gagal");
      const isChartActuallyCreated = Boolean(
        json.chartCreated &&
          json.toolTrace?.some(
            (t: { tool: string; ok: boolean; result?: { created?: boolean } }) =>
              t.tool === "create_chart" && t.ok && Boolean(t.result?.created),
          ),
      );
      const full: Msg[] = [
        ...next,
        {
          role: "assistant",
          content: json.answer || "(no answer)",
          tools: json.toolTrace ?? [],
          buildRunId: json.buildRunId,
          chartCreated: isChartActuallyCreated,
        },
      ];
      setMessages(full);
      if (isChartActuallyCreated) {
        try {
          window.dispatchEvent(new Event("dashboards:changed"));
        } catch {
          /* ignore */
        }
      }
      void persist(full, mode, sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [busy, messages, mode, sessionId, persist, enabledCaps]);

  /**
   * Pintu masuk dari UI. Menahan pesan lebih dulu bila ada kapabilitas
   * penulis yang aktif, dan meneruskannya langsung bila tidak ada — jadi
   * mode Ask (yang tidak pernah punya kapabilitas `write`) tidak terganggu.
   */
  const requestSend = React.useCallback(
    (text: string) => {
      const q = text.trim();
      if (!q || busy) return;
      if (writeCaps.length > 0) {
        setPendingSend(q);
        return;
      }
      void send(q);
    },
    [busy, writeCaps, send]
  );

  /** Pengguna menyetujui; kirim pesan yang tertahan. */
  const confirmSend = React.useCallback(() => {
    const q = pendingSend;
    setPendingSend(null);
    if (q) void send(q);
  }, [pendingSend, send]);

  /** Pengguna membatalkan; buang pesan yang tertahan. */
  const cancelSend = React.useCallback(() => setPendingSend(null), []);

  /** Confirm/Cancel a `needs_confirmation` tool step (T0.4). */
  const confirmTool = React.useCallback(async (messageIndex: number, stepIndex: number) => {
    const step = messages[messageIndex]?.tools?.[stepIndex];
    if (!step) return;
    const pending = step.result as { tool?: string; args?: Record<string, unknown> } | undefined;
    const toolName = pending?.tool ?? step.tool;
    const key = `${messageIndex}:${stepIndex}`;
    setConfirmingKey(key);
    try {
      const rawArgs = { ...((pending?.args ?? step.args) as Record<string, unknown>) };
      if (!rawArgs.title && (rawArgs.caption || rawArgs.name || rawArgs.text)) {
        rawArgs.title = String(rawArgs.caption || rawArgs.name || rawArgs.text);
      }
      const res = await apiFetch("/api/ai/tool", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          tool: toolName,
          args: { ...rawArgs, confirmed: true },
          mode,
          sessionId: sessionId ?? undefined,
        }),
      });
      const json = await res.json();
      const toolOk = res.ok && json.outcome !== "failed" && json.outcome !== "refused";
      const created = Boolean(toolOk && (json.result?.created || json.created || toolName === "create_chart"));
      if (!toolOk || json.result?.error || json.error) {
        setError(String(json.result?.error || json.error || "Failed to execute tool"));
      }
      const updated = replaceToolStep(messagesRef.current, messageIndex, stepIndex, (msg, step) => ({
        step: { ...step, ok: toolOk, result: json.result ?? json },
        chartCreated: Boolean(msg.chartCreated || created),
      }));
      if (updated) {
        setMessages(updated);
        void persist(updated, mode, sessionId);
      }
      if (created) {
        try { window.dispatchEvent(new Event("dashboards:changed")); } catch { /* ignore */ }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setConfirmingKey((k) => (k === key ? null : k));
    }
  }, [messages, mode, sessionId, persist]);

  const cancelTool = React.useCallback((messageIndex: number, stepIndex: number) => {
    const updated = replaceToolStep(messagesRef.current, messageIndex, stepIndex, (_msg, step) => ({
      step: { ...step, ok: false, result: { cancelled: true } },
      chartCreated: false,
    }));
    if (updated) {
      setMessages(updated);
      void persist(updated, mode, sessionId);
    }
  }, [mode, sessionId, persist]);

  /**
   * Marks a pending tool step done without re-running it — the draft was
   * saved another way (the chart builder). Persisted like Confirm/Cancel.
   */
  const completeToolStep = React.useCallback(
    (messageIndex: number, stepIndex: number, result: Record<string, unknown>) => {
      const updated = replaceToolStep(messagesRef.current, messageIndex, stepIndex, (_msg, step) => ({
        step: { ...step, ok: true, result },
        chartCreated: true,
      }));
      if (updated) {
        setMessages(updated);
        void persist(updated, mode, sessionId);
      }
      try { window.dispatchEvent(new Event("dashboards:changed")); } catch { /* ignore */ }
    },
    [mode, sessionId, persist],
  );

  const newChat = React.useCallback(() => {
    setMessages([]); setSessionId(null); setError(null);
  }, []);

  const loadSession = React.useCallback(async (id: string) => {
    setError(null);
    try {
      const res = await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { cache: "no-store" });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Failed to load session");
      setMessages((json.session.messages ?? []) as Msg[]);
      if (json.session.mode === "build" || json.session.mode === "ask") setMode(json.session.mode);
      setSessionId(json.session.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const removeSession = React.useCallback(async (id: string) => {
    await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    if (id === sessionId) newChat();
    void refreshSessions();
  }, [sessionId, newChat, refreshSessions]);

  return {
    mode, setMode, messages, busy, error, sessionId, sessions,
    enabledCaps, toggleCap, pageContext, setPageContext,
    send, newChat, loadSession, removeSession, refreshSessions,
    writeCaps, pendingSend, requestSend, confirmSend, cancelSend,
    confirmTool, cancelTool, completeToolStep, confirmingKey,
    dockPosition, setDockPosition, expanded, setExpanded,
    sidebarWidth, setSidebarWidth,
  };
}

type CopilotValue = ReturnType<typeof useCopilotState>;
const CopilotContext = React.createContext<CopilotValue | null>(null);

/**
 * Provider tunggal — bungkus app agar dock/halaman/sidebar berbagi 1
 * percakapan. Dialog persetujuan aksi tulis ikut dipasang di sini supaya
 * berlaku untuk semua pintu masuk percakapan sekaligus.
 */
export function CopilotProvider({ children }: { children: React.ReactNode }) {
  const value = useCopilotState();
  return React.createElement(
    CopilotContext.Provider,
    { value },
    children,
    React.createElement(CopilotConfirmWriteDialog)
  );
}

/** Akses otak Copilot bersama. */
export function useCopilot(): CopilotValue {
  const ctx = React.useContext(CopilotContext);
  if (!ctx) throw new Error("useCopilot must be used within a CopilotProvider");
  return ctx;
}
