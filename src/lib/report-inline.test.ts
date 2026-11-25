import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  type InlineRun,
  escapeInline,
  inlineFromNodes,
  inlinePlainText,
  inlineToNodes,
  normalizeRuns,
  parseInline,
  serializeInline,
} from "./report-inline"
import { blocksForSave } from "./report-blocks"

test("parseInline reads bold, italic and a link inside a sentence", () => {
  assert.deepEqual(parseInline("Visits **rose** in *June*, see [the method](https://a.example/m)."), [
    { text: "Visits " },
    { text: "rose", bold: true },
    { text: " in " },
    { text: "June", italic: true },
    { text: ", see " },
    { text: "the method", href: "https://a.example/m" },
    { text: "." },
  ])
  assert.deepEqual(parseInline(""), [])
  assert.deepEqual(parseInline("plain <b>text</b>"), [{ text: "plain <b>text</b>" }])
})

test("parseInline nests marks: bold and italic together, marks inside a link, a link inside bold", () => {
  assert.deepEqual(parseInline("***both***"), [{ text: "both", bold: true, italic: true }])
  assert.deepEqual(parseInline("*a **b** c*"), [
    { text: "a ", italic: true },
    { text: "b", bold: true, italic: true },
    { text: " c", italic: true },
  ])
  assert.deepEqual(parseInline("[**bold** words](/dashboards/b_1)"), [
    { text: "bold", bold: true, href: "/dashboards/b_1" },
    { text: " words", href: "/dashboards/b_1" },
  ])
  assert.deepEqual(parseInline("**see [x](mailto:a@b.example)**"), [
    { text: "see ", bold: true },
    { text: "x", bold: true, href: "mailto:a@b.example" },
  ])
})

test("an unbalanced star is text, and the words after it keep no mark", () => {
  assert.deepEqual(parseInline("**not closed"), [{ text: "**not closed" }])
  assert.deepEqual(parseInline("2 * 3 = 6"), [{ text: "2 * 3 = 6" }])
  assert.deepEqual(parseInline("a **b *c"), [{ text: "a **b *c" }])
  assert.deepEqual(parseInline("****"), [{ text: "****" }])
  assert.deepEqual(parseInline("*one* and *"), [{ text: "one", italic: true }, { text: " and *" }])
})

test("a link that could run code is its words, never a link", () => {
  for (const bad of [
    "[x](javascript:alert(1))",
    "[x](JavaScript:alert(1))",
    "[x](data:text/html,<b>)",
    "[x](//elsewhere.example)",
    "[x](vbscript:x)",
  ]) {
    const runs = parseInline(bad)
    assert.ok(
      runs.every((r) => r.href === undefined),
      `${bad}: ${JSON.stringify(runs)}`,
    )
  }
  assert.deepEqual(parseInline("[click](javascript:alert(1))"), [{ text: "click)" }])
  // The console may leave the first `[` alone because a bold closes inside
  // it; the later `[` is then a candidate, and it is checked too.
  const tricky = parseInline("**[a**](https://a.example/[b](javascript:alert)")
  assert.ok(tricky.every((r) => r.href === undefined || r.href.startsWith("https://")), JSON.stringify(tricky))
})

test("escaped characters are literal, and a backslash before a letter is itself", () => {
  assert.deepEqual(parseInline("\\*not italic\\*"), [{ text: "*not italic*" }])
  assert.deepEqual(parseInline("\\[x\\](javascript:alert(1))"), [{ text: "[x](javascript:alert(1))" }])
  assert.deepEqual(parseInline("C:\\Users\\a"), [{ text: "C:\\Users\\a" }])
  assert.deepEqual(parseInline("one \\\\ two"), [{ text: "one \\ two" }])
  assert.deepEqual(parseInline("[a](https://a.example/\\(1\\))"), [{ text: "a", href: "https://a.example/(1)" }])
  assert.deepEqual(parseInline("a [ b ( c ] d ) e"), [{ text: "a [ b ( c ] d ) e" }])
  assert.equal(escapeInline("2*3 [a](b) \\"), "2\\*3 \\[a\\]\\(b\\) \\\\")
})

test("a hostile text of brackets parses in one pass", () => {
  const started = Date.now()
  const many = "[".repeat(10_000)
  assert.deepEqual(parseInline(many), [{ text: many }])
  parseInline(`${"[](".repeat(2_500)})`)
  parseInline("*".repeat(10_000))
  assert.ok(Date.now() - started < 2_000, "linear, not quadratic")
})

test("whatever a person types reads back as typed once escaped", () => {
  for (const typed of ["2 * 3", "**not bold**", "[a](javascript:x)", "C:\\path\\*", "( ) [ ] \\", "emoji 😀 * ok"]) {
    assert.deepEqual(parseInline(escapeInline(typed)), [{ text: typed }], typed)
    assert.equal(inlinePlainText(escapeInline(typed)), typed)
  }
})

const MARKS: InlineRun[] = [
  { text: "x", bold: true },
  { text: "y", bold: true, italic: true },
  { text: "z", italic: true },
  { text: "w" },
  { text: "u", italic: true },
  { text: "v", bold: true, italic: true },
  { text: "t", bold: true },
  { text: "*[l]*", href: "https://a.example/(p)" },
  { text: "m", bold: true, href: "https://a.example/(p)" },
  { text: "n", italic: true, href: "/dashboards/b_1" },
  { text: "\\" },
]

test("serializeInline round-trips every pair of neighbouring marks", () => {
  // Every ordered pair of mark sets, so each boundary the star rule must read
  // back (open, close, close-and-reopen) is exercised.
  const sets: Omit<InlineRun, "text">[] = [{}, { bold: true }, { italic: true }, { bold: true, italic: true }]
  for (const a of sets) {
    for (const b of sets) {
      for (const c of sets) {
        const runs = normalizeRuns([
          { text: "a", ...a },
          { text: "b", ...b },
          { text: "c", ...c },
        ])
        const text = serializeInline(runs)
        assert.deepEqual(parseInline(text), runs, `${JSON.stringify(runs)} -> ${text}`)
      }
    }
  }
  const text = serializeInline(MARKS)
  assert.deepEqual(parseInline(text), normalizeRuns(MARKS), text)
})

test("serializeInline moves edge spaces outside a mark and writes an unsafe link as its words", () => {
  assert.equal(serializeInline([{ text: "see ", bold: true }, { text: "this" }]), "**see** this")
  assert.equal(serializeInline([{ text: "go", href: "javascript:alert(1)" }]), "go")
  assert.equal(serializeInline([{ text: "go", href: "https://a.example" }]), "[go](https://a.example)")
  assert.equal(serializeInline([{ text: "  ", italic: true }]), "  ")
})

test("Tiptap nodes and the subset convert both ways, hard breaks as newlines", () => {
  const nodes = [
    { type: "text", text: "Up " },
    { type: "text", text: "a lot", marks: [{ type: "bold" }, { type: "italic" }] },
    { type: "hardBreak" },
    { type: "text", text: "a [link]", marks: [{ type: "link", attrs: { href: "https://a.example" } }] },
    { type: "text", text: " 2*3", marks: [{ type: "code" }] },
  ]
  const text = inlineFromNodes(nodes)
  assert.equal(text, "Up ***a lot***\n[a \\[link\\]](https://a.example) 2\\*3")
  assert.deepEqual(inlineToNodes(text), [
    { type: "text", text: "Up " },
    { type: "text", text: "a lot", marks: [{ type: "bold" }, { type: "italic" }] },
    { type: "hardBreak" },
    { type: "text", text: "a [link]", marks: [{ type: "link", attrs: { href: "https://a.example" } }] },
    // A mark the subset does not carry (code) is dropped, its words kept.
    { type: "text", text: " 2*3" },
  ])
  assert.equal(inlineFromNodes(undefined), "")
  assert.deepEqual(inlineToNodes(""), [])
  // An unsafe link mark never reaches the saved text.
  assert.equal(inlineFromNodes([{ type: "text", text: "x", marks: [{ type: "link", attrs: { href: "javascript:x" } }] }]), "x")
})

test("blocksForSave keeps a wide chart's width and leaves the default out", () => {
  assert.deepEqual(
    blocksForSave([
      { type: "chart", chart: "c_1", board: "b", width: "wide" },
      { type: "chart", chart: "c_2", width: "text" },
      { type: "quote", text: "q" },
      { type: "divider" },
    ]),
    [
      { type: "chart", chart: "c_1", width: "wide" },
      { type: "chart", chart: "c_2" },
      { type: "quote", text: "q" },
      { type: "divider" },
    ],
  )
})
