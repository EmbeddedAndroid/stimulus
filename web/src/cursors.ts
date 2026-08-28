// Measurement cursors A..F, plus the fixed trigger (T) and reference (R)
// positions. Cursors are held client-side as sample indices; measurements
// resolve a reference to a sample and call the canonical capture.measure
// operation, so nothing here needs device or session state.

export type CursorId = "A" | "B" | "C" | "D" | "E" | "F";
export const CURSOR_IDS: readonly CursorId[] = ["A", "B", "C", "D", "E", "F"];

// GL colors (0..1) for the waveform overlay.
export const CURSOR_GL: Record<CursorId, [number, number, number]> = {
  A: [0.24, 0.84, 1.0],
  B: [1.0, 0.35, 0.8],
  C: [0.55, 0.9, 0.35],
  D: [1.0, 0.6, 0.2],
  E: [0.7, 0.5, 1.0],
  F: [0.3, 0.85, 0.7],
};
// Matching CSS colors for the ruler marks and readouts.
export const CURSOR_CSS: Record<CursorId, string> = {
  A: "#3cd6ff",
  B: "#ff59cc",
  C: "#8ce65a",
  D: "#ff9933",
  E: "#b380ff",
  F: "#4dd9b3",
};

export type Cursors = Partial<Record<CursorId, number>>;

// A measurement reference: a cursor id, the trigger (T), or the reference (R).
export type RefPoint = CursorId | "T" | "R";
export const REF_POINTS: readonly RefPoint[] = ["T", "R", ...CURSOR_IDS];

// Resolve a reference to a sample index, or null when the cursor is not placed.
export function resolveRef(ref: RefPoint, triggerSample: number, referenceSample: number, cursors: Cursors): number | null {
  if (ref === "T") return triggerSample;
  if (ref === "R") return referenceSample;
  return cursors[ref] ?? null;
}

export interface Delta {
  samples: number;
  seconds: number;
  hz: number | null;
}

// Time and rate between two cursors, or null unless both are placed.
export function cursorDelta(cursors: Cursors, a: CursorId, b: CursorId, periodS: number): Delta | null {
  const sa = cursors[a];
  const sb = cursors[b];
  if (sa === undefined || sb === undefined) return null;
  const samples = Math.abs(sb - sa);
  const seconds = samples * periodS;
  return { samples, seconds, hz: seconds > 0 ? 1 / seconds : null };
}
