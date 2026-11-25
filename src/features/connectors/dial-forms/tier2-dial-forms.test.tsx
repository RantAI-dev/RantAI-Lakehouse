import { fireEvent, render } from "@testing-library/react"
import { within } from "@testing-library/dom"
import { describe, expect, it, mock } from "bun:test"
import { MongoDialForm } from "./mongo-dial-form"
import { OracleDialForm } from "./oracle-dial-form"
import { KafkaDialForm } from "./kafka-dial-form"
import { SftpDialForm } from "./sftp-dial-form"

// Scoped to each render's own container (`within`), not the global
// `screen` -- the full test suite shares one DOM, and these four forms
// reuse common labels ("User", "Auth type") with the Tier 1 forms in
// dial-forms.test.tsx.
describe("tier 2 dial forms", () => {
  it("MongoDialForm states direct-connection-only and offers no discovery toggle", () => {
    const { container } = render(<MongoDialForm value={null} onChange={mock()} />)
    const screen = within(container)
    expect(screen.getByText(/refuses.*mongodb\+srv/i)).toBeDefined()
    expect(screen.getByLabelText("Seed host 1")).toBeDefined()
    expect(screen.getByLabelText("Username")).toHaveProperty("type", "text")
    // No control anywhere in this form's markup can toggle SRV/replica-set
    // discovery -- MongoDial has no such field at all (deny_unknown_fields).
    // Only checkboxes/selects are candidate toggles; the honest prose note
    // above legitimately mentions "srv" in text, so this checks for
    // CONTROLS, not absence of the word.
    expect(container.querySelectorAll('input[type="checkbox"], select').length).toBe(0)
    expect(screen.queryByLabelText(/discover/i)).toBeNull()
  })

  it("OracleDialForm renders the sql-shaped fields with driver fixed and a TLS DN field", () => {
    const onChange = mock()
    const { container } = render(<OracleDialForm value={null} onChange={onChange} />)
    const screen = within(container)
    expect(screen.getByLabelText("Host")).toBeDefined()
    expect(screen.getByLabelText(/TLS server certificate DN/i)).toBeDefined()
    expect(screen.getByText(/cannot run Oracle change-data-capture/i)).toBeDefined()
    // No driver selector -- Oracle's driver is fixed, never user-chosen.
    expect(screen.queryByLabelText("Driver")).toBeNull()
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "oracle.invalid" } })
    expect(onChange.mock.calls[0]?.[0]).toMatchObject({ driver: "oracle", host: "oracle.invalid" })
  })

  it("KafkaDialForm offers exactly the two Rust KafkaAuth variants, username shown for sasl_plain", () => {
    const { container } = render(<KafkaDialForm value={null} onChange={mock()} />)
    const screen = within(container)
    // WEB-9 D5: the control is a combobox; its shown label and the opened
    // list's options are what the native select's `.value` and `.options` were.
    const trigger = screen.getByLabelText("Auth type")
    expect(trigger.querySelector('[data-slot="select-value"]')?.textContent).toBe("sasl_plain")
    // The list is portalled onto the body, outside this form's container.
    fireEvent.click(trigger)
    expect(within(document.body).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "sasl_plain",
      "none",
    ])
    expect(screen.getByLabelText("SASL username")).toHaveProperty("type", "text")
  })

  it("KafkaDialForm drops the username field and states there is no credential for none auth", () => {
    const { container } = render(
      <KafkaDialForm
        value={{
          bootstrapServers: ["kafka.invalid:9092"],
          topic: "t",
          auth: { type: "none" },
          groupId: "g",
          microBatchSeconds: 30,
        }}
        onChange={mock()}
      />
    )
    const screen = within(container)
    expect(screen.queryByLabelText("SASL username")).toBeNull()
    expect(screen.getByText(/no credential/i)).toBeDefined()
  })

  it("SftpDialForm marks the host key fingerprint required and shows how to obtain it", () => {
    const { container } = render(<SftpDialForm value={null} onChange={mock()} />)
    const screen = within(container)
    const fingerprint = screen.getByLabelText("Host key fingerprint")
    expect(fingerprint).toHaveProperty("required", true)
    expect(screen.getByText(/ssh-keyscan/)).toBeDefined()
    expect(screen.getByText(/ssh-keygen -lf -/)).toBeDefined()
    // WEB-9 D5: the two choices are read from the opened list; each option's
    // text is the API's word the native option's value was.
    const trigger = screen.getByLabelText("Auth type")
    fireEvent.click(trigger)
    expect(within(document.body).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "password",
      "public_key",
    ])
  })
})
