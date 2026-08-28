import type { RefPoint } from "./cursors";

// The ten measurement types the analyzer supports, matching the device
// software. `needsSource` types measure a channel; `timeOnly` types depend only
// on the two reference points (Interval and Rate).
export interface MeasurementKindInfo {
  value: string;
  label: string;
  needsSource: boolean;
  timeOnly: boolean;
}
export const MEASUREMENT_KINDS: readonly MeasurementKindInfo[] = [
  { value: "frequency", label: "Frequency", needsSource: true, timeOnly: false },
  { value: "period", label: "Period", needsSource: true, timeOnly: false },
  { value: "interval", label: "Interval", needsSource: false, timeOnly: true },
  { value: "rate", label: "Rate", needsSource: false, timeOnly: true },
  { value: "transitions", label: "Transitions", needsSource: true, timeOnly: false },
  { value: "cycles", label: "Cycles", needsSource: true, timeOnly: false },
  { value: "duty", label: "Duty", needsSource: true, timeOnly: false },
  { value: "inverse_duty", label: "1−Duty", needsSource: true, timeOnly: false },
  { value: "positive_width", label: "+Width", needsSource: true, timeOnly: false },
  { value: "negative_width", label: "−Width", needsSource: true, timeOnly: false },
];
export function kindInfo(type: string): MeasurementKindInfo {
  return MEASUREMENT_KINDS.find((kind) => kind.value === type) ?? MEASUREMENT_KINDS[0]!;
}

export interface MeasurementSlot {
  type: string;
  source: number;
  x: RefPoint;
  y: RefPoint;
}
// The four default status-bar measurements, matching the device software's
// example: CLK1 frequency, A-B interval, A-B rate, and CLK2 cycles A to T.
export const DEFAULT_SLOTS: readonly MeasurementSlot[] = [
  { type: "frequency", source: 32, x: "T", y: "R" },
  { type: "interval", source: 0, x: "A", y: "B" },
  { type: "rate", source: 0, x: "A", y: "B" },
  { type: "cycles", source: 33, x: "A", y: "T" },
];

// Interval and Rate are pure time between two references, computed without
// touching the device. Rate follows the vendor definition, 1 / Interval.
export function timeOnlyMeasurement(type: string, left: number, right: number, periodS: number): { value: number | null; unit: string } {
  const seconds = Math.abs(right - left) * periodS;
  if (type === "rate") return { value: seconds > 0 ? 1 / seconds : null, unit: "Hz" };
  return { value: seconds, unit: "s" };
}
