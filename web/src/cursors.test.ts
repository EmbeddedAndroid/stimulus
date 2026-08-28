import { describe, expect, it } from "vitest";
import { cursorDelta, CURSOR_IDS, resolveRef } from "./cursors";

describe("resolveRef", () => {
  it("resolves trigger, reference, and placed cursors", () => {
    const cursors = { A: 100, C: 300 };
    expect(resolveRef("T", 50, 80, cursors)).toBe(50);
    expect(resolveRef("R", 50, 80, cursors)).toBe(80);
    expect(resolveRef("A", 50, 80, cursors)).toBe(100);
    expect(resolveRef("C", 50, 80, cursors)).toBe(300);
  });
  it("returns null for an unplaced cursor", () => {
    expect(resolveRef("B", 50, 80, { A: 100 })).toBeNull();
  });
});

describe("cursorDelta", () => {
  it("measures time and rate between two placed cursors", () => {
    const delta = cursorDelta({ A: 100, B: 300 }, "A", "B", 1e-6);
    expect(delta).not.toBeNull();
    expect(delta?.samples).toBe(200);
    expect(delta?.seconds).toBeCloseTo(2e-4, 12);
    expect(delta?.hz).toBeCloseTo(5000, 6);
  });
  it("is null unless both cursors are placed", () => {
    expect(cursorDelta({ A: 100 }, "A", "B", 1e-6)).toBeNull();
  });
  it("reports no rate for coincident cursors", () => {
    expect(cursorDelta({ A: 100, B: 100 }, "A", "B", 1e-6)?.hz).toBeNull();
  });
});

describe("cursor ids", () => {
  it("are the six letters A..F", () => {
    expect(CURSOR_IDS).toEqual(["A", "B", "C", "D", "E", "F"]);
  });
});
