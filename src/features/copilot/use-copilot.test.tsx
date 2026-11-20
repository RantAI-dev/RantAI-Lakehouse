import { act, render, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeEach, describe, expect, it, mock } from "bun:test";
import * as realNavigation from "next/navigation";
import * as React from "react";
import type { Msg } from "./use-copilot";

/**
 * `usePathname` lives in `next/navigation`; the hook reads it for the page
 * context. The happy-dom environment does not provide it, so the module is
 * stubbed before the hook is imported.
 */
// BI-62 PR B review SHOULD-FIX 1: `mock.module` outlives this file when CI runs
// every test in one bun process, so the override keeps the real exports (a
// later file importing `useRouter` must still find it) and `afterAll` puts the
// real module back.
const realNavigationModule = { ...realNavigation };
mock.module("next/navigation", () => ({
  ...realNavigationModule,
  usePathname: () => "/dashboards/main-board",
}));

afterAll(() => {
  mock.module("next/navigation", () => realNavigationModule);
});

const {
  CopilotProvider,
  useCopilot,
  useCopilotStream,
  continuationText,
  chatHistory,
  STOPPED_ANSWER_TEXT,
} = await import("./use-copilot");

/** A draft the model made and the person still has to confirm. */
type Draft = { tool: string; args: Record<string, unknown>; summary: string };

/** A step the model ran before its draft, when a case needs one. */
type ReadStep = { tool: string; args: Record<string, unknown>; result: Record<string, unknown> };

/** The draft the answer holds when a case names no other. */
const defaultDraft: Draft = {
  tool: "create_board",
  args: { name: "Overview" },
  summary: "Creates the board Overview.",
};

/** The requests the hook sent to the chat, so a test can read what it asked. */
const chatRequests: { body: string }[] = [];
/** What the tool route answers: a run that succeeded, or one that failed. */
let toolResult: Record<string, unknown> = { outcome: "ok", result: { created: true, id: "b1", name: "Overview" } };
/** What the tool route answers for one tool, when a case needs another payload. */
let toolResults: Record<string, Record<string, unknown>> = {};
/** A second draft in the same answer, when a case needs two steps to confirm. */
let extraDraft: Draft | null = null;
/** The drafts in the same answer, when a case needs other drafts than the default one. */
let drafts: Draft[] = [defaultDraft];
/** The steps the model ran before its draft, when a case needs them. */
let readSteps: ReadStep[] = [];
/** The steps the model says the request still has. */
let nextStep: string | null = "add two charts";
/** Whether the chat answer hangs until the person stops it. */
let slowChat = false;

/** A JSON response, for the session routes. */
function fakeJson(status: number, json: unknown): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => json,
  } as unknown as Response;
}

/** What `GET /api/ai/sessions?id=` answers, per id. */
let sessionOpens: Record<string, { status: number; json: unknown }> = {};
/** What the list and the writes answer. */
let sessionList: unknown = { sessions: [], hasMore: false, nextOffset: null, counts: {} };
let sessionSave: unknown = { ok: true, id: "s1", title: "A saved conversation" };
/** The bodies of the writes to /api/ai/sessions, so a test can read what was saved. */
const sessionSaveBodies: string[] = [];
/** What GET /api/agents/runs/{id} answers, per run id (AI-28). */
let runOpens: Record<string, unknown> = {};
/**
 * A controllable pause for the run lookups, so a test can change the chat
 * while they are in flight (AI-28 review R2).
 */
type RunGate = { promise: Promise<void>; release: () => void };
function deferredRunGate(): RunGate {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { promise, release };
}
let runGate: RunGate | null = null;
let sessionRename: unknown = null;
let sessionDeleteStatus = 200;
/** The ids `DELETE /api/ai/sessions?id=` was sent, in order. */
const deletedSessions: string[] = [];

/** A response the hook can read: the body as one chunk, and a JSON body. */
function fakeResponse(body: string, json: unknown): Response {
  const encoder = new TextEncoder();
  let sent = false;
  return {
    ok: true,
    status: 200,
    json: async () => json,
    body: {
      getReader: () => ({
        read: async () => {
          if (sent) return { done: true, value: undefined };
          sent = true;
          return { done: false, value: encoder.encode(body) };
        },
      }),
    },
  } as unknown as Response;
}

/** A chat answer that has sent nothing and rejects when the person stops it. */
function hangingResponse(signal?: AbortSignal): Response {
  return {
    ok: true,
    status: 200,
    json: async () => ({}),
    body: {
      getReader: () => ({
        read: () =>
          new Promise((_resolve, reject) => {
            signal?.addEventListener("abort", () =>
              reject(new DOMException("Aborted", "AbortError")),
            );
          }),
      }),
    },
  } as unknown as Response;
}

/**
 * The chat's answer: the steps the model ran before its draft, a draft that
 * waits for a Confirm, and the steps the request still has. `extraDraft`
 * adds a second draft to the same answer.
 */
function chatBody(): string {
  const list = [...drafts];
  if (extraDraft) list.push(extraDraft);
  const body: Record<string, unknown> = {
    answer: "The board draft is ready.",
    toolTrace: [
      ...readSteps.map((s) => ({ tool: s.tool, args: s.args, ok: true, result: s.result })),
      ...list.map((d) => ({
        tool: d.tool,
        args: d.args,
        ok: true,
        result: {
          needs_confirmation: true,
          tool: d.tool,
          args: d.args,
          summary: d.summary,
        },
      })),
    ],
  };
  if (nextStep) body.nextStep = nextStep;
  return `${JSON.stringify({ type: "done", body })}\n`;
}

const originalFetch = globalThis.fetch;

beforeEach(() => {
  chatRequests.length = 0;
  toolResult = { outcome: "ok", result: { created: true, id: "b1", name: "Overview" } };
  toolResults = {};
  extraDraft = null;
  drafts = [defaultDraft];
  readSteps = [];
  nextStep = "add two charts";
  slowChat = false;
  sessionOpens = {};
  sessionList = { sessions: [], hasMore: false, nextOffset: null, counts: {} };
  sessionSave = { ok: true, id: "s1", title: "A saved conversation" };
  sessionSaveBodies.length = 0;
  runOpens = {};
  runGate = null;
  sessionRename = null;
  sessionDeleteStatus = 200;
  deletedSessions.length = 0;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : String(input);
    const body = typeof init?.body === "string" ? init.body : "";
    if (url.startsWith("/api/ai/chat")) {
      chatRequests.push({ body });
      if (slowChat) return hangingResponse(init?.signal ?? undefined);
      return fakeResponse(chatBody(), {});
    }
    if (url === "/api/ai/tool") {
      const sent = body ? (JSON.parse(body) as { tool?: string }) : {};
      return fakeResponse("{}", toolResults[sent.tool ?? ""] ?? toolResult);
    }
    if (url.startsWith("/api/agents/runs/")) {
      const runId = url.slice("/api/agents/runs/".length);
      if (runGate) await runGate.promise;
      // The real getRun body always carries the run id and its approvals;
      // matching the step by those ids is what the lookup does (AI-28 R2).
      return fakeJson(200, {
        id: runId,
        approvals: [],
        ...(runOpens[runId] ?? { status: "succeeded" }),
      });
    }
    if (url.startsWith("/api/ai/sessions")) {
      const method = init?.method ?? "GET";
      const id = new URL(url, "http://test.invalid").searchParams.get("id");
      if (method === "POST") {
        sessionSaveBodies.push(body);
        return fakeJson(200, sessionSave);
      }
      if (method === "PATCH") {
        const sent = body ? (JSON.parse(body) as { title?: string }) : {};
        return fakeJson(200, sessionRename ?? { ok: true, id, title: sent.title });
      }
      if (method === "DELETE") {
        if (id) deletedSessions.push(id);
        return fakeJson(sessionDeleteStatus, { ok: sessionDeleteStatus < 300 });
      }
      if (id) {
        const answer = sessionOpens[id] ?? {
          status: 200,
          json: { session: { id, title: "Untitled", mode: "ask", messages: [] } },
        };
        return fakeJson(answer.status, answer.json);
      }
      return fakeJson(200, sessionList);
    }
    return fakeResponse("{}", {});
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

type Copilot = ReturnType<typeof useCopilot>;
let copilot: Copilot | null = null;

function Harness() {
  const api = useCopilot();
  React.useEffect(() => {
    copilot = api;
  }, [api]);
  return null;
}

/**
 * Renders the shared Copilot in Build mode. The hook's value is read from
 * `copilot`, which every render refreshes: the value the provider handed
 * out is a new object each time, so a copy taken here would be stale.
 */
function renderCopilot(): void {
  render(
    <CopilotProvider>
      <Harness />
    </CopilotProvider>,
  );
  act(() => {
    copilot!.setMode("build");
  });
}

/** The messages of the chat request the hook sent. */
function sentMessages(index: number): { role: string; content: string }[] {
  return JSON.parse(chatRequests[index].body).messages;
}

describe("the dashboard on screen (AI-30)", () => {
  const view = { board: "b1", filters: [{ column: "region", values: ["West"] }] };

  it("sends the dashboard page's view with the chat body", async () => {
    renderCopilot();
    act(() => {
      copilot!.setPageContext({
        key: "dashboard-view",
        title: "Working on a dashboard",
        hint: "",
        suggest: { ask: [], build: [] },
        system: "dashboard",
        view,
      });
    });
    await act(async () => {
      await copilot!.send("Send this PDF.");
    });
    expect(JSON.parse(chatRequests[0].body).view).toEqual(view);
  });

  it("sends no view when the page did not set one", async () => {
    renderCopilot();
    await act(async () => {
      await copilot!.send("Send this PDF.");
    });
    expect(JSON.parse(chatRequests[0].body).view).toBeUndefined();
  });
});

describe("the Copilot goes on after the draft is confirmed (AI-15)", () => {
  it("sends the steps the request still has back as a message", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    expect(chatRequests.length).toBe(1);

    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)).toEqual({
      role: "user",
      content: 'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    });
  });

  it("shows the continuation in the chat as a message of the person", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });
    await waitFor(() => expect(copilot!.messages.length).toBe(4));

    const typed = copilot!.messages.filter((m) => m.role === "user");
    expect(typed.at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    );
  });

  it("sends nothing when the draft is cancelled", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      copilot!.cancelTool(1, 0);
    });

    expect(chatRequests.length).toBe(1);
  });

  it("sends nothing when the confirm fails", async () => {
    toolResult = { outcome: "failed", result: { error: "The board could not be created." } };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(1);
  });

  it("sends at most three continuations after one typed message", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The typed message, then the answer with the draft; every continuation
    // adds the same two.
    const expected = [4, 6, 8, 8];
    for (const [round, index] of [1, 3, 5, 7].entries()) {
      await act(async () => {
        await copilot!.confirmTool(index, 0);
      });
      await waitFor(() => expect(copilot!.messages.length).toBe(expected[round]));
    }

    expect(chatRequests.length).toBe(4);
    expect(sentMessages(3).at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    );
  });

  it("names every object the person confirmed, in the order of the steps", async () => {
    extraDraft = {
      tool: "create_chart",
      args: { title: "Visitors", kind: "bar" },
      summary: "Creates the chart Visitors.",
    };
    toolResults = {
      create_chart: { outcome: "ok", result: { created: true, id: "c1", title: "Visitors" } },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The board is confirmed first; the chart still waits for its own
    // Confirm, so the request does not go on yet.
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });
    expect(chatRequests.length).toBe(1);

    await act(async () => {
      await copilot!.confirmTool(1, 1);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview"; create_chart c1 "Visitors". Do not draft these again. Continue: add two charts',
    );
  });

  it("names only the step the person confirmed, not the steps that only read", async () => {
    readSteps = [
      { tool: "list_boards", args: {}, result: { boards: [{ id: "b1" }] } },
      { tool: "describe_mart", args: { mart: "sales" }, result: { name: "sales" } },
    ];
    drafts = [
      { tool: "create_chart", args: { title: "Visitors", kind: "bar" }, summary: "Creates the chart Visitors." },
    ];
    toolResults = {
      create_chart: { outcome: "ok", result: { created: true, id: "c1", title: "Visitors" } },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The chart is the third step of the answer: the two reads came first.
    await act(async () => {
      await copilot!.confirmTool(1, 2);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      'Confirmed and saved: create_chart c1 "Visitors". Do not draft these again. Continue: add two charts',
    );
  });

  it("names a draft saved through the chart builder as well", async () => {
    extraDraft = {
      tool: "create_chart",
      args: { title: "Visitors", kind: "bar" },
      summary: "Creates the chart Visitors.",
    };
    toolResults = {
      create_chart: { outcome: "ok", result: { created: true, id: "c1", title: "Visitors" } },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The chart was saved through the chart builder, so no Confirm ran for
    // it; the board still waits for its own Confirm.
    await act(async () => {
      copilot!.completeToolStep(1, 1, { created: true, id: "c1", title: "Visitors" });
    });

    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview"; create_chart c1 "Visitors". Do not draft these again. Continue: add two charts',
    );
  });

  it("leaves out the id and the title the result does not carry", async () => {
    toolResult = { outcome: "ok", result: { created: true } };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      "Confirmed and saved: create_board. Do not draft these again. Continue: add two charts",
    );
  });

  it("names a calculated field by the id and the name under `field`", () => {
    const messages: Msg[] = [
      {
        role: "assistant",
        content: "",
        tools: [
          {
            tool: "create_calculated_field",
            args: {},
            ok: true,
            confirmed: true,
            result: { field: { id: "f1", name: "Margin" } },
          },
        ],
      },
    ];

    expect(continuationText(messages, 0, "add the chart")).toBe(
      'Confirmed and saved: create_calculated_field f1 "Margin". Do not draft these again. Continue: add the chart',
    );
  });
});

describe("the status line under a confirmed draft stays out of the chat (AI-6)", () => {
  const modelText = "The board draft is ready.";

  it("leaves the reply as the model wrote it and out of the next request the person types", async () => {
    nextStep = null;
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a board.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    // With no step left the console sends nothing by itself, and the stored
    // reply is the model's text alone.
    expect(chatRequests.length).toBe(1);
    expect(copilot!.messages[1].content).toBe(modelText);
    expect(copilot!.messages[1].tools?.[0]).toMatchObject({ ok: true, confirmed: true });

    await act(async () => {
      await copilot!.send("Thanks.");
    });

    const history = sentMessages(1);
    expect(history.map((m) => m.content)).toEqual(["Build me a board.", modelText, "Thanks."]);
    expect(history.some((m) => m.content.includes("Saved:"))).toBe(false);
  });

  it("leaves the reply as the model wrote it and out of the continuation the console sends", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(copilot!.messages[1].content).toBe(modelText);
    const history = sentMessages(1);
    expect(history[1]).toEqual({ role: "assistant", content: modelText });
    expect(history.some((m) => m.content.includes("Saved:"))).toBe(false);
    // The continuation names the object in its own words, not the line the
    // chat shows.
    expect(history.at(-1)?.content).toContain("Confirmed and saved:");
  });
});

describe("a stopped answer stays in the history (BI-41 D3)", () => {
  it("sends the fixed line between the stopped question and the next one", () => {
    const messages: Msg[] = [
      { role: "user", content: "How many events are there?" },
      { role: "assistant", content: "", stopped: true },
      { role: "user", content: "And how many visitors?" },
    ];

    expect(chatHistory(messages)).toEqual([
      { role: "user", content: "How many events are there?" },
      { role: "assistant", content: STOPPED_ANSWER_TEXT },
      { role: "user", content: "And how many visitors?" },
    ]);
  });

  it("sends a session saved before this change, with no line stored, the same way", () => {
    // An older saved session stores the stopped answer as empty content and
    // `stopped: true`, exactly as today; no migration is needed.
    const saved: Msg[] = [
      { role: "user", content: "How many events are there?" },
      { role: "assistant", content: "", stopped: true },
    ];

    expect(chatHistory(saved).at(-1)).toEqual({ role: "assistant", content: STOPPED_ANSWER_TEXT });
  });

  it("still drops empty messages that were not stopped", () => {
    const messages: Msg[] = [
      { role: "user", content: "Hi." },
      { role: "assistant", content: "" },
    ];

    expect(chatHistory(messages)).toEqual([{ role: "user", content: "Hi." }]);
  });

  it("leaves a continuation's history unchanged", () => {
    const continuation =
      'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts';
    const messages: Msg[] = [
      { role: "user", content: "Build me a dashboard about events." },
      { role: "assistant", content: "The board draft is ready." },
      { role: "user", content: continuation },
    ];

    expect(chatHistory(messages)).toEqual([
      { role: "user", content: "Build me a dashboard about events." },
      { role: "assistant", content: "The board draft is ready." },
      { role: "user", content: continuation },
    ]);
  });

  it("sends the line in the request after the person stops an answer", async () => {
    slowChat = true;
    renderCopilot();
    act(() => {
      void copilot!.send("How many events are there?");
    });
    await waitFor(() => expect(chatRequests.length).toBe(1));
    act(() => {
      copilot!.stop();
    });
    await waitFor(() => expect(copilot!.busy).toBe(false));
    expect(copilot!.messages.at(-1)?.stopped).toBe(true);

    slowChat = false;
    await act(async () => {
      await copilot!.send("And how many visitors?");
    });

    expect(sentMessages(1)).toEqual([
      { role: "user", content: "How many events are there?" },
      { role: "assistant", content: STOPPED_ANSWER_TEXT },
      { role: "user", content: "And how many visitors?" },
    ]);
  });
});

describe("the open conversation's title and its load errors (AI-24)", () => {
  const opened = (id: string, title: string): { status: number; json: unknown } => ({
    status: 200,
    json: { session: { id, title, mode: "ask", messages: [{ id: "u1", role: "user", content: "Revenue?" }] } },
  });

  it("shows the title of a conversation older than the newest 20", async () => {
    sessionList = {
      sessions: [{ id: "recent", title: "A recent one", mode: "ask" }],
      hasMore: true,
      nextOffset: 20,
      counts: {},
    };
    sessionOpens = { old: opened("old", "Quarterly revenue") };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("old");
    });

    expect(copilot!.sessionId).toBe("old");
    expect(copilot!.sessionTitle).toBe("Quarterly revenue");
    expect(
      copilot!.sessions.some((s) => s.id === "old"),
      "the fixture is a conversation the recent list does not hold",
    ).toBe(false);
  });

  it("renames the open conversation without it being in the recent list", async () => {
    sessionOpens = { old: opened("old", "Quarterly revenue") };
    renderCopilot();
    await act(async () => {
      await copilot!.loadSession("old");
    });

    let renamed = false;
    await act(async () => {
      renamed = await copilot!.renameSession("old", "Board plan");
    });

    expect(renamed).toBe(true);
    expect(copilot!.sessionTitle).toBe("Board plan");
  });

  it("shows a plain sentence for a deleted conversation, never the API's text", async () => {
    sessionOpens = { gone: { status: 404, json: { error: "sesi tidak ditemukan" } } };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("gone");
    });

    expect(copilot!.error).toBe("This conversation could not be found. It may have been deleted.");
    expect(copilot!.error).not.toContain("sesi");
  });

  it("shows the other plain sentence when the conversation could not be loaded", async () => {
    sessionOpens = { broken: { status: 500, json: { error: "database error" } } };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("broken");
    });

    expect(copilot!.error).toBe("This conversation could not be loaded. Try again.");
    expect(copilot!.error).not.toContain("database");
  });

  it("starts a new chat when the open conversation is deleted", async () => {
    sessionOpens = { open: opened("open", "Open conversation") };
    renderCopilot();
    await act(async () => {
      await copilot!.loadSession("open");
    });

    let removed = false;
    await act(async () => {
      removed = await copilot!.removeSession("open");
    });

    expect(removed).toBe(true);
    expect(deletedSessions).toEqual(["open"]);
    expect(copilot!.sessionId).toBeNull();
    expect(copilot!.sessionTitle).toBeNull();
    expect(copilot!.messages).toEqual([]);
  });
});

/** A chat stream the test pushes events into, one macrotask apart. */
function controlledChatStream() {
  const encoder = new TextEncoder();
  let push!: (chunk: Uint8Array) => void;
  let close!: () => void;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      push = (chunk) => controller.enqueue(chunk);
      close = () => controller.close();
    },
  });
  return {
    response: { ok: true, status: 200, json: async () => ({}), body } as unknown as Response,
    async send(line: string) {
      push(encoder.encode(`${line}\n`));
      await new Promise((resolve) => setTimeout(resolve, 0));
    },
    end() {
      close();
    },
  };
}

describe("streaming does not re-render the whole Copilot (AI-24 D6)", () => {
  it("leaves a useCopilot() consumer alone while 50 deltas arrive", async () => {
    const stream = controlledChatStream();
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : String(input);
      if (url.startsWith("/api/ai/chat")) return stream.response;
      if (url.startsWith("/api/ai/sessions")) {
        return fakeJson(200, init?.method === "POST" ? sessionSave : sessionList);
      }
      return fakeJson(200, {});
    }) as typeof fetch;

    // Counted in an effect, after each commit: a memory write during render
    // would itself be what the count is meant to catch.
    const mainRenders = { current: 0 };
    const streamRenders = { current: 0 };
    function Main() {
      const api = useCopilot();
      React.useEffect(() => {
        copilot = api;
        mainRenders.current += 1;
      });
      return null;
    }
    function StreamConsumer() {
      useCopilotStream();
      React.useEffect(() => {
        streamRenders.current += 1;
      });
      return null;
    }

    render(
      <CopilotProvider>
        <Main />
        <StreamConsumer />
      </CopilotProvider>,
    );

    let sending: Promise<void> = Promise.resolve();
    act(() => {
      sending = copilot!.send("Show me revenue");
    });
    await act(async () => {
      await stream.send(JSON.stringify({ type: "delta", text: "The " }));
    });

    const mainBefore = mainRenders.current;
    const streamBefore = streamRenders.current;
    await act(async () => {
      for (let i = 0; i < 50; i += 1) {
        await stream.send(JSON.stringify({ type: "delta", text: `piece ${i} ` }));
      }
    });

    expect(streamRenders.current).toBeGreaterThan(streamBefore);
    expect(mainRenders.current).toBe(mainBefore);

    await act(async () => {
      await stream.send(JSON.stringify({ type: "done", body: { answer: "The answer." } }));
      stream.end();
      await sending;
    });
  });
});

describe("one message in another mode (BI-62 D8)", () => {
  it("asks in Ask mode for that message and leaves the conversation in Build", async () => {
    renderCopilot();
    await act(async () => {
      await copilot!.send("Write for this report: a short summary", undefined, { mode: "ask" });
    });
    const sent = JSON.parse(chatRequests[0].body) as { mode: string; tools: string[] };
    expect(sent.mode).toBe("ask");
    // Ask mode's tools, not Build's.
    const { ALL_CAP_KEYS, toolsFromCaps } = await import("./capabilities");
    expect(sent.tools).toEqual(toolsFromCaps(new Set(ALL_CAP_KEYS), "ask"));
    expect(sent.tools).not.toEqual(toolsFromCaps(new Set(ALL_CAP_KEYS), "build"));
    expect(copilot!.mode).toBe("build");
    // The next message without a mode is Build again.
    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    expect((JSON.parse(chatRequests[1].body) as { mode: string }).mode).toBe("build");
  });
});

describe("a reopened conversation shows what happened to its approvals (AI-28, LH-AI-18-09)", () => {
  it("reads the run on open and never saves the state back into the chat", async () => {
    sessionOpens = {
      appr: {
        status: 200,
        json: {
          session: {
            id: "appr",
            title: "Delete a chart",
            mode: "build",
            messages: [
              {
                id: "m1",
                role: "assistant",
                content: "The action needs approval.",
                tools: [
                  {
                    tool: "delete_chart",
                    args: { id: "c1" },
                    ok: true,
                    result: {
                      needs_approval: true,
                      approval_id: "appr-1",
                      run_id: "r1",
                      summary: "Moves chart c1 from its dashboard to the trash.",
                    },
                  },
                ],
              },
            ],
          },
        },
      },
    };
    runOpens = { r1: { status: "succeeded" } };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("appr");
    });
    await waitFor(() => {
      expect(copilot!.messages[0]?.tools?.[0]?.approvalState).toBe("ran");
    });

    // The next save carries no approvalState: it is read from the run on
    // every open, not written into the conversation.
    await act(async () => {
      await copilot!.send("And now?");
    });
    expect(sessionSaveBodies.length).toBeGreaterThan(0);
    const saved = JSON.parse(sessionSaveBodies.at(-1)!) as {
      messages: { tools?: Record<string, unknown>[] }[];
    };
    expect(saved.messages[0].tools?.[0]?.approvalState).toBeUndefined();
  });

  it("uses the run's last step so an executed failure is not shown as not run (AI-28 review R1)", async () => {
    sessionOpens = {
      appr2: {
        status: 200,
        json: {
          session: {
            id: "appr2",
            title: "Delete a chart",
            mode: "build",
            messages: [
              {
                id: "m1",
                role: "assistant",
                content: "The action needs approval.",
                tools: [
                  {
                    tool: "delete_chart",
                    args: { id: "c1" },
                    ok: true,
                    result: {
                      needs_approval: true,
                      approval_id: "appr-2",
                      run_id: "r2",
                      summary: "Moves chart c1 from its dashboard to the trash.",
                    },
                  },
                ],
              },
            ],
          },
        },
      },
    };
    runOpens = {
      r2: {
        status: "failed",
        steps: [{ label: "Executed after approval", status: "failed", detail: "x" }],
      },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("appr2");
    });
    await waitFor(() => {
      expect(copilot!.messages[0]?.tools?.[0]?.approvalState).toBe("failed");
    });
  });

  it("keeps a message sent while the run lookups are in flight (AI-28 review R2)", async () => {
    runGate = deferredRunGate();
    sessionOpens = {
      appr3: {
        status: 200,
        json: {
          session: {
            id: "appr3",
            title: "Delete a chart",
            mode: "build",
            messages: [
              {
                id: "m1",
                role: "assistant",
                content: "The action needs approval.",
                tools: [
                  {
                    tool: "delete_chart",
                    args: { id: "c1" },
                    ok: true,
                    result: {
                      needs_approval: true,
                      approval_id: "appr-3",
                      run_id: "r3",
                      summary: "Moves chart c1 from its dashboard to the trash.",
                    },
                  },
                ],
              },
            ],
          },
        },
      },
    };
    runOpens = { r3: { status: "succeeded" } };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("appr3");
    });
    // Both the send and the lookup now read the same pre-render snapshot:
    // whichever writes last must not drop the other's update.
    await act(async () => {
      const sending = copilot!.send("And now?");
      runGate!.release();
      await sending;
    });
    await waitFor(() => {
      expect(
        copilot!.messages.some((m) => m.role === "user" && m.content === "And now?")
      ).toBe(true);
      expect(copilot!.messages[0]?.tools?.[0]?.approvalState).toBe("ran");
    });
  });

  it("tags the step by run id, not its position at load time (AI-28 review R2)", async () => {
    runGate = deferredRunGate();
    sessionOpens = {
      appr4: {
        status: 200,
        json: {
          session: {
            id: "appr4",
            title: "Two approvals",
            mode: "build",
            messages: [
              {
                id: "m1",
                role: "assistant",
                content: "First approval.",
                tools: [
                  {
                    tool: "delete_chart",
                    args: { id: "c1" },
                    ok: true,
                    result: {
                      needs_approval: true,
                      approval_id: "appr-4a",
                      run_id: "r4a",
                      summary: "First chart.",
                    },
                  },
                ],
              },
              {
                id: "m2",
                role: "assistant",
                content: "Second approval.",
                tools: [
                  {
                    tool: "delete_chart",
                    args: { id: "c2" },
                    ok: true,
                    result: {
                      needs_approval: true,
                      approval_id: "appr-4b",
                      run_id: "r4b",
                      summary: "Second chart.",
                    },
                  },
                ],
              },
            ],
          },
        },
      },
    };
    runOpens = {
      r4a: { status: "succeeded" },
      r4b: { status: "rejected" },
    };
    // Keep the open id across the delete's save: otherwise the lookup's
    // "same session still open" guard would skip, hiding what we test here.
    sessionSave = { ok: true, id: "appr4", title: "Two approvals" };
    renderCopilot();

    await act(async () => {
      await copilot!.loadSession("appr4");
    });
    // The first message (and its step) is gone before the lookups resolve;
    // index 0 now points at the second step, which must still get r4b's state.
    await act(async () => {
      copilot!.deleteMessage(0);
    });
    await act(async () => {
      runGate!.release();
    });
    await waitFor(() => {
      expect(copilot!.messages[0]?.tools?.[0]?.approvalState).toBe("rejected");
    });
  });
});
