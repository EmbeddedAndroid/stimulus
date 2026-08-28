import { describe, expect, it } from "vitest";
import { centerView, clampView, findEdge, fullView, panView, ZOOM_STEP, zoomView } from "./view";

describe("fullView / clampView", () => {
  it("shows the whole capture and never leaves bounds", () => {
    expect(fullView(1000)).toEqual({ start: 0, count: 1000 });
    expect(clampView({ start: -50, count: 2000 }, 1000)).toEqual({ start: 0, count: 1000 });
    expect(clampView({ start: 990, count: 100 }, 1000)).toEqual({ start: 900, count: 100 });
    expect(clampView({ start: 0, count: 0 }, 1000)).toEqual({ start: 0, count: 1 });
  });
});

describe("zoomView", () => {
  it("zooms in about the focus sample and keeps it fixed on screen", () => {
    const zoomed = zoomView({ start: 0, count: 1000 }, 1 / ZOOM_STEP, 500, 1000);
    expect(zoomed.count).toBe(800);
    expect((500 - zoomed.start) / zoomed.count).toBeCloseTo(0.5, 5);
  });
  it("clamps zoom-out at the full capture", () => {
    expect(zoomView({ start: 0, count: 1000 }, ZOOM_STEP, 500, 1000)).toEqual({ start: 0, count: 1000 });
  });
});

describe("panView / centerView", () => {
  it("pans and clamps at the edges", () => {
    expect(panView({ start: 100, count: 200 }, 50, 1000)).toEqual({ start: 150, count: 200 });
    expect(panView({ start: 900, count: 200 }, 500, 1000)).toEqual({ start: 800, count: 200 });
  });
  it("centers the window on a sample", () => {
    expect(centerView({ start: 0, count: 200 }, 500, 1000)).toEqual({ start: 400, count: 200 });
  });
});

describe("findEdge", () => {
  const runs = [
    { data: 0b00, count: 10 },
    { data: 0b01, count: 10 },
    { data: 0b00, count: 10 },
  ];
  it("finds the next transition on a channel after a sample", () => {
    expect(findEdge(runs, 0, 0, 1)).toBe(10);
    expect(findEdge(runs, 0, 10, 1)).toBe(20);
    expect(findEdge(runs, 0, 20, 1)).toBeNull();
  });
  it("finds the previous transition before a sample", () => {
    expect(findEdge(runs, 0, 25, -1)).toBe(20);
    expect(findEdge(runs, 0, 15, -1)).toBe(10);
    expect(findEdge(runs, 0, 5, -1)).toBeNull();
  });
  it("ignores channels that never change", () => {
    expect(findEdge(runs, 5, 0, 1)).toBeNull();
  });
});
