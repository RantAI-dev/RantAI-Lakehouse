import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test";
import * as React from "react";
import type { Msg } from "./use-copilot";

/**
 * `usePathname` lives in `next/navigation`; the hook reads it for the page
 * context. The happy-dom environment does not provide it, so the module is
 * stubbed before the hook is imported.
 */
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/main-board",
}));

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
    if (url.startsWith("/api/ai/sessions")) {
      const method = init?.method ?? "GET";
      const id = new URL(url, "http://test.invalid").searchParams.get("id");
      if (method === "POST") return fakeJson(200, sessionSave);
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
