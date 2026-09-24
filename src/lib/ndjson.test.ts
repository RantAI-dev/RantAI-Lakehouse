import { strict as assert } from "node:assert"
import { test } from "node:test"
import { readNdjson } from "./ndjson"

/** A response whose body arrives in the given chunks, split anywhere. */
function chunked(chunks: string[]): Response {
  const enc = new TextEncoder()
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const c of chunks) controller.enqueue(enc.encode(c))
        controller.close()
      },
    })
  )
}

async function collect(res: Response): Promise<unknown[]> {
  const out: unknown[] = []
  for await (const v of readNdjson(res)) out.push(v)
  return out
}

test("readNdjson menyatukan baris yang terpotong antar-chunk", async () => {
  const res = chunked(['{"type":"sta', 'tus"}\n{"type":"tool","tool":"run_sql"}\n', '{"type":"done"}'])
  assert.deepEqual(await collect(res), [
    { type: "status" },
    { type: "tool", tool: "run_sql" },
    { type: "done" },
  ])
})

test("readNdjson melewati baris kosong", async () => {
  assert.deepEqual(await collect(chunked(["\n{\"a\":1}\n\n"])), [{ a: 1 }])
})
