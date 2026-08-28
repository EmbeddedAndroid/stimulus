import { describe, expect, it } from "vitest";
import { operations } from "./generated/ops";
import { matchingOperations } from "./command-palette";

describe("command palette operation coverage", () => {
  it("exposes every canonical operation exactly once", () => {
    const listed = matchingOperations("");
    expect(listed).toHaveLength(459);
    expect(new Set(listed.map((operation) => operation.id)).size).toBe(459);
    expect(listed.map((operation) => operation.id)).toEqual(operations.map((operation) => operation.id));
  });

  it("searches by id, title, and area", () => {
    expect(matchingOperations("acq.single").map((operation) => operation.id)).toContain("acq.single");
    expect(matchingOperations("capture").length).toBeGreaterThan(0);
  });
});
