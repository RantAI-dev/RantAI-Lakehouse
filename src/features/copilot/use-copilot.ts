"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import type { ToolStep } from "./tool-step";
import { ALL_CAP_KEYS, toolsFromCaps } from "./capabilities";
import { derivePageContext, type PageContext } from "./page-context";
import { readNdjson } from "@/lib/ndjson";
import { notifyError, notifySuccess } from "@/lib/notify";
import { apiFetch } from "@/services/http";

export type Mode = "ask" | "build";
export type DockPosition = "bottom" | "right";
export type Msg = {
  /** Stable React key; older saved sessions get one when loaded. */
  id?: string;
  role: "user" | "assistant";
  content: string;
  /** The user stopped this answer before it arrived. */
  stopped?: boolean;
  tools?: ToolStep[];
  buildRunId?: string;
  chartCreated?: boolean;
};
export type SessionMeta = {
  id: string;
  title: string;
  mode: string;
  updatedAt?: string;
  chartCreated?: boolean;
  /** Last message's content, truncated server-side; still markdown. */
  preview?: string;
};

/** What Copilot is doing while an answer is on its way. */
export type ChatProgress = { phase: "thinking" | "tool"; tool?: string; startedAt: number };

function newMsgId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `m_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

/** The message a failed chat request should show, from its JSON error body. */
function chatErrorText(body: unknown): string {
  const b = (body ?? {}) as { error?: string; detail?: string; hint?: string };
  return b.detail ?? b.error ?? "Copilot couldn't answer. Try again.";
}

type ChatResult = {
  answer?: string;
  toolTrace?: ToolStep[];
  buildRunId?: string;
  chartCreated?: boolean;
};

/** How many recent sessions the header history menus list. */
const RECENT_SESSIONS = 20;

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
      const res = await apiFetch(`/api/ai/sessions?limit=${RECENT_SESSIONS}`, { cache: "no-store" });
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

  // Every tool that changes something is gated per call on the server
  // (`needs_confirmation` / approval), so messages are sent as typed; the
  // old ask-before-every-Build-message dialog only added a second prompt.
  const abortRef = React.useRef<AbortController | null>(null);
  // A request abandoned by New chat / switching conversation: its late
  // result must not land in whatever is open now.
  const discardedRef = React.useRef<AbortController | null>(null);
  const discardInFlight = React.useCallback(() => {
    discardedRef.current = abortRef.current;
    abortRef.current?.abort();
  }, []);
  const [progress, setProgress] = React.useState<ChatProgress | null>(null);

  /**
   * Sends `text` after `history` (default: the current conversation) and
   * streams Copilot's progress until the answer arrives.
   */
  const send = React.useCallback(async (text: string, history?: Msg[]) => {
    const q = text.trim();
    if (!q || busy) return;
    setError(null);
    const next: Msg[] = [...(history ?? messagesRef.current), { id: newMsgId(), role: "user", content: q }];
    setMessages(next);
    setBusy(true);
    setProgress({ phase: "thinking", startedAt: Date.now() });
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const res = await apiFetch("/api/ai/chat", {
        method: "POST", headers: { "Content-Type": "application/json" },
        signal: controller.signal,
        body: JSON.stringify({
          mode,
          stream: true,
          tools: toolsFromCaps(enabledCaps, mode),
          context: pageContextRef.current?.system,
          messages: next.filter((m) => m.content).map((m) => ({ role: m.role, content: m.content })),
        }),
      });
      if (!res.ok) throw new Error(chatErrorText(await res.json().catch(() => null)));

      let result: ChatResult | null = null;
      for await (const raw of readNdjson(res)) {
        const event = raw as { type?: string; tool?: string; body?: unknown };
        if (event.type === "status") {
          setProgress((p) => ({ phase: "thinking", startedAt: p?.startedAt ?? Date.now() }));
        } else if (event.type === "tool") {
          setProgress((p) => ({ phase: "tool", tool: event.tool, startedAt: p?.startedAt ?? Date.now() }));
        } else if (event.type === "done") {
          result = event.body as ChatResult;
        } else if (event.type === "error") {
          throw new Error(chatErrorText(event.body));
        }
      }
      if (!result) throw new Error("Copilot's answer was cut off. Try again.");

      const chartCreated = Boolean(
        result.chartCreated &&
          result.toolTrace?.some(
            (t) => t.tool === "create_chart" && t.ok && Boolean((t.result as { created?: boolean } | undefined)?.created),
          ),
      );
      const tools = result.toolTrace ?? [];
      const full: Msg[] = [
        ...next,
        {
          id: newMsgId(),
          role: "assistant",
          content: result.answer || (tools.length ? "" : "Copilot didn't return an answer."),
          tools,
          buildRunId: result.buildRunId,
          chartCreated,
        },
      ];
      setMessages(full);
      if (chartCreated) {
        try { window.dispatchEvent(new Event("dashboards:changed")); } catch { /* ignore */ }
      }
      void persist(full, mode, sessionId);
    } catch (e) {
      if (discardedRef.current === controller) return;
      if (controller.signal.aborted) {
        const stopped: Msg[] = [...next, { id: newMsgId(), role: "assistant", content: "", stopped: true }];
        setMessages(stopped);
        void persist(stopped, mode, sessionId);
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      abortRef.current = null;
      setBusy(false);
      setProgress(null);
    }
  }, [busy, mode, sessionId, persist, enabledCaps]);

  /** Stop waiting for the answer in flight; the server stops before its next step. */
  const stop = React.useCallback(() => abortRef.current?.abort(), []);

  /** Send the last question again, dropping the failed or stopped attempt. */
  const retry = React.useCallback(() => {
    const msgs = messagesRef.current;
    let last = msgs.length - 1;
    while (last >= 0 && msgs[last].role !== "user") last--;
    if (last < 0) return;
    void send(msgs[last].content, msgs.slice(0, last));
  }, [send]);

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
    discardInFlight();
    setMessages([]); setSessionId(null); setError(null);
  }, [discardInFlight]);

  const loadSession = React.useCallback(async (id: string) => {
    discardInFlight();
    setError(null);
    try {
      const res = await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { cache: "no-store" });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Failed to load session");
      setMessages(((json.session.messages ?? []) as Msg[]).map((m) => (m.id ? m : { ...m, id: newMsgId() })));
      if (json.session.mode === "build" || json.session.mode === "ask") setMode(json.session.mode);
      setSessionId(json.session.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [discardInFlight]);

  const removeSession = React.useCallback(async (id: string): Promise<boolean> => {
    try {
      const res = await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!res.ok) throw new Error((await res.json().catch(() => null))?.error ?? "Failed to delete");
    } catch (e) {
      notifyError("Couldn't delete conversation", e);
      return false;
    }
    if (id === sessionId) newChat();
    notifySuccess("Conversation deleted");
    void refreshSessions();
    return true;
  }, [sessionId, newChat, refreshSessions]);

  const renameSession = React.useCallback(async (id: string, title: string): Promise<boolean> => {
    try {
      const res = await apiFetch("/api/ai/sessions", {
        method: "PATCH", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id, title }),
      });
      if (!res.ok) throw new Error((await res.json().catch(() => null))?.error ?? "Failed to rename");
    } catch (e) {
      notifyError("Couldn't rename conversation", e);
      return false;
    }
    notifySuccess("Conversation renamed");
    void refreshSessions();
    return true;
  }, [refreshSessions]);

  return {
    mode, setMode, messages, busy, error, sessionId, sessions,
    enabledCaps, toggleCap, pageContext, setPageContext,
    send, newChat, loadSession, removeSession, renameSession, refreshSessions,
    progress, stop, retry,
    confirmTool, cancelTool, completeToolStep, confirmingKey,
    dockPosition, setDockPosition, expanded, setExpanded,
    sidebarWidth, setSidebarWidth,
  };
}

type CopilotValue = ReturnType<typeof useCopilotState>;
const CopilotContext = React.createContext<CopilotValue | null>(null);

/** Provider tunggal — bungkus app agar dock/halaman/sidebar berbagi 1 percakapan. */
export function CopilotProvider({ children }: { children: React.ReactNode }) {
  const value = useCopilotState();
  return React.createElement(CopilotContext.Provider, { value }, children);
}

/** Akses otak Copilot bersama. */
export function useCopilot(): CopilotValue {
  const ctx = React.useContext(CopilotContext);
  if (!ctx) throw new Error("useCopilot must be used within a CopilotProvider");
  return ctx;
}
