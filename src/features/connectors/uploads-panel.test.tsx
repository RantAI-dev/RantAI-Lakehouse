// The "Uploaded files" tab (plan T11): the columns the plan names, "Not
// measured" for a row count nobody measured, Load only where a load may
// start, Delete behind a confirmation that says the table stays and disabled
// while loading, polling only while a row is loading, and the API's refusal
// shown through the existing error state.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { Upload, UploadFolder } from "@/services/contracts/uploads"
import { UploadsPanel } from "./uploads-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function upload(over: Partial<Upload> = {}): Upload {
  return {
    id: "up-1",
    originalFilename: "stock.csv",
    sizeBytes: 2048,
    uploadedBy: "Ana",
    status: "uploaded",
    createdAt: "2026-10-02T10:00:00Z",
    updatedAt: "2026-10-02T10:00:00Z",
    ...over,
  }
}

type Call = { url: string; method: string; body?: unknown }

function folder(over: Partial<UploadFolder> & Pick<UploadFolder, "id" | "name">): UploadFolder {
  return {
    parentId: null,
    fileCount: 0,
    childCount: 0,
    createdAt: "2026-10-26T10:00:00Z",
    updatedAt: "2026-10-26T10:00:00Z",
    ...over,
  }
}

type FolderRoutes = {
  /** What `GET /api/uploads/folders` answers (default: no folders). */
  folders?: UploadFolder[]
  /** Answers `POST /api/uploads/folders`, `PATCH` and `DELETE` of a folder, and `POST /api/uploads/move`. */
  write?: (call: Call) => Response
}

/**
 * Answers the list (`list(n, url)` is told which read it is), the folders, the delete of an upload
 * and, through `extra.write`, the folder and move writes. `GET /api/uploads/folders` is not
 * recorded: the polling tests count the reads of the LIST.
 */
function stubFetch(list: (n: number, url: string) => Upload[] | Response, remove?: () => Response, extra: FolderRoutes = {}): Call[] {
  const calls: Call[] = []
  let lists = 0
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    if (url === "/api/uploads/folders" && method === "GET") return json(extra.folders ?? [])
    const call: Call = { url, method, body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined }
    calls.push(call)
    if ((url === "/api/uploads" || url.startsWith("/api/uploads?")) && method === "GET") {
      lists += 1
      const out = list(lists, url)
      return out instanceof Response ? out : json(out)
    }
    if (url.startsWith("/api/uploads/folders") || url === "/api/uploads/move") {
      if (extra.write) return extra.write(call)
    } else if (method === "DELETE" && remove) return remove()
    throw new Error(`unexpected fetch: ${method} ${url}`)
  }) as unknown as typeof fetch
  return calls
}

function rowOf(name: string) {
  return screen.getByText(name).closest("tr") as HTMLElement
}

describe("UploadsPanel", () => {
  it("lists each upload with its table as a link, and Not measured where there is no row count", async () => {
    stubFetch(() => [
      upload({ id: "up-1", originalFilename: "loaded.csv", status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 120 }),
      upload({ id: "up-2", originalFilename: "failed.csv", status: "failed", error: "The load into the table failed." }),
      upload({ id: "up-3", originalFilename: "fresh.csv" }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("loaded.csv")

    for (const heading of ["File", "Size", "Status", "Table", "Rows", "Uploaded by", "Uploaded"]) {
      expect(screen.getByRole("columnheader", { name: heading })).toBeDefined()
    }
    const loaded = within(rowOf("loaded.csv"))
    expect(loaded.getByText("Loaded")).toBeDefined()
    expect(loaded.getByText("120")).toBeDefined()
    expect(loaded.getByRole("link", { name: "stock" }).getAttribute("href")).toBe("/data/assets/stock")
    expect(loaded.getByText("2.0 KB")).toBeDefined()
    expect(loaded.getByText("Ana")).toBeDefined()

    const failed = within(rowOf("failed.csv"))
    expect(failed.getByText("Not measured")).toBeDefined()
    expect(failed.queryByText("0")).toBeNull()
    expect(failed.getByText("The load into the table failed.")).toBeDefined()
    expect(within(rowOf("fresh.csv")).getByText("Not measured")).toBeDefined()
  })

  it("says a file of a load of several files was loaded with the first, and that its rows are counted there", async () => {
    stubFetch(() => [
      upload({ id: "up-1", originalFilename: "stock_a.csv", status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 5 }),
      upload({ id: "up-2", originalFilename: "stock_b.csv", status: "ingested", bronzeTable: "stock", assetId: "stock", loadedWith: "up-1" }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("stock_b.csv")

    const second = within(rowOf("stock_b.csv"))
    expect(second.getByText("Loaded with stock_a.csv")).toBeDefined()
    expect(second.getByText("Counted with the first file")).toBeDefined()
    expect(second.queryByText("Not measured")).toBeNull()
    const first = within(rowOf("stock_a.csv"))
    expect(first.queryByText(/Loaded with/)).toBeNull()
    expect(first.getByText("5")).toBeDefined()
  })

  it("shows a file still being received as Receiving, with the size it announced, and offers no Load", async () => {
    stubFetch(() => [
      upload({ id: "up-5", originalFilename: "big.csv", status: "receiving", sizeBytes: 0, declaredSize: 20 * 1024 * 1024 }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("big.csv")
    const row = within(rowOf("big.csv"))
    expect(row.getByText("Receiving")).toBeDefined()
    expect(row.getByText("20.0 MB")).toBeDefined()
    expect(row.queryByText("Load")).toBeNull()
    // It can still be deleted: that aborts the upload.
    expect((row.getByRole("button", { name: "Delete big.csv" }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("says in the empty state that CSV, TSV, Excel and Parquet files can be uploaded", async () => {
    stubFetch(() => [])
    render(<UploadsPanel />)
    expect(await screen.findByText("No uploaded files")).toBeDefined()
    expect(screen.getByText("Upload CSV, TSV, Excel or Parquet files to bring them in as raw tables.")).toBeDefined()
  })

  it("offers Load for an uploaded or failed file, opening the upload page on it, and for no other", async () => {
    stubFetch(() => [
      upload({ id: "up-1", originalFilename: "a.csv", status: "uploaded" }),
      upload({ id: "up-2", originalFilename: "b.csv", status: "failed" }),
      upload({ id: "up-3", originalFilename: "c.csv", status: "ingested", bronzeTable: "c", assetId: "c" }),
      upload({ id: "up-4", originalFilename: "d.csv", status: "ingesting" }),
    ])
    render(<UploadsPanel />)
    await screen.findByText("a.csv")
    expect(within(rowOf("a.csv")).getByText("Load").closest("a")?.getAttribute("href")).toBe("/connectors/upload?id=up-1")
    expect(within(rowOf("b.csv")).getByText("Load").closest("a")?.getAttribute("href")).toBe("/connectors/upload?id=up-2")
    expect(within(rowOf("c.csv")).queryByText("Load")).toBeNull()
    expect(within(rowOf("d.csv")).queryByText("Load")).toBeNull()
  })

  it("asks first, says the table stays, and deletes only after the confirmation", async () => {
    let removed = false
    const calls = stubFetch(
      () => (removed ? [] : [upload({ status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 3 })]),
      () => {
        removed = true
        return new Response(null, { status: 204 })
      }
    )
    render(<UploadsPanel />)
    await screen.findByText("stock.csv")

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    expect(await screen.findByText("Delete stock.csv?")).toBeDefined()
    expect(screen.getByText("The table stock stays, with its rows.")).toBeDefined()
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))
    await waitFor(() => expect(calls.filter((c) => c.method === "DELETE").length).toBe(1))
    expect(calls.find((c) => c.method === "DELETE")?.url).toBe("/api/uploads/up-1")
    expect(await screen.findByText("No uploaded files")).toBeDefined()
  })

  it("does not delete when the confirmation is cancelled, and shows a refusal in the dialog", async () => {
    const sentence = "This upload is being loaded, so it cannot be deleted yet."
    const calls = stubFetch(
      () => [upload()],
      () => json({ error: sentence }, 409)
    )
    render(<UploadsPanel />)
    await screen.findByText("stock.csv")

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    await screen.findByText("Delete stock.csv?")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByText("Delete stock.csv?")).toBeNull())
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)

    fireEvent.click(screen.getByRole("button", { name: "Delete stock.csv" }))
    await screen.findByText("Delete stock.csv?")
    fireEvent.click(screen.getByRole("button", { name: "Delete" }))
    expect((await screen.findByRole("alert")).textContent).toBe(sentence)
    expect(screen.getByText("stock.csv")).toBeDefined()
  })

  it("disables Delete while a row is loading", async () => {
    stubFetch(() => [upload({ status: "ingesting" })])
    render(<UploadsPanel pollMs={1000} />)
    await screen.findByText("stock.csv")
    expect((screen.getByRole("button", { name: "Delete stock.csv" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("refreshes while a row is loading and stops when none is", async () => {
    const calls = stubFetch((n) => [n < 3 ? upload({ status: "ingesting" }) : upload({ status: "ingested", bronzeTable: "stock", assetId: "stock", rows: 3 })])
    render(<UploadsPanel pollMs={20} />)
    await screen.findByText("Loaded")
    const settled = calls.length
    expect(settled).toBe(3)
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(settled)
  })

  it("does not refresh a list that has nothing loading", async () => {
    const calls = stubFetch(() => [upload()])
    render(<UploadsPanel pollMs={20} />)
    await screen.findByText("stock.csv")
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(1)
  })

  it("stops refreshing when the tab is left", async () => {
    const calls = stubFetch(() => [upload({ status: "ingesting" })])
    const view = render(<UploadsPanel pollMs={20} />)
    await screen.findByText("stock.csv")
    view.unmount()
    const after = calls.length
    await new Promise((resolve) => setTimeout(resolve, 120))
    expect(calls.length).toBe(after)
  })

  it("shows the upload button in the empty state", async () => {
    stubFetch(() => [])
    render(<UploadsPanel />)
    expect(await screen.findByText("No uploaded files")).toBeDefined()
    expect(screen.getByText("Upload file").closest("a")?.getAttribute("href")).toBe("/connectors/upload")
  })

  it("says a missing permission through the existing state, never a blank page", async () => {
    stubFetch(() => json({ error: "You do not have permission to do this." }, 403))
    render(<UploadsPanel />)
    expect(await screen.findByText("You don't have access")).toBeDefined()
  })

  it("shows the API's sentence for an unavailable list, with Retry", async () => {
    stubFetch(() => json({ error: "database error" }, 500))
    render(<UploadsPanel />)
    expect(await screen.findByText("Service unavailable")).toBeDefined()
    expect(screen.getByText(/database error/)).toBeDefined()
    expect(screen.getByRole("button", { name: "Retry" })).toBeDefined()
  })
})

// DATA-35 (docs/superpowers/plans/2026-10-26-data-35-upload-folders.md, T4): the folder column,
// "No folder", a search over every folder that shows each file's folder, moving several files, and
// a Delete that is off with its reason while a folder holds anything.
describe("UploadsPanel folders", () => {
  const FOLDERS = [
    folder({ id: "uf-sales", name: "Sales", fileCount: 1, childCount: 1 }),
    folder({ id: "uf-q1", name: "Q1", parentId: "uf-sales", fileCount: 2 }),
    folder({ id: "uf-ops", name: "Ops" }),
  ]
  const ALL = [
    upload({ id: "up-1", originalFilename: "jan.csv", folderId: "uf-q1" }),
    upload({ id: "up-2", originalFilename: "feb.csv", folderId: "uf-q1" }),
    upload({ id: "up-3", originalFilename: "loose.csv" }),
  ]

  function folderNav() {
    return within(screen.getByRole("navigation", { name: "Folders" }))
  }

  it("shows All files, No folder and the folders nested, with the server's counts, and every file with its folder path", async () => {
    stubFetch(() => ALL, undefined, { folders: FOLDERS })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")

    const nav = folderNav()
    for (const name of ["All files", "No folder", "Sales", "Q1", "Ops"]) {
      expect(nav.getByRole("button", { name: new RegExp(`^${name}`) })).toBeDefined()
    }
    expect(nav.getByRole("button", { name: /^Q1/ }).textContent).toContain("2")
    expect(screen.getByRole("columnheader", { name: "Folder" })).toBeDefined()
    expect(within(rowOf("jan.csv")).getByText("Sales / Q1")).toBeDefined()
    expect(within(rowOf("loose.csv")).getByText("No folder")).toBeDefined()
  })

  it("asks for the open folder's files, and for the files in no folder, and drops the folder column inside a folder", async () => {
    const calls = stubFetch(
      (_n, url) => (url.includes("folderId=uf-q1") ? [ALL[0]!, ALL[1]!] : url.includes("folderId=none") ? [ALL[2]!] : ALL),
      undefined,
      { folders: FOLDERS }
    )
    render(<UploadsPanel />)
    await screen.findByText("loose.csv")

    fireEvent.click(folderNav().getByRole("button", { name: /^Q1/ }))
    // Compared as a boolean: while the element is still there, bun formats the whole DOM node into the
    // failure message on every poll, which took ~8 s and timed this test out (PR #259 CI).
    await waitFor(() => expect(screen.queryByText("loose.csv") !== null).toBe(false))
    expect(screen.getByText("jan.csv")).toBeDefined()
    expect(calls.map((c) => c.url)).toContain("/api/uploads?folderId=uf-q1")
    expect(screen.queryByRole("columnheader", { name: "Folder" })).toBeNull()

    fireEvent.click(folderNav().getByRole("button", { name: /^No folder/ }))
    await screen.findByText("loose.csv")
    expect(calls.map((c) => c.url)).toContain("/api/uploads?folderId=none")
  })

  it("searches every folder, whichever is open, and shows where each match sits", async () => {
    const calls = stubFetch((_n, url) => (url.includes("q=jan") ? [ALL[0]!] : ALL), undefined, { folders: FOLDERS })
    render(<UploadsPanel searchDelayMs={0} />)
    await screen.findByText("loose.csv")
    fireEvent.click(folderNav().getByRole("button", { name: /^Ops/ }))

    fireEvent.change(screen.getByRole("textbox", { name: "Search uploaded files" }), { target: { value: " jan " } })
    await waitFor(() => expect(calls.map((c) => c.url)).toContain("/api/uploads?q=jan"))
    await waitFor(() => expect(screen.queryByText("loose.csv") !== null).toBe(false))
    expect(screen.getByText("Searching every folder.")).toBeDefined()
    expect(within(rowOf("jan.csv")).getByText("Sales / Q1")).toBeDefined()
  })

  it("says nothing matches a search that finds nothing", async () => {
    stubFetch((_n, url) => (url.includes("q=") ? [] : ALL), undefined, { folders: FOLDERS })
    render(<UploadsPanel searchDelayMs={0} />)
    await screen.findByText("loose.csv")
    fireEvent.change(screen.getByRole("textbox", { name: "Search uploaded files" }), { target: { value: "zzz" } })
    expect(await screen.findByText("No file or table matches the search.")).toBeDefined()
  })

  it("moves several ticked files out of their folder in one request", async () => {
    const calls = stubFetch(() => ALL, undefined, {
      folders: FOLDERS,
      write: () => json({ moved: 2 }),
    })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")
    expect(screen.queryByRole("button", { name: "Move to folder…" })).toBeNull()

    fireEvent.click(screen.getByRole("checkbox", { name: "Select jan.csv" }))
    fireEvent.click(screen.getByRole("checkbox", { name: "Select feb.csv" }))
    expect(screen.getByText("2 selected")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Move to folder…" }))
    await screen.findByText("Move to folder")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => expect(calls.some((c) => c.url === "/api/uploads/move")).toBe(true))
    expect(calls.find((c) => c.url === "/api/uploads/move")?.body).toEqual({ uploadIds: ["up-1", "up-2"], folderId: null })
    await waitFor(() => expect(screen.queryByText("2 selected") !== null).toBe(false))
  })

  it("moves a file into a folder chosen in the picker", async () => {
    const calls = stubFetch(() => ALL, undefined, { folders: FOLDERS, write: () => json({ moved: 1 }) })
    render(<UploadsPanel />)
    await screen.findByText("loose.csv")
    fireEvent.click(screen.getByRole("checkbox", { name: "Select loose.csv" }))
    fireEvent.click(screen.getByRole("button", { name: "Move to folder…" }))
    await screen.findByText("Move to folder")

    fireEvent.click(screen.getByRole("combobox", { name: "Destination folder" }))
    fireEvent.click(await screen.findByRole("option", { name: "Sales / Q1" }))
    fireEvent.click(screen.getByRole("button", { name: "Move" }))

    await waitFor(() => expect(calls.some((c) => c.url === "/api/uploads/move")).toBe(true))
    expect(calls.find((c) => c.url === "/api/uploads/move")?.body).toEqual({ uploadIds: ["up-3"], folderId: "uf-q1" })
  })

  it("shows the API's sentence when a move is refused, and keeps the files ticked", async () => {
    stubFetch(() => ALL, undefined, { folders: FOLDERS, write: () => json({ error: "Upload not found." }, 404) })
    render(<UploadsPanel />)
    await screen.findByText("loose.csv")
    fireEvent.click(screen.getByRole("checkbox", { name: "Select loose.csv" }))
    fireEvent.click(screen.getByRole("button", { name: "Move to folder…" }))
    await screen.findByText("Move to folder")
    fireEvent.click(screen.getByRole("button", { name: "Move" }))
    expect((await screen.findByRole("alert")).textContent).toBe("Upload not found.")
    expect(screen.getByText("1 selected")).toBeDefined()
  })

  it("turns Delete folder off while a folder holds a file or a folder, and says why", async () => {
    stubFetch(() => ALL, undefined, { folders: FOLDERS })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")

    fireEvent.click(folderNav().getByRole("button", { name: /^Sales/ }))
    const off = screen.getByRole("button", { name: "Delete folder Sales" }) as HTMLButtonElement
    expect(off.disabled).toBe(true)
    expect(screen.getByTestId("folder-delete-reason").textContent).toBe(
      "This folder holds 1 file and 1 folder. Move them out and delete its folders first."
    )

    fireEvent.click(folderNav().getByRole("button", { name: /^Ops/ }))
    expect((screen.getByRole("button", { name: "Delete folder Ops" }) as HTMLButtonElement).disabled).toBe(false)
    expect(screen.queryByTestId("folder-delete-reason")).toBeNull()
  })

  it("deletes an empty folder after a confirmation and goes back to every file", async () => {
    let gone = false
    const calls = stubFetch(() => ALL, undefined, {
      get folders() {
        return gone ? FOLDERS.filter((f) => f.id !== "uf-ops") : FOLDERS
      },
      write: () => {
        gone = true
        return new Response(null, { status: 204 })
      },
    })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")
    fireEvent.click(folderNav().getByRole("button", { name: /^Ops/ }))

    fireEvent.click(screen.getByRole("button", { name: "Delete folder Ops" }))
    expect(await screen.findByText("Delete the folder Ops?")).toBeDefined()
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    await waitFor(() => expect(calls.find((c) => c.method === "DELETE")?.url).toBe("/api/uploads/folders/uf-ops"))
    await waitFor(() => expect(within(screen.getByRole("navigation", { name: "Folders" })).queryByText("Ops")).toBeNull())
  })

  it("makes a folder inside the open one, and shows the API's sentence for a refusal", async () => {
    let refuse = true
    const calls = stubFetch(() => ALL, undefined, {
      folders: FOLDERS,
      write: () =>
        refuse
          ? json({ error: "A folder with that name already exists in the same place." }, 409)
          : json(folder({ id: "uf-new", name: "Archive", parentId: "uf-ops" }), 201),
    })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")
    fireEvent.click(folderNav().getByRole("button", { name: /^Ops/ }))

    fireEvent.click(screen.getByRole("button", { name: "New subfolder" }))
    await screen.findByText("New folder in Ops")
    fireEvent.change(screen.getByLabelText("Folder name"), { target: { value: " Archive " } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    expect((await screen.findByRole("alert")).textContent).toBe("A folder with that name already exists in the same place.")

    refuse = false
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    await waitFor(() => expect(screen.queryByText("New folder in Ops") !== null).toBe(false))
    const posts = calls.filter((c) => c.method === "POST" && c.url === "/api/uploads/folders")
    expect(posts.map((c) => c.body)).toEqual([
      { name: "Archive", parentId: "uf-ops" },
      { name: "Archive", parentId: "uf-ops" },
    ])
  })

  it("offers no New subfolder in a folder on the third level", async () => {
    stubFetch(() => ALL, undefined, {
      folders: [
        folder({ id: "uf-a", name: "A", childCount: 1 }),
        folder({ id: "uf-b", name: "B", parentId: "uf-a", childCount: 1 }),
        folder({ id: "uf-c", name: "C", parentId: "uf-b" }),
      ],
    })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")
    fireEvent.click(folderNav().getByRole("button", { name: /^C/ }))
    const button = screen.getByRole("button", { name: "New subfolder" }) as HTMLButtonElement
    expect(button.disabled).toBe(true)
    expect(button.getAttribute("title")).toBe("Folders can be nested at most 3 levels deep.")
  })

  it("renames the open folder", async () => {
    const calls = stubFetch(() => ALL, undefined, {
      folders: FOLDERS,
      write: () => json(folder({ id: "uf-ops", name: "Operations" })),
    })
    render(<UploadsPanel />)
    await screen.findByText("jan.csv")
    fireEvent.click(folderNav().getByRole("button", { name: /^Ops/ }))

    fireEvent.click(screen.getByRole("button", { name: "Rename folder Ops" }))
    await screen.findByText("Rename folder")
    expect((screen.getByLabelText("Folder name") as HTMLInputElement).value).toBe("Ops")
    fireEvent.change(screen.getByLabelText("Folder name"), { target: { value: "Operations" } })
    fireEvent.click(screen.getByRole("button", { name: "Rename" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true))
    expect(calls.find((c) => c.method === "PATCH")).toMatchObject({ url: "/api/uploads/folders/uf-ops", body: { name: "Operations" } })
  })

  it("keeps the flat list when the folders cannot be loaded, and says so", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      if (String(input) === "/api/uploads/folders") return json({ error: "database error" }, 500)
      return json([upload()])
    }) as unknown as typeof fetch
    render(<UploadsPanel />)
    expect(await screen.findByText("stock.csv")).toBeDefined()
    expect((await screen.findByRole("alert")).textContent).toContain("The folders could not be loaded")
  })
})
