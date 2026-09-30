import { strict as assert } from "node:assert"
import { test } from "node:test"
import { buildFolderTree, flattenFolders, folderPath } from "./folder-tree"

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
