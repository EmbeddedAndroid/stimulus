import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { opCall } from "./generated/api";
import type { OperationId } from "./generated/ops";
import { CommandPalette } from "./command-palette";
import { WaveformTimeline, totalSamples } from "./waveform";
import { TimeRuler } from "./ruler";
import { centerView, clampView, findEdge, fullView, panView, ZOOM_STEP, zoomView, type ViewWindow } from "./view";
import { CURSOR_CSS, CURSOR_GL, CURSOR_IDS, cursorDelta, REF_POINTS, resolveRef, type CursorId, type Cursors } from "./cursors";
import { DEFAULT_SLOTS, kindInfo, MEASUREMENT_KINDS, timeOnlyMeasurement, type MeasurementSlot } from "./measure";
import { isLiveState, nextSelectedId, pollIntervalMs, sortedByNewest } from "./live";
import "./style.css";

export interface Run { data: number; count: number }
export interface Capture { id: number; seq: number; sample_period_s: number; trigger_sample: number; reference_sample: number; channels_acquired: number; runs: Run[] }
interface DeviceStatus { state: string; backend: string; fpga: string; fpga_image_id: number | null; usb_error_count: number; needs_replug?: boolean; error?: string | null }
interface AcquisitionStatus { state: string; samples: number; acq_count: number; recurring: boolean; buffer_fill_pct: number }
interface SampleSettings { mode: string; rate_index: number; rate_hz: number; compression: boolean; pretrigger_pct: number }
interface CaptureEnvelope { capture: Capture }
interface Group { id: string; name: string; wires: number[]; radix: string }
const channels = Array.from({ length: 34 }, (_, index) => index);
// Channel used for next/previous-edge navigation until a measurement source is
// selected (Phase 4); D0 by convention.
const NAV_CHANNEL = 0;

function App() {
  const [device, setDevice] = useState<DeviceStatus | null>(null);
  const [acquisition, setAcquisition] = useState<AcquisitionStatus | null>(null);
  const [sample, setSample] = useState<SampleSettings | null>(null);
  const [captures, setCaptures] = useState<Capture[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  useEffect(() => { followingRef.current = following; }, [following]);
  const [cursors, setCursors] = useState<Cursors>({});
  const [activeCursor, setActiveCursor] = useState<CursorId>("A");
  const [view, setView] = useState<ViewWindow | null>(null);
  // Logic threshold is a device setting not returned by sample.get, so it is
  // held locally (default 1.65 V, matching the daemon's default).
  const [thresholdV, setThresholdV] = useState(1.65);
  const [measurements, setMeasurements] = useState<MeasurementSlot[]>(() => DEFAULT_SLOTS.map((slot) => ({ ...slot })));
  const [measureResults, setMeasureResults] = useState<string[]>([]);
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const [signalNames, setSignalNames] = useState<Record<number, string>>({});
  // The wire whose name is being edited, so the periodic refresh does not
  // overwrite an in-progress edit with the server value.
  const editingWireRef = useRef<number | null>(null);
  const renameSignal = async (wire: number, name: string) => {
    if (name.trim() === "") return;
    try { await opCall("signals.rename", { wire, name }); } catch { /* keep the local edit */ }
  };
  const [groups, setGroups] = useState<Group[]>([]);
  const [groupValues, setGroupValues] = useState<Record<string, string>>({});
  const [newGroupName, setNewGroupName] = useState("");
  const [newGroupWires, setNewGroupWires] = useState("");
  const loadGroups = useCallback(async () => {
    try { const result = await opCall<{ groups: Group[] }>("groups.list"); setGroups(result.groups); } catch { /* keep current groups */ }
  }, []);
  const createGroup = async () => {
    const wires = parseWires(newGroupWires);
    if (newGroupName.trim() === "" || wires.length === 0) return;
    try { await opCall("groups.create", { name: newGroupName.trim(), wires, radix: "hex" }); setNewGroupName(""); setNewGroupWires(""); await loadGroups(); } catch { /* validation reported inline */ }
  };
  const deleteGroup = async (id: string) => {
    try { await opCall("groups.delete", { id }); await loadGroups(); } catch { /* ignore */ }
  };
  // Cursors and the view window are capture-relative, so reset them when the
  // shown capture changes (e.g. auto-following a new one); a null view means
  // "fit the whole capture".
  useEffect(() => { setCursors({}); setView(null); }, [selectedId]);
  const [importPath, setImportPath] = useState("/usr/local/share/logicport/examples/Quickstart.LPF");
  const refresh = useCallback(async () => {
    const [deviceResult, acquisitionResult, sampleResult, captureResult, signalsResult, groupsResult] = await Promise.all([
      opCall<DeviceStatus>("device.status"), opCall<AcquisitionStatus>("acq.status"),
      opCall<{ sample: SampleSettings }>("sample.get"), opCall<{ captures: CaptureEnvelope[] }>("capture.list", { limit: 20 }),
      opCall<{ signals: { wire: number; name: string }[] }>("signals.list"), opCall<{ groups: Group[] }>("groups.list"),
    ]);
    const nextCaptures = captureResult.captures.map((entry) => entry.capture);
    setDevice(deviceResult); setAcquisition(acquisitionResult); setSample(sampleResult.sample); setCaptures(nextCaptures);
    setSelectedId((current) => nextSelectedId(nextCaptures, current, followingRef.current));
    setGroups(groupsResult.groups);
    // Signal names track the project, except for a name being edited right now.
    const names: Record<number, string> = {};
    for (const signal of signalsResult.signals) names[signal.wire] = signal.name;
    setSignalNames((previous) => {
      const editing = editingWireRef.current;
      if (editing !== null && previous[editing] !== undefined) names[editing] = previous[editing];
      return names;
    });
  }, []);
  useEffect(() => { void refresh().catch((cause: unknown) => setError(message(cause))); }, [refresh]);
  const live = isLiveState(acquisition);
  const period = pollIntervalMs(live);
  // Always poll so fresh captures surface on their own; briskly while a capture
  // is filling, calmly when idle. No manual refresh is ever needed.
  useEffect(() => {
    const timer = window.setInterval(() => {
      void refresh().catch((cause: unknown) => setError(message(cause)));
    }, period);
    return () => window.clearInterval(timer);
  }, [period, refresh]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") { event.preventDefault(); setPaletteOpen(true); }
      if (event.key === "Escape") setPaletteOpen(false);
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, []);
  const invoke = useCallback(async (id: OperationId, params: unknown = {}) => {
    setBusy(true); setError(null);
    try { await opCall(id, params); await refresh(); } catch (cause: unknown) { setError(message(cause)); } finally { setBusy(false); }
  }, [refresh]);
  const ordered = useMemo(() => sortedByNewest(captures), [captures]);
  const selected = useMemo(() => captures.find((capture) => capture.id === selectedId) ?? ordered[0] ?? null, [captures, ordered, selectedId]);
  const total = selected === null ? 0 : expandedLength(selected);
  // The visible sample window; a null `view` fits the whole capture. Zoom and
  // scroll are pure transforms of this window (see view.ts). The canonical
  // view.* operations expose the same navigation to agents over REST and MCP.
  const effView = useMemo<ViewWindow | null>(() => (total > 0 ? clampView(view ?? fullView(total), total) : null), [view, total]);
  const visibleChannels = useMemo(() => channels.filter((channel) => !hidden.has(channel)), [hidden]);
  const activeSample = selected === null ? null : cursors[activeCursor] ?? null;
  const placeActive = (sample: number) => setCursors((current) => ({ ...current, [activeCursor]: sample }));
  const clearCursor = (id: CursorId) => setCursors((current) => { const next = { ...current }; delete next[id]; return next; });
  const focusSample = activeSample ?? (effView === null ? 0 : Math.round(effView.start + effView.count / 2));
  const zoomIn = () => { if (effView !== null) setView(zoomView(effView, 1 / ZOOM_STEP, focusSample, total)); };
  const zoomOut = () => { if (effView !== null) setView(zoomView(effView, ZOOM_STEP, focusSample, total)); };
  const fitView = () => setView(fullView(total));
  const gotoEdge = (direction: 1 | -1) => {
    if (selected === null || effView === null) return;
    const target = findEdge(selected.runs, NAV_CHANNEL, focusSample, direction);
    if (target !== null) { placeActive(target); setView(centerView(effView, target, total)); }
  };
  const waveCursors = useMemo(() => CURSOR_IDS.flatMap((id) => { const sample = cursors[id]; return sample === undefined ? [] : [{ sample, color: CURSOR_GL[id] }]; }), [cursors]);
  const rulerCursors = useMemo(() => CURSOR_IDS.flatMap((id) => { const sample = cursors[id]; return sample === undefined ? [] : [{ id, sample, css: CURSOR_CSS[id] }]; }), [cursors]);
  const delta = selected === null ? null : cursorDelta(cursors, "A", "B", selected.sample_period_s);
  // Keyboard navigation reads live state through a ref so the listener is
  // registered once: +/- zoom, 0 fits, arrows scroll, n/p jump edges.
  const navRef = useRef({ effView, total, selected, activeSample, activeCursor });
  navRef.current = { effView, total, selected, activeSample, activeCursor };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const tag = (event.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      const { effView: v, total: t, selected: s, activeSample: a, activeCursor: id } = navRef.current;
      if (v === null || t <= 0) return;
      const focus = a ?? Math.round(v.start + v.count / 2);
      const jump = (direction: 1 | -1) => {
        if (s === null) return;
        const target = findEdge(s.runs, NAV_CHANNEL, focus, direction);
        if (target !== null) { setCursors((current) => ({ ...current, [id]: target })); setView(centerView(v, target, t)); }
      };
      switch (event.key) {
        case "+": case "=": event.preventDefault(); setView(zoomView(v, 1 / ZOOM_STEP, focus, t)); break;
        case "-": case "_": setView(zoomView(v, ZOOM_STEP, focus, t)); break;
        case "0": setView(fullView(t)); break;
        case "ArrowLeft": event.preventDefault(); setView(panView(v, -Math.round(v.count * 0.15), t)); break;
        case "ArrowRight": event.preventDefault(); setView(panView(v, Math.round(v.count * 0.15), t)); break;
        case "n": case "N": jump(1); break;
        case "p": case "P": jump(-1); break;
        default: break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const capId = selected?.id ?? null;
  const capTrig = selected?.trigger_sample ?? 0;
  const capRef = selected?.reference_sample ?? 0;
  const capPeriod = selected?.sample_period_s ?? 0;
  const updateSlot = (index: number, patch: Partial<MeasurementSlot>) => setMeasurements((slots) => slots.map((slot, i) => (i === index ? { ...slot, ...patch } : slot)));
  // Recompute the four status-bar measurements when the capture, cursors, or
  // slot configuration change. Interval and Rate are pure time; the rest call
  // the canonical capture.measure operation. Captures are immutable by id, so
  // depending on the id (not the object) avoids recomputing on every poll.
  useEffect(() => {
    if (capId === null) { setMeasureResults([]); return; }
    let cancelled = false;
    void (async () => {
      const results = await Promise.all(measurements.map(async (slot) => {
        const left = resolveRef(slot.x, capTrig, capRef, cursors);
        const right = resolveRef(slot.y, capTrig, capRef, cursors);
        if (kindInfo(slot.type).timeOnly) {
          if (left === null || right === null) return "—";
          const { value, unit } = timeOnlyMeasurement(slot.type, left, right, capPeriod);
          return formatMeasurement(value, unit);
        }
        try {
          const bounds = left !== null && right !== null ? { left: Math.min(left, right), right: Math.max(left, right) } : {};
          const measurement = await opCall<{ value: number | null; unit: string }>("capture.measure", { capture_id: capId, type: slot.type, source: channelLabel(slot.source), ...bounds });
          return formatMeasurement(measurement.value, measurement.unit);
        } catch { return "—"; }
      }));
      if (!cancelled) setMeasureResults(results);
    })();
    return () => { cancelled = true; };
  }, [measurements, cursors, capId, capTrig, capRef, capPeriod]);
  // Grouped bus values are read at the active cursor via group.value_at.
  useEffect(() => {
    if (capId === null || activeSample === null || groups.length === 0) { setGroupValues({}); return; }
    let cancelled = false;
    void (async () => {
      const entries = await Promise.all(groups.map(async (group) => {
        try {
          const result = await opCall<{ formatted?: string; value?: string | number }>("group.value_at", { id: group.id, sample: activeSample });
          return [group.id, String(result.formatted ?? result.value ?? "—")] as const;
        } catch { return [group.id, "—"] as const; }
      }));
      if (!cancelled) setGroupValues(Object.fromEntries(entries));
    })();
    return () => { cancelled = true; };
  }, [groups, capId, activeSample]);
  return <div className="app-shell">
    <header className="topbar"><div className="brand"><span className="brand-mark">MP</span><strong>MagicPort</strong></div><div className="transport-controls"><button className="primary" disabled={busy} onClick={() => void invoke("acq.single")}>▶ Capture</button><button disabled={busy} onClick={() => void invoke("acq.halt")}>■ Stop</button><button disabled={busy} onClick={() => void invoke("acq.trigger_immediate")}>Trigger now</button><button disabled={busy} onClick={() => setImportOpen(true)}>Import LPF</button><button disabled={busy} onClick={() => setPaletteOpen(true)}>Operations <kbd>Ctrl K</kbd></button></div><div className="device-pill" data-state={device?.state ?? "offline"}><span className="status-dot" /><span>{device?.backend ?? "connecting"}</span><small>{device == null || device.fpga_image_id == null ? "" : `FPGA ${hex(device.fpga_image_id)}`}</small></div></header>
    <aside className="left-panel"><section><h2>Acquisition</h2><label>Mode<select value={sample?.mode ?? "timing"} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, mode: event.target.value } })}><option value="timing">Timing</option><option value="state">State</option></select></label><label>Sample rate<select value={sample?.rate_index ?? 0} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, rate_index: Number(event.target.value) } })}>{rateLabels.map((label, index) => <option key={label} value={index}>{label}</option>)}</select></label><label className="check"><input type="checkbox" checked={sample?.compression ?? false} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, compression: event.target.checked } })} />Compression</label><label>Logic threshold<input className="threshold" type="number" step="0.05" min="-6" max="6" value={thresholdV} disabled={busy} onChange={(event) => { const volts = Number(event.target.value); setThresholdV(volts); void invoke("threshold.set", { volts }); }} /></label><label>Pre-trigger buffer<select value={sample?.pretrigger_pct ?? 50} disabled={busy} onChange={(event) => void invoke("sample.pretrigger_buffer.set", { percent: Number(event.target.value) })}>{[0, 25, 50, 75, 100].map((percent) => <option key={percent} value={percent}>{percent}%</option>)}</select></label></section><section><h2>Channels</h2><div className="channel-list">{channels.map((channel) => <div className="channel-row" key={channel}><input type="checkbox" className="ch-vis" checked={!hidden.has(channel)} title={`Show ${channelLabel(channel)}`} onChange={(event) => setHidden((current) => { const next = new Set(current); if (event.target.checked) next.delete(channel); else next.add(channel); return next; })} /><span className={channel < 32 ? `swatch c${channel % 8}` : "swatch clk"} /><b>{channelLabel(channel)}</b><input className="signal-name" value={signalNames[channel] ?? channelLabel(channel)} title={`Signal name for ${channelLabel(channel)}`} onFocus={() => { editingWireRef.current = channel; }} onChange={(event) => setSignalNames((current) => ({ ...current, [channel]: event.target.value }))} onBlur={(event) => { editingWireRef.current = null; void renameSignal(channel, event.target.value); }} /><span>{selected === null ? "—" : channelValue(selected, channel, activeSample)}</span></div>)}</div></section></aside>
    <main className="workspace"><div className="timeline-toolbar"><div><strong>{selected === null ? "No capture" : `Capture ${selected.id}`}</strong><span>{selected === null ? "Run an acquisition to begin" : `${expandedLength(selected).toLocaleString()} samples · ${formatPeriod(selected.sample_period_s)}`}</span>{selected !== null && <div className="cursor-bar">{CURSOR_IDS.map((id) => <button key={id} className={`cur${id === activeCursor ? " active" : ""}${cursors[id] !== undefined ? " placed" : ""}`} style={{ color: CURSOR_CSS[id], borderColor: id === activeCursor ? CURSOR_CSS[id] : undefined }} title={cursors[id] !== undefined ? `Cursor ${id} placed — click to select, again to clear` : `Select cursor ${id}, then click the waveform`} onClick={() => { if (activeCursor === id && cursors[id] !== undefined) clearCursor(id); else setActiveCursor(id); }}>{id}</button>)}{delta !== null && <span className="cursor-delta" title="A to B">Δ {formatTime(delta.seconds)}{delta.hz !== null ? ` · ${formatHz(delta.hz)}` : ""}</span>}</div>}</div>{selected !== null && effView !== null && <div className="view-controls"><button title="Zoom out (−)" onClick={zoomOut}>−</button><button title="Zoom in (+)" onClick={zoomIn}>+</button><button title="Fit (0)" onClick={fitView}>⤢</button><button title="Previous edge (p)" onClick={() => gotoEdge(-1)}>◁</button><button title="Next edge (n)" onClick={() => gotoEdge(1)}>▷</button><span className="view-span" title="Visible samples">{effView.count.toLocaleString()} / {total.toLocaleString()}</span></div>}<div className="buffer"><span style={{ width: `${acquisition?.buffer_fill_pct ?? 0}%` }} /></div><output>{live && <span className="live-dot" aria-label="live" />}{acquisition?.state ?? "idle"}</output></div>{selected === null ? <div className="empty-state"><div className="empty-icon">⌁</div><h1>Ready to capture</h1><p>Connect signals D0–D31 and start a single acquisition.</p></div> : <div className="plot-area"><TimeRuler capture={selected} viewStart={effView?.start ?? 0} viewCount={effView?.count ?? totalSamples(selected)} cursors={rulerCursors} /><WaveformTimeline capture={selected} channels={visibleChannels} viewStart={effView?.start ?? 0} viewCount={effView?.count ?? totalSamples(selected)} cursors={waveCursors} onPlace={placeActive} /></div>}{error !== null && <div className="error-banner" role="alert">{error}<button onClick={() => setError(null)}>×</button></div>}</main>
    <aside className="right-panel"><section><h2>Session</h2><dl><dt>Status</dt><dd>{acquisition?.state ?? "—"}</dd><dt>Captures</dt><dd>{acquisition?.acq_count ?? 0}</dd><dt>USB errors</dt><dd>{device?.usb_error_count ?? 0}</dd></dl></section>{selected !== null && <section className="measure-panel"><h2>Measurements</h2><div className="measure-list">{measurements.map((slot, index) => { const info = kindInfo(slot.type); return <div className="measure-row" key={index}><select value={slot.type} title="Measurement type" onChange={(event) => updateSlot(index, { type: event.target.value })}>{MEASUREMENT_KINDS.map((kind) => <option key={kind.value} value={kind.value}>{kind.label}</option>)}</select>{info.needsSource && <select value={slot.source} title="Source channel" onChange={(event) => updateSlot(index, { source: Number(event.target.value) })}>{channels.map((channel) => <option key={channel} value={channel}>{channelLabel(channel)}</option>)}</select>}<select value={slot.x} title="From" onChange={(event) => updateSlot(index, { x: event.target.value as MeasurementSlot["x"] })}>{REF_POINTS.map((point) => <option key={point} value={point}>{point}</option>)}</select><span className="arrow">→</span><select value={slot.y} title="To" onChange={(event) => updateSlot(index, { y: event.target.value as MeasurementSlot["y"] })}>{REF_POINTS.map((point) => <option key={point} value={point}>{point}</option>)}</select><b className="measure-value">{measureResults[index] ?? "…"}</b></div>; })}</div></section>}
    {selected !== null && <section className="cursors-panel"><h2>Cursors</h2>{CURSOR_IDS.every((id) => cursors[id] === undefined) ? <p className="muted">Select A–F, then click the waveform</p> : <dl className="cursor-list">{CURSOR_IDS.flatMap((id) => { const s = cursors[id]; if (s === undefined) return []; return [<div className="cursor-item" key={id}><dt style={{ color: CURSOR_CSS[id] }}>{id}</dt><dd>{formatTime((s - selected.trigger_sample) * selected.sample_period_s)}<button className="mini" title={`Clear cursor ${id}`} onClick={() => clearCursor(id)}>✕</button></dd></div>]; })}</dl>}{delta !== null && <p className="cursor-delta-row">Δ A→B: {formatTime(delta.seconds)}{delta.hz !== null ? ` · ${formatHz(delta.hz)}` : ""}</p>}</section>}
{selected !== null && <section className="groups-panel"><h2>Groups</h2>{groups.length === 0 ? <p className="muted">No groups yet</p> : <div className="group-list">{groups.map((group) => <div className="group-item" key={group.id}><b>{group.name}</b><small>{group.wires.map(channelLabel).join(",")}</small><span className="group-value">{activeSample === null ? "cursor?" : (groupValues[group.id] ?? "…")}</span><button className="mini" title={`Delete group ${group.name}`} onClick={() => void deleteGroup(group.id)}>✕</button></div>)}</div>}<div className="group-add"><input aria-label="Group name" placeholder="Name" value={newGroupName} onChange={(event) => setNewGroupName(event.target.value)} /><input aria-label="Group wires" placeholder="D0,D1,D2,D3" value={newGroupWires} onChange={(event) => setNewGroupWires(event.target.value)} /><button onClick={() => void createGroup()} disabled={busy || newGroupName.trim() === "" || newGroupWires.trim() === ""}>Add</button></div></section>}
    <section className="history"><h2>Capture history{!following && captures.length > 0 && <button className="follow-latest" onClick={() => { setFollowing(true); setSelectedId(ordered[0]?.id ?? null); }}>Jump to latest</button>}</h2>{captures.length === 0 ? <p className="muted">No captures yet</p> : ordered.map((capture) => <button className={capture.id === selected?.id ? "selected" : ""} key={capture.id} onClick={() => { setFollowing(false); setSelectedId(capture.id); }}><span>Capture {capture.id}</span><small>{expandedLength(capture).toLocaleString()} samples</small></button>)}</section></aside>
    {importOpen && <div className="modal-backdrop" role="presentation"><form className="modal" aria-label="Import LPF project" onSubmit={(event) => { event.preventDefault(); void invoke("project.import_lpf", { path: importPath }).then(() => setImportOpen(false)); }}><h1>Import LPF project</h1><label>LPF path<input autoFocus value={importPath} onChange={(event) => setImportPath(event.target.value)} /></label><div className="modal-actions"><button type="button" disabled={busy} onClick={() => setImportOpen(false)}>Cancel</button><button className="primary" type="submit" disabled={busy || importPath.trim() === ""}>Import</button></div></form></div>}
    <CommandPalette open={paletteOpen} busy={busy} onClose={() => setPaletteOpen(false)} onInvoke={invoke} />
  </div>;
}
const rateLabels = ["500 MHz", "200 MHz", "100 MHz", "50 MHz", "20 MHz", "10 MHz", "5 MHz", "2 MHz", "1 MHz", "500 kHz", "200 kHz", "100 kHz", "50 kHz", "20 kHz", "10 kHz", "5 kHz", "2 kHz", "1 kHz"];
function expandedLength(capture: Capture): number { return capture.runs.reduce((sum, run) => sum + run.count, 0); }
// JS bitwise `&` truncates to 32 bits, so it cannot test channels 32/33 (the
// clocks). Extract the bit by division instead; `data` (a double) holds all 34
// channel bits exactly.
function channelBit(data: number, channel: number): number { return Math.floor(data / 2 ** channel) % 2; }
function channelLabel(channel: number): string { return channel < 32 ? `D${channel}` : channel === 32 ? "CLK1" : "CLK2"; }
// Parse a wire label (D0..D31, CLK1, CLK2) or a bare index into a wire number.
function labelToWire(label: string): number | null {
  const upper = label.trim().toUpperCase();
  if (upper === "CLK1") return 32;
  if (upper === "CLK2") return 33;
  const match = upper.match(/^D(\d+)$/);
  if (match) { const wire = Number(match[1]); return wire >= 0 && wire <= 31 ? wire : null; }
  const numeric = Number(upper);
  return Number.isInteger(numeric) && numeric >= 0 && numeric <= 33 ? numeric : null;
}
function parseWires(text: string): number[] {
  return text.split(",").map((part) => labelToWire(part)).filter((wire): wire is number => wire !== null);
}
function sampleValueAt(capture: Capture, sample: number): number { let acc = 0; for (const run of capture.runs) { acc += run.count; if (sample < acc) return run.data; } return capture.runs.at(-1)?.data ?? 0; }
function channelValue(capture: Capture, channel: number, sample: number | null): string { const data = sample === null ? (capture.runs.at(-1)?.data ?? 0) : sampleValueAt(capture, sample); return channelBit(data, channel) === 0 ? "0" : "1"; }
function formatTime(seconds: number): string { const s = Math.abs(seconds); if (s < 1e-6) return `${(seconds * 1e9).toFixed(1)} ns`; if (s < 1e-3) return `${(seconds * 1e6).toFixed(2)} µs`; if (s < 1) return `${(seconds * 1e3).toFixed(3)} ms`; return `${seconds.toFixed(4)} s`; }
function formatHz(hz: number): string { if (hz >= 1e6) return `${(hz / 1e6).toFixed(3)} MHz`; if (hz >= 1e3) return `${(hz / 1e3).toFixed(3)} kHz`; return `${hz.toFixed(2)} Hz`; }
function formatMeasurement(value: number | null, unit: string): string { if (value === null || !Number.isFinite(value)) return "—"; if (unit === "Hz") return formatHz(value); if (unit === "s") return formatTime(value); if (unit === "count") return String(Math.round(value)); if (unit === "ratio") return `${(value * 100).toFixed(1)}%`; return unit ? `${value} ${unit}` : `${value}`; }
function formatPeriod(seconds: number): string { if (seconds < 1e-9) return `${(seconds * 1e12).toFixed(1)} ps/sample`; if (seconds < 1e-6) return `${(seconds * 1e9).toFixed(1)} ns/sample`; if (seconds < 1e-3) return `${(seconds * 1e6).toFixed(1)} µs/sample`; return `${(seconds * 1e3).toFixed(1)} ms/sample`; }
function hex(value: number | null | undefined): string { return value == null ? "?" : `0x${value.toString(16).padStart(2, "0")}`; }
function message(cause: unknown): string { return cause instanceof Error ? cause.message : String(cause); }
const root = document.getElementById("root"); if (root === null) throw new Error("missing #root"); createRoot(root).render(<StrictMode><App /></StrictMode>);
