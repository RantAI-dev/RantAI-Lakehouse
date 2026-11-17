import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  dropLegacyDraft,
  LEGACY_QUERY_DRAFT_KEY,
  parseDraft,
  parseTabs,
  queryDraftKey,
  readTabs,
  rememberSavedTab,
  writeTabs,
} from "./query-draft"
import type { QueryTab, QueryTabs } from "./query-draft"

/** The key of the person these tests write as: `u1`, no active tenant. */
const KEY = queryDraftKey("u1", null)

test("parseDraft mengembalikan null untuk isi yang tidak terpakai", () => {
  assert.equal(parseDraft(null), null)
  assert.equal(parseDraft("bukan json"), null)
  assert.equal(parseDraft(JSON.stringify({ sql: "   ", question: "" })), null)
})

test("parseDraft memulihkan draf yang berisi", () => {
  assert.deepEqual(
    parseDraft(JSON.stringify({ sql: "SELECT 1", question: "", tab: "sql" })),
    { sql: "SELECT 1", question: "", tab: "sql" }
  )
})

test("parseDraft memakai tab natural language bila nilainya tidak dikenal", () => {
  const draft = parseDraft(JSON.stringify({ sql: "", question: "berapa?", tab: "xyz" }))
  assert.equal(draft?.tab, "nl")
})

test("parseTabs memulihkan tab yang tersimpan dalam bentuk baru", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "b", name: "Users", sql: "SELECT 2", engine: "trino" },
      ],
      activeId: "b",
      question: "berapa?",
      tab: "sql",
    })
  )
  assert.deepEqual(tabs, {
    tabs: [
      { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse", parameters: [], values: {} },
      { id: "b", name: "Users", sql: "SELECT 2", engine: "trino", parameters: [], values: {} },
    ],
    activeId: "b",
    question: "berapa?",
    tab: "sql",
  })
})

test("parseTabs memulihkan tab yang menyimpan id kueri tersimpan dan levelnya", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        {
          id: "a",
          name: "Orders",
          sql: "SELECT 1",
          engine: "clickhouse",
          savedId: "q1",
          level: "edit",
        },
      ],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs, [
    {
      id: "a",
      name: "Orders",
      sql: "SELECT 1",
      engine: "clickhouse",
      savedId: "q1",
      level: "edit",
      parameters: [],
      values: {},
    },
  ])
})

test("parseTabs membaca tab yang tidak menyimpan id kueri tersimpan", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [{ id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" }],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs, [
    { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse", parameters: [], values: {} },
  ])
  assert.equal("savedId" in (tabs?.tabs[0] ?? {}), false)
})

test("parseTabs mengabaikan medan yang tidak dikenal", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        {
          id: "a",
          name: "Orders",
          sql: "SELECT 1",
          engine: "clickhouse",
          warna: "merah",
          savedId: 42,
        },
      ],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs, [
    { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse", parameters: [], values: {} },
  ])
})

test("parseTabs membuang entri yang tidak bisa dipakai", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" },
        { id: "", name: "Tanpa id", sql: "SELECT 2", engine: "clickhouse" },
        { id: "c", name: "Tanpa sql", engine: "clickhouse" },
        { id: "d", name: "Mesin lain", sql: "SELECT 3", engine: "postgres" },
        { id: "e", sql: "SELECT 4", engine: "clickhouse" },
        "bukan tab",
      ],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs, [
    { id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse", parameters: [], values: {} },
  ])
  assert.equal(tabs?.activeId, "a")
})

test("parseTabs membuang id yang kembar dan menyimpan yang pertama", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        { id: "a", name: "Pertama", sql: "SELECT 1", engine: "clickhouse" },
        { id: "a", name: "Kedua", sql: "SELECT 2", engine: "clickhouse" },
      ],
      activeId: "a",
    })
  )
  assert.equal(tabs?.tabs.length, 1)
  assert.equal(tabs?.tabs[0].name, "Pertama")
})

test("parseTabs memotong tab lebih dari 20 menjadi 20", () => {
  const stored = {
    tabs: Array.from({ length: 25 }, (_, index) => ({
      id: `t${index}`,
      name: `Query ${index}`,
      sql: `SELECT ${index}`,
      engine: "clickhouse",
    })),
    activeId: "t0",
  }
  const tabs = parseTabs(JSON.stringify(stored))
  assert.equal(tabs?.tabs.length, 20)
  assert.equal(tabs?.tabs[0].id, "t0")
  assert.equal(tabs?.tabs[19].id, "t19")
})

test("parseTabs memakai tab pertama ketika activeId tidak ada di dalam daftar", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [{ id: "a", name: "Orders", sql: "SELECT 1", engine: "clickhouse" }],
      activeId: "hilang",
    })
  )
  assert.equal(tabs?.activeId, "a")
})

test("parseTabs memindahkan draf lama menjadi satu tab", () => {
  const tabs = parseTabs(JSON.stringify({ sql: "SELECT 1", question: "berapa?", tab: "sql" }))
  assert.deepEqual(tabs, {
    tabs: [
      {
        id: "tab-1",
        name: "Query 1",
        sql: "SELECT 1",
        engine: "clickhouse",
        parameters: [],
        values: {},
      },
    ],
    activeId: "tab-1",
    question: "berapa?",
    tab: "sql",
  })
})

test("parseTabs mengembalikan null untuk isi yang bukan salah satu bentuk", () => {
  assert.equal(parseTabs(null), null)
  assert.equal(parseTabs("bukan json"), null)
  assert.equal(parseTabs(JSON.stringify({ sql: "   ", question: "" })), null)
  assert.equal(parseTabs(JSON.stringify({})), null)
  assert.equal(parseTabs(JSON.stringify({ tabs: [] })), null)
})

const PARAMETER_TAB: QueryTab = {
  id: "a",
  name: "Orders",
  sql: "SELECT * FROM t WHERE region = {{region}} AND day = {{day}}",
  engine: "clickhouse",
  parameters: [
    {
      name: "region",
      type: "dropdown",
      default: "eu",
      options: [
        { value: "eu", label: "Europe" },
        { value: "us", label: "United States" },
      ],
    },
    { name: "day", type: "date" },
  ],
  values: { region: "us", day: "2026-11-07" },
}

test("parseTabs restores the definitions and the values a tab kept", () => {
  const tabs = parseTabs(JSON.stringify({ tabs: [PARAMETER_TAB], activeId: "a" }))
  assert.deepEqual(tabs?.tabs, [PARAMETER_TAB])
})

test("parseTabs drops a malformed definition and a malformed value, never the tab", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [
        {
          ...PARAMETER_TAB,
          parameters: [
            ...PARAMETER_TAB.parameters,
            { name: "", type: "text" },
            { name: "kind", type: "list" },
            "not a definition",
          ],
          values: { ...PARAMETER_TAB.values, day: 20261107, junk: null },
        },
      ],
      activeId: "a",
    })
  )
  assert.equal(tabs?.tabs.length, 1)
  assert.deepEqual(tabs?.tabs[0].parameters, PARAMETER_TAB.parameters)
  assert.deepEqual(tabs?.tabs[0].values, { region: "us" })
})

test("parseTabs gives a tab stored with fields of the wrong kind no definitions and no values", () => {
  const tabs = parseTabs(
    JSON.stringify({
      tabs: [{ ...PARAMETER_TAB, parameters: "region", values: ["us"] }],
      activeId: "a",
    })
  )
  assert.deepEqual(tabs?.tabs[0].parameters, [])
  assert.deepEqual(tabs?.tabs[0].values, {})
})

test("writeTabs stores the definitions and the values, and readTabs gets them back", () => {
  window.localStorage.clear()
  try {
    const tabs = {
      tabs: [PARAMETER_TAB],
      activeId: "a",
      question: "",
      tab: "sql",
    } as QueryTabs
    writeTabs(KEY, tabs)
    assert.deepEqual(readTabs(KEY), tabs)
  } finally {
    window.localStorage.clear()
  }
})

// `WEB-7` D5: the save moved to its own page, so the tab is told afterwards
// which query a save produced, instead of the editor updating it in place.
test("rememberSavedTab points the tab that saved at the query it produced", () => {
  window.localStorage.clear()
  try {
    writeTabs(KEY, {
      tabs: [
        { ...PARAMETER_TAB, id: "a" },
        { ...PARAMETER_TAB, id: "b", savedId: "q1", level: "view" },
      ],
      activeId: "a",
      question: "",
      tab: "sql",
    })
    rememberSavedTab(KEY, "a", "q2", "edit")
    const saved = readTabs(KEY)
    assert.equal(saved?.tabs[0]?.savedId, "q2")
    assert.equal(saved?.tabs[0]?.level, "edit")
    // The tab that saved is the one that changes; another tab keeps its own.
    assert.equal(saved?.tabs[1]?.savedId, "q1")
    assert.equal(saved?.tabs[1]?.level, "view")
  } finally {
    window.localStorage.clear()
  }
})

test("rememberSavedTab changes nothing when the tab is not in the draft", () => {
  window.localStorage.clear()
  try {
    writeTabs(KEY, { tabs: [PARAMETER_TAB], activeId: "a", question: "", tab: "sql" })
    rememberSavedTab(KEY, "gone", "q2", "edit")
    assert.deepEqual(readTabs(KEY)?.tabs, [PARAMETER_TAB])
  } finally {
    window.localStorage.clear()
  }
})

test("rememberSavedTab writes nothing when the browser refuses storage", () => {
  window.localStorage.clear()
  const original = globalThis.localStorage.setItem
  try {
    Object.defineProperty(window.localStorage, "setItem", {
      configurable: true,
      value: () => {
        throw new Error("blocked")
      },
    })
    // It must not throw: a refused write leaves the editor's button as it was.
    rememberSavedTab(KEY, "a", "q2", "edit")
  } finally {
    Object.defineProperty(window.localStorage, "setItem", {
      configurable: true,
      value: original,
    })
    window.localStorage.clear()
  }
})

// `QS-8` (`LH-QS-1-04` F1): the draft belongs to one person in one tenant.

test("queryDraftKey gives each person and each tenant a key of their own, never the old shared key", () => {
  const first = queryDraftKey("u1", "t1")
  const second = queryDraftKey("u2", "t1")
  const otherTenant = queryDraftKey("u1", "t2")
  const noTenant = queryDraftKey("u1", null)

  assert.equal(first, `${LEGACY_QUERY_DRAFT_KEY}:u1:t1`)
  assert.equal(new Set([first, second, otherTenant, noTenant]).size, 4)
  assert.notEqual(noTenant, LEGACY_QUERY_DRAFT_KEY)
  assert.equal(queryDraftKey("u1", undefined), noTenant)
  assert.equal(queryDraftKey("u1", ""), noTenant)
})

test("readTabs reads only the tabs stored under the key it is given", () => {
  window.localStorage.clear()
  try {
    const mine = {
      tabs: [{ ...PARAMETER_TAB, id: "mine", sql: "SELECT 'mine'" }],
      activeId: "mine",
      question: "",
      tab: "sql",
    } as QueryTabs
    const theirs = {
      tabs: [{ ...PARAMETER_TAB, id: "theirs", sql: "SELECT 'theirs'" }],
      activeId: "theirs",
      question: "",
      tab: "sql",
    } as QueryTabs
    writeTabs(KEY, mine)
    writeTabs(queryDraftKey("u2", null), theirs)
    // The old shared entry holds tabs too; it is not one of the keys read.
    window.localStorage.setItem(
      LEGACY_QUERY_DRAFT_KEY,
      JSON.stringify({ ...theirs, tabs: [{ ...theirs.tabs[0], sql: "SELECT 'shared'" }] })
    )

    assert.deepEqual(readTabs(KEY), mine)
    assert.deepEqual(readTabs(queryDraftKey("u2", null)), theirs)
    assert.equal(readTabs(queryDraftKey("u3", null)), null)
  } finally {
    window.localStorage.clear()
  }
})

test("dropLegacyDraft removes the entry under the old shared key and leaves a person's own draft", () => {
  window.localStorage.clear()
  try {
    const mine = { tabs: [PARAMETER_TAB], activeId: "a", question: "", tab: "sql" } as QueryTabs
    writeTabs(KEY, mine)
    window.localStorage.setItem(LEGACY_QUERY_DRAFT_KEY, JSON.stringify(mine))

    dropLegacyDraft()

    assert.equal(window.localStorage.getItem(LEGACY_QUERY_DRAFT_KEY), null)
    assert.deepEqual(readTabs(KEY), mine)
    // A second call is harmless: the entry it removes is already gone.
    dropLegacyDraft()
  } finally {
    window.localStorage.clear()
  }
})
