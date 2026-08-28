import { describe, expect, it } from "vitest";
import { DEFAULT_SLOTS, kindInfo, MEASUREMENT_KINDS, timeOnlyMeasurement } from "./measure";

describe("measurement kinds", () => {
  it("covers the ten measurement types", () => {
    expect(MEASUREMENT_KINDS).toHaveLength(10);
    expect(MEASUREMENT_KINDS.map((kind) => kind.value)).toContain("frequency");
    expect(kindInfo("interval").timeOnly).toBe(true);
    expect(kindInfo("frequency").needsSource).toBe(true);
    expect(kindInfo("interval").needsSource).toBe(false);
  });
});

describe("timeOnlyMeasurement", () => {
  it("interval is the time between references", () => {
    const measurement = timeOnlyMeasurement("interval", 100, 300, 1e-6);
    expect(measurement.unit).toBe("s");
    expect(measurement.value).toBeCloseTo(2e-4, 12);
  });
  it("rate is one over the interval", () => {
    expect(timeOnlyMeasurement("rate", 100, 300, 1e-6).value).toBeCloseTo(5000, 6);
  });
  it("rate is null for coincident references", () => {
    expect(timeOnlyMeasurement("rate", 100, 100, 1e-6).value).toBeNull();
  });
});

describe("default slots", () => {
  it("match the vendor examples", () => {
    expect(DEFAULT_SLOTS.map((slot) => slot.type)).toEqual(["frequency", "interval", "rate", "cycles"]);
    expect(DEFAULT_SLOTS[0]!.source).toBe(32);
    expect(DEFAULT_SLOTS[3]!.x).toBe("A");
    expect(DEFAULT_SLOTS[3]!.y).toBe("T");
  });
});
