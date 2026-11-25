import { describe, expect, test } from "bun:test";
import { levelAtLeast, offerFor } from "./access-level";

describe("the level a dashboard offers", () => {
  test("a higher level includes the lower ones and an unknown word includes nothing", () => {
    expect(levelAtLeast("manage", "edit")).toBe(true);
    expect(levelAtLeast("edit", "manage")).toBe(false);
    expect(levelAtLeast("view", "edit")).toBe(false);
    expect(levelAtLeast("owner", "view")).toBe(false);
    expect(levelAtLeast(undefined, "view")).toBe(false);
  });

  test("view offers neither editing nor managing, even to somebody who holds dashboard:write", () => {
    expect(offerFor("view", true)).toEqual({ canEdit: false, canManage: false });
    expect(offerFor("edit", false)).toEqual({ canEdit: true, canManage: false });
    expect(offerFor("manage", false)).toEqual({ canEdit: true, canManage: true });
  });

  test("an answer without a level falls back to the global permission", () => {
    expect(offerFor(undefined, true)).toEqual({ canEdit: true, canManage: true });
    expect(offerFor(undefined, false)).toEqual({ canEdit: false, canManage: false });
  });
});
