import { describe, expect, it } from "vitest";
import { formatTickTime, niceInterval, timeTicks } from "./ruler";

describe("niceInterval", () => {
  it("rounds up to 1/2/5 times a power of ten", () => {
    expect(niceInterval(0.9)).toBeCloseTo(1);
    expect(niceInterval(2.5)).toBeCloseTo(2);
    expect(niceInterval(6)).toBeCloseTo(5);
    expect(niceInterval(2e-7)).toBeCloseTo(2e-7);
  });
});

describe("formatTickTime", () => {
  it("labels the trigger as zero and scales the unit", () => {
    expect(formatTickTime(0)).toBe("0");
    expect(formatTickTime(2e-9)).toBe("2 ns");
    expect(formatTickTime(-1.5e-6)).toBe("-1.5 µs");
    expect(formatTickTime(3e-3)).toBe("3 ms");
  });
});

describe("timeTicks", () => {
  it("spans the visible window with a tick at the trigger", () => {
    const ticks = timeTicks(0, 1000, 1e-9, 500, 900);
    expect(ticks.length).toBeGreaterThan(2);
    expect(ticks.every((tick) => tick.fraction >= 0 && tick.fraction <= 1)).toBe(true);
    const zero = ticks.find((tick) => tick.label === "0");
    expect(zero?.fraction).toBeCloseTo(0.5, 5);
  });

  it("puts the trigger at the window edge when zoomed to the trigger", () => {
    const ticks = timeTicks(500, 200, 1e-9, 500, 900);
    const zero = ticks.find((tick) => tick.label === "0");
    expect(zero?.fraction).toBeCloseTo(0, 5);
  });

  it("returns nothing without a window or width", () => {
    expect(timeTicks(0, 0, 1e-9, 0, 900)).toEqual([]);
    expect(timeTicks(0, 1000, 1e-9, 0, 0)).toEqual([]);
  });
});
