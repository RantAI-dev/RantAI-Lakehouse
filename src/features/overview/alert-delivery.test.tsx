import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { AlertDelivery, deliverySummary } from "./alert-delivery"

afterEach(cleanup)

// SRC-17: the Alerts page says whether an alert's webhook or e-mail arrived.
describe("deliverySummary", () => {
  it("says a failed delivery was not delivered and gives the API's reason as it is", () => {
    expect(
      deliverySummary({
        delivered: false,
        deliveryReason: "the webhook server answered with an error",
      })
    ).toEqual({
      state: "not-delivered",
      text: "Not delivered: the webhook server answered with an error",
    })
  })

  it("says Not delivered alone when a failed delivery carries no reason", () => {
    expect(deliverySummary({ delivered: false }).text).toBe("Not delivered")
  })

  it("says Delivered for a delivery that arrived", () => {
    expect(deliverySummary({ delivered: true })).toEqual({
      state: "delivered",
      text: "Delivered",
    })
  })

  it("says nothing when the outcome is not known, never Delivered", () => {
    expect(deliverySummary({})).toEqual({ state: "unknown", text: "" })
  })
})

describe("AlertDelivery", () => {
  it("shows Not delivered with the reason on the alert", () => {
    const { container } = render(
      <AlertDelivery
        alert={{
          delivered: false,
          deliveryReason: "the webhook server could not be reached",
        }}
      />
    )
    expect(container.textContent).toBe(
      "Not delivered: the webhook server could not be reached"
    )
  })

  it("shows a dash, not Delivered, for an alert whose outcome is not known", () => {
    const { container } = render(<AlertDelivery alert={{}} />)
    expect(container.textContent).toBe("—")
  })
})
