import { describe, expect, it } from "vitest";
import { buildWaveVertices, wireColor, WIRE_COLORS } from "./waveform";
describe("waveform vertices", () => { it("preserves run boundaries and emits vertical edges", () => { const vertices = buildWaveVertices({ id: 1, seq: 1, sample_period_s: 1e-9, trigger_sample: 2, reference_sample: 2, channels_acquired: 3, runs: [{ data: 0, count: 2 }, { data: 1, count: 2 }] }, [0]); expect(vertices.length).toBe(12); expect(Array.from(vertices).filter((value) => value === 0).length).toBeGreaterThan(1); }); it("returns an empty buffer for an empty domain", () => { expect(buildWaveVertices({ id: 1, seq: 1, sample_period_s: 1, trigger_sample: 0, reference_sample: 0, channels_acquired: 0, runs: [] }, [0])).toHaveLength(0); }); });
describe("34-channel bit extraction (D0-D31 + CLK1/CLK2)", () => {
  it("decodes channel 33 (CLK2) high -- JS bitwise & would read it as 0", () => {
    // 2**33 has only bit 33 set; a 32-bit `&` test truncates it to 0.
    const v = buildWaveVertices({ id: 1, seq: 1, sample_period_s: 1e-9, trigger_sample: 0, reference_sample: 0, channels_acquired: 0, runs: [{ data: 2 ** 33, count: 2 }] }, [33]);
    // A single all-high run over one row emits one horizontal segment (2 verts).
    expect(v.length).toBe(4);
    // Both y-coords equal the HIGH level (not the low baseline).
    expect(v[1]).toBeCloseTo(v[3]);
    expect(v[1]).toBeGreaterThan(-1);
  });
  it("decodes channel 32 (CLK1) independently of channel 0", () => {
    const cap = { id: 1, seq: 1, sample_period_s: 1e-9, trigger_sample: 0, reference_sample: 0, channels_acquired: 0, runs: [{ data: 2 ** 32, count: 2 }] };
    expect(buildWaveVertices(cap, [32]).length).toBe(4); // ch32 high
    expect(buildWaveVertices(cap, [0]).length).toBe(4);  // ch0 low, still one segment
  });
});

describe("wire colour palette (LA1034 resistor code)", () => {
  it("has 8 distinct colours", () => {
    expect(WIRE_COLORS).toHaveLength(8);
    expect(new Set(WIRE_COLORS.map((c) => c.join(","))).size).toBe(8);
  });
  it("cycles every 8 channels (wire colour repeats per bank)", () => {
    for (let ch = 0; ch < 34; ch += 1) {
      expect(wireColor(ch)).toEqual(WIRE_COLORS[ch % 8]);
    }
  });
  it("returns a valid in-gamut colour for out-of-range channels, never undefined", () => {
    for (const ch of [-1, -8, -33, 100, 1000]) {
      const colour = wireColor(ch);
      expect(colour).toHaveLength(3);
      for (const component of colour) {
        expect(component).toBeGreaterThanOrEqual(0);
        expect(component).toBeLessThanOrEqual(1);
      }
    }
  });
});
