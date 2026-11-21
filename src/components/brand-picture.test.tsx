import { render } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import { BrandPicture } from "@/components/brand-picture"

describe("BrandPicture (ADM-2)", () => {
  it("draws an empty box of the same size when there is no picture yet", () => {
    const { container } = render(
      <BrandPicture
        src=""
        alt=""
        fallbackSrc="/logo-light.png"
        className="size-8 object-contain"
      />,
    )

    // No picture at all: an `<img>` with an empty source would ask the
    // server for the current page, and the product's own file must not
    // stand in for the workspace's logo (`adm-120`).
    expect(container.querySelector("img")).toBeNull()
    expect(container.innerHTML).not.toContain("/logo-light.png")
    // The slot keeps the size the picture would have had, so the layout
    // does not jump when the logo lands.
    expect(container.firstElementChild?.getAttribute("class")).toBe(
      "size-8 object-contain",
    )
  })

  it("hides the empty box from assistive technology", () => {
    const { container } = render(
      <BrandPicture
        src=""
        alt=""
        fallbackSrc="/logo-light.png"
        className="size-8 object-contain"
      />,
    )

    // The empty box is decoration: it holds no picture and no word, so a
    // screen reader must not read it out (`adm-120`).
    const box = container.firstElementChild
    expect(box?.tagName).toBe("SPAN")
    expect(box?.getAttribute("aria-hidden")).toBe("true")
  })
})
