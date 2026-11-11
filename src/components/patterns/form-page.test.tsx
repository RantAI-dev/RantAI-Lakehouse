// WEB-7 D1: the form-page shell that replaces the side panel. Its labels are
// the point of the change — "Creating…" on a new page, "Saving…" on an edit
// page — so this test pins both, plus Cancel and the back link.
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { FormPage } from "./form-page"

afterEach(cleanup)

function renderForm(overrides: Partial<Parameters<typeof FormPage>[0]> = {}) {
  const calls = { submit: 0, cancel: 0 }
  render(
    <FormPage
      title="Create Role"
      description="Define a role template."
      backHref="/admin/roles"
      submitLabel="Create"
      submittingLabel="Creating…"
      canSubmit
      onSubmit={() => {
        calls.submit += 1
      }}
      onCancel={() => {
        calls.cancel += 1
      }}
      {...overrides}
    >
      <p>fields</p>
    </FormPage>
  )
  return calls
}

describe("FormPage (WEB-7 D1)", () => {
  it("shows the title, the description, the fields and an enabled submit label", () => {
    renderForm()
    expect(screen.getByRole("heading", { name: "Create Role" })).toBeTruthy()
    expect(screen.getByText("Define a role template.")).toBeTruthy()
    expect(screen.getByText("fields")).toBeTruthy()
    const submit = screen.getByRole("button", { name: "Create" }) as HTMLButtonElement
    expect(submit.disabled).toBe(false)
  })

  it("links back to the list the form came from", () => {
    renderForm()
    expect(screen.getByRole("link", { name: /Back/ }).getAttribute("href")).toBe("/admin/roles")
  })

  it("cancel calls onCancel, and save calls onSubmit", () => {
    const calls = renderForm()
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    expect(calls).toEqual({ submit: 1, cancel: 1 })
  })

  it("says the page's submitting label while saving, never 'Creating…' on an edit page", () => {
    renderForm({
      title: "Edit Role",
      submitLabel: "Save",
      submittingLabel: "Saving…",
      submitting: true,
    })
    expect(screen.queryByRole("button", { name: "Creating…" })).toBeNull()
    const saving = screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement
    expect(saving.disabled).toBe(true)
    expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("keeps save disabled until the page says it can submit", () => {
    renderForm({ canSubmit: false })
    expect((screen.getByRole("button", { name: "Create" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("shows the API's sentence next to the footer", () => {
    renderForm({ error: "A role with that name already exists." })
    expect(screen.getByRole("alert").textContent).toBe("A role with that name already exists.")
  })
})
