import { fireEvent, screen } from "@testing-library/react"

/**
 * Chooses an option of a `SelectField` the way a pointer does (`WEB-9` D5):
 * the trigger opens the list, and the option is pressed, released and
 * clicked. A bare `click` on an item that never saw a pointerdown is
 * dropped as a stray mouse click, so the whole sequence is part of the
 * choice. Shared by every test that drives the console's one dropdown.
 */
export function chooseOption(trigger: HTMLElement, optionName: string | RegExp) {
  fireEvent.click(trigger)
  const option = screen.getByRole("option", { name: optionName })
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}
