// A visible slice of a capture: `start` is the leftmost sample, `count` the
// number of samples shown. Rendering maps this window to the canvas width, so
// zooming and scrolling are pure transforms of the window.

export interface ViewWindow {
  start: number;
  count: number;
}

// Matches the device software's 1.25x-per-step zoom.
export const ZOOM_STEP = 1.25;

export function fullView(total: number): ViewWindow {
  return { start: 0, count: Math.max(1, total) };
}

// Keep the window within [0, total] with at least one sample visible.
export function clampView(view: ViewWindow, total: number): ViewWindow {
  const max = Math.max(1, total);
  const count = Math.max(1, Math.min(max, Math.round(view.count)));
  const start = Math.max(0, Math.min(max - count, Math.round(view.start)));
  return { start, count };
}

// Scale the window by `factor` (<1 zooms in) while holding `focus` at the same
// on-screen fraction, so the point under the cursor stays put.
export function zoomView(view: ViewWindow, factor: number, focus: number, total: number): ViewWindow {
  const count = view.count * factor;
  const fraction = view.count > 0 ? (focus - view.start) / view.count : 0.5;
  return clampView({ start: focus - fraction * count, count }, total);
}

export function panView(view: ViewWindow, deltaSamples: number, total: number): ViewWindow {
  return clampView({ start: view.start + deltaSamples, count: view.count }, total);
}

// Recenter the window on a sample without changing the zoom level.
export function centerView(view: ViewWindow, sample: number, total: number): ViewWindow {
  return clampView({ start: sample - view.count / 2, count: view.count }, total);
}

export interface Rle {
  data: number;
  count: number;
}

// Sample index of the next (dir 1) or previous (dir -1) transition on a channel
// relative to `fromSample`, or null when there is none. A transition is a run
// boundary where the channel bit differs from the preceding run.
export function findEdge(runs: Rle[], channel: number, fromSample: number, dir: 1 | -1): number | null {
  let sample = 0;
  let previous: number | null = null;
  const edges: number[] = [];
  for (const run of runs) {
    const level = Math.floor(run.data / 2 ** channel) % 2;
    if (previous !== null && previous !== level) edges.push(sample);
    previous = level;
    sample += run.count;
  }
  if (dir === 1) return edges.find((edge) => edge > fromSample) ?? null;
  for (let index = edges.length - 1; index >= 0; index -= 1) {
    const edge = edges[index];
    if (edge !== undefined && edge < fromSample) return edge;
  }
  return null;
}
