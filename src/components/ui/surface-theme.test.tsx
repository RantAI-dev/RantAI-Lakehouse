import * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select"
import { SurfaceThemeProvider } from "@/components/ui/surface-theme"

afterEach(cleanup)

/** Find a portaled popup by its slot. A portal is a sibling of the render container on <body>. */
function portaled(slot: string): HTMLElement {
  const popup = document.body.querySelector(`[data-slot="${slot}"]`)
  if (!popup) throw new Error(`No portaled [data-slot="${slot}"] was opened`)
  return popup as HTMLElement
}

function classes(popup: HTMLElement): string[] {
  return popup.className.split(/\s+/).filter(Boolean)
}

function openSelect(label: string) {
  fireEvent.click(screen.getByRole("combobox", { name: label }))
}

function openMenu(label: string) {
  fireEvent.click(screen.getByRole("button", { name: label }))
}

function openPopover(label: string) {
  fireEvent.click(screen.getByRole("button", { name: label }))
}

describe("WEB-10: portaled popups follow the opening surface's theme", () => {
  it("a Select opened on a light surface copies theme-light onto its portaled list", async () => {
    const { container } = render(
      <SurfaceThemeProvider themeClass="theme-light">
        <Select>
          <SelectTrigger aria-label="Fruit">Fruit</SelectTrigger>
          <SelectContent><SelectItem value="apple">Apple</SelectItem></SelectContent>
        </Select>
      </SurfaceThemeProvider>
    )
    openSelect("Fruit")
    await waitFor(() => expect(portaled("select-content")).toBeTruthy())
    const popup = portaled("select-content")
    expect(classes(popup)).toContain("theme-light")
    // The portal is still on <body>, never inside a scrolling dashboard pane.
    expect(container.contains(popup)).toBe(false)
  })

  it("a Select opened outside a light surface keeps the console's theme", async () => {
    render(
      <Select>
        <SelectTrigger aria-label="Fruit">Fruit</SelectTrigger>
        <SelectContent><SelectItem value="apple">Apple</SelectItem></SelectContent>
      </Select>
    )
    openSelect("Fruit")
    await waitFor(() => expect(portaled("select-content")).toBeTruthy())
    expect(classes(portaled("select-content"))).not.toContain("theme-light")
  })

  it("a dropdown menu opened on a light surface copies theme-light onto its portaled list", async () => {
    render(
      <SurfaceThemeProvider themeClass="theme-light">
        <DropdownMenu>
          <DropdownMenuTrigger>Actions</DropdownMenuTrigger>
          <DropdownMenuContent><DropdownMenuItem>Rename</DropdownMenuItem></DropdownMenuContent>
        </DropdownMenu>
      </SurfaceThemeProvider>
    )
    openMenu("Actions")
    await screen.findByRole("menu")
    expect(classes(portaled("dropdown-menu-content"))).toContain("theme-light")
  })

  it("a dropdown menu opened outside a light surface keeps the console's theme", async () => {
    render(
      <DropdownMenu>
        <DropdownMenuTrigger>Actions</DropdownMenuTrigger>
        <DropdownMenuContent><DropdownMenuItem>Rename</DropdownMenuItem></DropdownMenuContent>
      </DropdownMenu>
    )
    openMenu("Actions")
    await screen.findByRole("menu")
    expect(classes(portaled("dropdown-menu-content"))).not.toContain("theme-light")
  })

  it("a popover opened on a light surface copies theme-light onto its portaled panel", async () => {
    render(
      <SurfaceThemeProvider themeClass="theme-light">
        <Popover>
          <PopoverTrigger>Details</PopoverTrigger>
          <PopoverContent>A detail</PopoverContent>
        </Popover>
      </SurfaceThemeProvider>
    )
    openPopover("Details")
    await waitFor(() => expect(portaled("popover-content")).toBeTruthy())
    expect(classes(portaled("popover-content"))).toContain("theme-light")
  })

  it("a popover opened outside a light surface keeps the console's theme", async () => {
    render(
      <Popover>
        <PopoverTrigger>Details</PopoverTrigger>
        <PopoverContent>A detail</PopoverContent>
      </Popover>
    )
    openPopover("Details")
    await waitFor(() => expect(portaled("popover-content")).toBeTruthy())
    expect(classes(portaled("popover-content"))).not.toContain("theme-light")
  })

  it("switching a surface from dark to light updates an open popover's theme class", async () => {
    function SwitchableSurface() {
      const [light, setLight] = React.useState(false)
      return (
        <SurfaceThemeProvider themeClass={light ? "theme-light" : undefined}>
          <button onClick={() => setLight((v) => !v)}>{light ? "Surface: light" : "Surface: dark"}</button>
          <Popover>
            <PopoverTrigger>Details</PopoverTrigger>
            <PopoverContent>A detail</PopoverContent>
          </Popover>
        </SurfaceThemeProvider>
      )
    }
    render(<SwitchableSurface />)
    openPopover("Details")
    await waitFor(() => expect(portaled("popover-content")).toBeTruthy())
    expect(classes(portaled("popover-content"))).not.toContain("theme-light")

    fireEvent.click(screen.getByRole("button", { name: "Surface: dark" }))
    await waitFor(() => expect(classes(portaled("popover-content"))).toContain("theme-light"))

    fireEvent.click(screen.getByRole("button", { name: "Surface: light" }))
    await waitFor(() => expect(classes(portaled("popover-content"))).not.toContain("theme-light"))
  })
})
