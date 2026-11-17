import { describe, expect, it } from "bun:test"
import type { Upload, UploadFolder } from "@/services/contracts/uploads"
import {
  ALL_FILES,
  NO_FOLDER,
  UPLOAD_FOLDER_MAX_DEPTH,
  asFolderLikes,
  canNestIn,
  commonFolder,
  deleteFolderReason,
  folderChoices,
  folderDepth,
  folderLabel,
  folderOfTable,
} from "./upload-folders"

function folder(id: string, name: string, parentId: string | null, counts: [number, number] = [0, 0]): UploadFolder {
  return {
    id,
    name,
    parentId,
    fileCount: counts[0],
    childCount: counts[1],
    createdAt: "2026-10-26T10:00:00Z",
    updatedAt: "2026-10-26T10:00:00Z",
  }
}

function upload(id: string, over: Partial<Upload> = {}): Upload {
  return {
    id,
    originalFilename: `${id}.csv`,
    sizeBytes: 1,
    uploadedBy: "Ana",
    status: "uploaded",
    createdAt: "2026-10-26T10:00:00Z",
    updatedAt: "2026-10-26T10:00:00Z",
    ...over,
  }
}

const FOLDERS = [
  folder("uf-a", "Sales", null, [0, 1]),
  folder("uf-b", "Q1", "uf-a", [0, 1]),
  folder("uf-c", "Jan", "uf-b", [2, 0]),
  folder("uf-z", "Ops", null),
]

describe("upload folder rules", () => {
  it("allows three levels and offers no fourth", () => {
    expect(UPLOAD_FOLDER_MAX_DEPTH).toBe(3)
    expect(folderDepth(FOLDERS, "uf-a")).toBe(1)
    expect(folderDepth(FOLDERS, "uf-c")).toBe(3)
    expect(folderDepth(FOLDERS, "uf-missing")).toBe(0)
    expect(canNestIn(FOLDERS, "uf-a")).toBe(true)
    expect(canNestIn(FOLDERS, "uf-b")).toBe(true)
    expect(canNestIn(FOLDERS, "uf-c")).toBe(false)
  })

  it("turns the API's null parent into the tree's top level", () => {
    expect(asFolderLikes(FOLDERS).map((f) => f.parentId)).toEqual(["", "uf-a", "uf-b", ""])
  })

  it("names a folder by its path, and a file in none or in an unknown folder as No folder", () => {
    expect(folderLabel(FOLDERS, "uf-c")).toBe("Sales / Q1 / Jan")
    expect(folderLabel(FOLDERS, undefined)).toBe("No folder")
    expect(folderLabel(FOLDERS, "uf-gone")).toBe("No folder")
    expect(NO_FOLDER).toBe("none")
    expect(ALL_FILES).toBe("all")
  })

  it("lists every folder in tree order for a picker", () => {
    expect(folderChoices(FOLDERS).map((c) => [c.path, c.depth])).toEqual([
      ["Ops", 1],
      ["Sales", 1],
      ["Sales / Q1", 2],
      ["Sales / Q1 / Jan", 3],
    ])
  })

  it("says why a folder cannot be deleted, with the counts the server gave, and nothing for an empty one", () => {
    expect(deleteFolderReason(FOLDERS[3]!)).toBeNull()
    expect(deleteFolderReason(FOLDERS[2]!)).toBe("This folder holds 2 files. Move them out and delete its folders first.")
    expect(deleteFolderReason(FOLDERS[0]!)).toBe("This folder holds 1 folder. Move them out and delete its folders first.")
    expect(deleteFolderReason({ fileCount: 1, childCount: 3 })).toBe(
      "This folder holds 1 file and 3 folders. Move them out and delete its folders first."
    )
  })

  it("offers a table's folder from the newest upload loaded into it", () => {
    const uploads = [
      upload("up-3", { bronzeTable: "stock", folderId: "uf-z" }),
      upload("up-2", { bronzeTable: "stock", folderId: "uf-a" }),
      upload("up-1", { bronzeTable: "orders" }),
    ]
    expect(folderOfTable("stock", uploads)).toBe("uf-z")
    expect(folderOfTable("orders", uploads)).toBeUndefined()
    expect(folderOfTable("none_such", uploads)).toBeUndefined()
  })

  it("finds the folder a selection shares, and none when they differ", () => {
    expect(commonFolder([upload("a", { folderId: "uf-a" }), upload("b", { folderId: "uf-a" })])).toBe("uf-a")
    expect(commonFolder([upload("a", { folderId: "uf-a" }), upload("b")])).toBeUndefined()
    expect(commonFolder([])).toBeUndefined()
  })
})
