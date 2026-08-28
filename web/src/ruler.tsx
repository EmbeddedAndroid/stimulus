import { useEffect, useRef, useState } from "react";
import type { Capture } from "./main";

export interface RulerTick {
  fraction: number;
  label: string;
}

// Round a raw interval up to the nearest 1/2/5 times a power of ten, so tick
// spacing lands on human-readable values.
export function niceInterval(raw: number): number {
  if (!(raw > 0)) return 1;
  const base = 10 ** Math.floor(Math.log10(raw));
  const f = raw / base;
  return (f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10) * base;
}

function trim(value: number): string {
  return (Math.round(value * 1000) / 1000).toString();
}

// Time label relative to the trigger, which sits at zero.
export function formatTickTime(seconds: number): string {
  const magnitude = Math.abs(seconds);
  if (magnitude === 0) return "0";
  if (magnitude < 1e-6) return `${trim(seconds * 1e9)} ns`;
  if (magnitude < 1e-3) return `${trim(seconds * 1e6)} µs`;
  if (magnitude < 1) return `${trim(seconds * 1e3)} ms`;
  return `${trim(seconds)} s`;
}

// Evenly spaced time ticks across the visible window [viewStart, viewStart +
// viewCount), measured from the trigger sample. `targetPx` is the desired pixel
// spacing between labels; tick count follows the available width so labels
// never crowd, which also makes ticks finer as the view zooms in.
export function timeTicks(
  viewStart: number,
  viewCount: number,
  periodS: number,
  triggerSample: number,
  widthPx: number,
  targetPx = 90,
): RulerTick[] {
  if (viewCount <= 0 || periodS <= 0 || widthPx <= 0) return [];
  const interval = niceInterval((viewCount * periodS) / Math.max(2, Math.floor(widthPx / targetPx)));
  const tMin = (viewStart - triggerSample) * periodS;
  const tMax = (viewStart + viewCount - 1 - triggerSample) * periodS;
  const ticks: RulerTick[] = [];
  for (let t = Math.ceil(tMin / interval) * interval; t <= tMax + interval * 1e-9; t += interval) {
    const fraction = (triggerSample + t / periodS - viewStart) / viewCount;
    if (fraction >= 0 && fraction <= 1) ticks.push({ fraction, label: formatTickTime(t) });
  }
  return ticks;
}

// A time ruler drawn above the waveform. Tick positions share the waveform's
// visible-window sample mapping, so a tick lines up with its sample at any zoom
// level. The trigger (T) and, when in view and distinct, the reference (R)
// positions are marked.
export function TimeRuler({ capture, viewStart, viewCount }: { capture: Capture; viewStart: number; viewCount: number }) {
  const container = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const element = container.current;
    if (element === null) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, []);
  const ticks = timeTicks(viewStart, viewCount, capture.sample_period_s, capture.trigger_sample, width);
  const percent = (sample: number) => ((sample - viewStart) / viewCount) * 100;
  const inView = (sample: number) => sample >= viewStart && sample <= viewStart + viewCount;
  return (
    <div className="time-ruler" ref={container} aria-label="Time ruler">
      {ticks.map((tick) => (
        <span className="ruler-tick" key={tick.label} style={{ left: `${tick.fraction * 100}%` }}>
          <i />
          <small>{tick.label}</small>
        </span>
      ))}
      {inView(capture.trigger_sample) && (
        <span className="ruler-mark trigger" style={{ left: `${percent(capture.trigger_sample)}%` }} title="Trigger">T</span>
      )}
      {capture.reference_sample !== capture.trigger_sample && inView(capture.reference_sample) && (
        <span className="ruler-mark reference" style={{ left: `${percent(capture.reference_sample)}%` }} title="Reference">R</span>
      )}
    </div>
  );
}
