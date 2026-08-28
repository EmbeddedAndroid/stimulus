import { describe, expect, it } from "vitest";
import { isLiveState, newestId, nextSelectedId, pollIntervalMs, sortedByNewest } from "./live";

describe("isLiveState", () => {
  it("is false when idle, ready, halted, or null", () => {
    expect(isLiveState(null)).toBe(false);
    for (const state of ["idle", "ready", "halted"]) {
      expect(isLiveState({ state, recurring: false })).toBe(false);
    }
  });
  it("is true while a capture is filling or a recurring run is active", () => {
    for (const state of ["prefill", "armed", "postfill"]) {
      expect(isLiveState({ state, recurring: false })).toBe(true);
    }
    expect(isLiveState({ state: "idle", recurring: true })).toBe(true);
  });
});

describe("pollIntervalMs", () => {
  it("polls faster when live and still polls when idle", () => {
    expect(pollIntervalMs(true)).toBeLessThan(pollIntervalMs(false));
    expect(pollIntervalMs(false)).toBeGreaterThan(0);
  });
});

describe("newestId / sortedByNewest", () => {
  const caps = [{ id: 4 }, { id: 9 }, { id: 5 }];
  it("finds the highest id regardless of order", () => {
    expect(newestId(caps)).toBe(9);
    expect(newestId([])).toBe(null);
  });
  it("sorts newest first without mutating the input", () => {
    const input = [{ id: 1 }, { id: 3 }, { id: 2 }];
    expect(sortedByNewest(input).map((c) => c.id)).toEqual([3, 2, 1]);
    expect(input.map((c) => c.id)).toEqual([1, 3, 2]);
  });
});

describe("nextSelectedId (follow-latest)", () => {
  const caps = [{ id: 4 }, { id: 9 }, { id: 5 }];
  it("follows the newest capture when following", () => {
    expect(nextSelectedId(caps, 4, true)).toBe(9);
    expect(nextSelectedId(caps, null, true)).toBe(9);
  });
  it("keeps the pinned capture when not following", () => {
    expect(nextSelectedId(caps, 4, false)).toBe(4);
  });
  it("returns null for an empty capture list", () => {
    expect(nextSelectedId([], null, true)).toBe(null);
    expect(nextSelectedId([], 3, true)).toBe(3);
  });
});
