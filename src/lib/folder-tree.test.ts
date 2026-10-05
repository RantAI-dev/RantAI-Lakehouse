import { strict as assert } from "node:assert"
import { test } from "node:test"
import { buildFolderTree, flattenFolders, folderPath, groupBoardsByFolder } from "./folder-tree"

const folders = [
  { id: "a", name: "Sales", parentId: "" },
  { id: "b", name: "Q1", parentId: "a" },
  { id: "d", name: "Ops", parentId: "" },
]
const boards = [
  { id: "b1", name: "Revenue", folderId: "b" },
  { id: "b2", name: "Uptime", folderId: "d" },
  { id: "b3", name: "Loose", folderId: "" },
  { id: "b4", name: "Orphan", folderId: "gone" },
]

test("boards are filed under their folder and nested folders keep their depth", () => {
  const tree = buildFolderTree(folders, boards)
  assert.deepEqual(tree.folders.map((n) => n.folder.name), ["Ops", "Sales"])
  const sales = tree.folders[1]
  assert.equal(sales.depth, 1)
  assert.equal(sales.children[0].depth, 2)
  assert.deepEqual(sales.children[0].boards.map((b) => b.id), ["b1"])
})

test("a board in a deleted folder falls back to the root instead of vanishing", () => {
  const tree = buildFolderTree(folders, boards)
  assert.deepEqual(tree.rootBoards.map((b) => b.id), ["b3", "b4"])
})

test("flattening lists every folder once in display order", () => {
  const flat = flattenFolders(buildFolderTree(folders, boards).folders)
  assert.deepEqual(flat.map((f) => [f.folder.id, f.depth]), [["d", 1], ["a", 1], ["b", 2]])
})

test("the path of a nested folder joins its ancestors", () => {
  assert.equal(folderPath(folders, "b"), "Sales / Q1")
  assert.equal(folderPath(folders, ""), "")
})

test("a stored cycle does not loop forever", () => {
  const loop = [
    { id: "x", name: "X", parentId: "y" },
    { id: "y", name: "Y", parentId: "x" },
  ]
  assert.equal(folderPath(loop, "x"), "Y / X")
  assert.deepEqual(buildFolderTree(loop, []).folders, [])
})

const gFolders = [
  { id: "sales", name: "Sales", parentId: "" },
  { id: "q1", name: "Q1", parentId: "sales" },
  { id: "ops", name: "Ops", parentId: "" },
]
const gBoards = [
  { id: "z", name: "Zeta", folderId: "q1" },
  { id: "main", name: "Main", folderId: null },
  { id: "a", name: "Alpha", folderId: "q1" },
  { id: "gone", name: "Orphan", folderId: "deleted" },
]

test("sections: no-folder first, then folders in tree order under their full path", () => {
  const out = groupBoardsByFolder(gFolders, gBoards)
  // "Sales" holds no board itself and has a subfolder, so it is not a
  // section: it would read as an empty folder above "Sales / Q1".
  assert.deepEqual(out.map((s) => s.path), ["", "Ops", "Sales / Q1"])
  assert.deepEqual(out[0].boards.map((b) => b.id), ["main", "gone"], "a board in a deleted folder lands in no-folder")
})

test("boards keep the order they arrive in, so the list's own sort still applies", () => {
  const q1 = groupBoardsByFolder(gFolders, gBoards).find((s) => s.folderId === "q1")
  assert.deepEqual(q1?.boards.map((b) => b.id), ["z", "a"])
})

test("empty folders are shown unless asked not to, and no empty no-folder section is made", () => {
  assert.deepEqual(groupBoardsByFolder(gFolders, gBoards, false).map((s) => s.path), ["", "Sales / Q1"])
  const onlyFiled = [{ id: "a", name: "Alpha", folderId: "ops" }]
  assert.deepEqual(groupBoardsByFolder(gFolders, onlyFiled).map((s) => s.path), ["Ops", "Sales / Q1"])
})

test("a parent folder with a board of its own is still a section", () => {
  const withParent = [...gBoards, { id: "p", name: "Parent board", folderId: "sales" }]
  assert.deepEqual(groupBoardsByFolder(gFolders, withParent).map((s) => s.path), ["", "Ops", "Sales", "Sales / Q1"])
})
